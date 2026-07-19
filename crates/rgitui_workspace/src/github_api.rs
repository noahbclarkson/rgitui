use futures::AsyncReadExt;
use http_client::http::StatusCode;
use http_client::HttpClient;
use std::collections::HashSet;
use std::sync::Arc;

const MAX_COLLECTION_PAGES: usize = 10;

/// Error from a GitHub collection fetch. `auth_required` is set for 401/403/404
/// responses — a missing/invalid token on a private repo reads as 404 — so
/// panels can prompt the user to authenticate instead of showing a raw error.
#[derive(Clone, Debug)]
pub(crate) struct GithubCollectionError {
    pub auth_required: bool,
    pub message: String,
    pub rate_limit: GithubRateLimit,
}

impl GithubCollectionError {
    pub(crate) fn transport(message: String) -> Self {
        Self {
            auth_required: false,
            message,
            rate_limit: GithubRateLimit::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GithubRateLimit {
    pub remaining: Option<u64>,
    pub reset_epoch_seconds: Option<u64>,
    pub retry_after_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubCollectionResponse {
    pub bodies: Arc<[String]>,
    pub etag: Option<String>,
    pub rate_limit: GithubRateLimit,
    pub not_modified: bool,
}

#[derive(Debug)]
struct GithubCollectionPage {
    body: String,
    next_url: Option<String>,
    etag: Option<String>,
    rate_limit: GithubRateLimit,
    not_modified: bool,
}

/// GET a GitHub REST collection, attaching the bearer token only when present so
/// public repositories can be read unauthenticated. Returns the response body on
/// success, or a [`GithubCollectionError`] (with `auth_required` flagged for
/// 401/403/404) otherwise.
async fn github_get_collection_page(
    http: &Arc<dyn HttpClient>,
    url: &str,
    token: Option<&str>,
    owner: &str,
    repo: &str,
    etag: Option<&str>,
) -> Result<GithubCollectionPage, GithubCollectionError> {
    let mut builder = http_client::http::Request::builder()
        .uri(url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui");
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }
    if let Some(etag) = etag {
        builder = builder.header("If-None-Match", etag);
    }
    let request = builder
        .body(Default::default())
        .map_err(|e: http_client::http::Error| GithubCollectionError::transport(e.to_string()))?;
    let response = http
        .send(request)
        .await
        .map_err(|e| GithubCollectionError::transport(e.to_string()))?;
    let status = response.status();
    let rate_limit = parse_rate_limit(response.headers());
    let response_etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    let next_url = response
        .headers()
        .get("link")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_next_link);
    if status == StatusCode::NOT_MODIFIED {
        return Ok(GithubCollectionPage {
            body: String::new(),
            next_url: None,
            etag: response_etag,
            rate_limit,
            not_modified: true,
        });
    }
    let mut body = String::new();
    response
        .into_body()
        .read_to_string(&mut body)
        .await
        .map_err(|e| GithubCollectionError::transport(e.to_string()))?;
    if !status.is_success() {
        return Err(GithubCollectionError {
            auth_required: github_auth_required(status, &rate_limit),
            message: format_rate_limited_error(
                format_github_collection_error(status, &body, owner, repo),
                &rate_limit,
            ),
            rate_limit,
        });
    }
    Ok(GithubCollectionPage {
        body,
        next_url,
        etag: response_etag,
        rate_limit,
        not_modified: false,
    })
}

fn github_auth_required(status: StatusCode, rate_limit: &GithubRateLimit) -> bool {
    match status.as_u16() {
        401 | 404 => true,
        403 => rate_limit.remaining != Some(0) && rate_limit.retry_after_seconds.is_none(),
        _ => false,
    }
}

/// Fetch every page in a GitHub REST collection, following the `rel="next"`
/// link while keeping a hard bound and refusing to forward credentials to a
/// non-GitHub API URL. Network I/O remains asynchronous; callers parse the
/// returned page bodies after the final page arrives.
pub(crate) async fn github_get_collection(
    http: &Arc<dyn HttpClient>,
    url: &str,
    token: Option<&str>,
    owner: &str,
    repo: &str,
    etag: Option<&str>,
) -> Result<GithubCollectionResponse, GithubCollectionError> {
    let mut bodies = Vec::new();
    let mut next_url = Some(url.to_string());
    let mut seen = HashSet::new();
    let mut first_etag = None;
    let mut latest_rate_limit = GithubRateLimit::default();

    while let Some(url) = next_url.take() {
        if bodies.len() >= MAX_COLLECTION_PAGES {
            log::warn!(
                "GitHub collection pagination stopped after {} pages for {}/{}",
                MAX_COLLECTION_PAGES,
                owner,
                repo
            );
            break;
        }
        if !is_supported_github_api_url(&url) || !seen.insert(url.clone()) {
            log::warn!("Ignoring unsafe or cyclic GitHub pagination URL: {url}");
            break;
        }

        let page_etag = if bodies.is_empty() { etag } else { None };
        let page = github_get_collection_page(http, &url, token, owner, repo, page_etag).await?;
        latest_rate_limit = page.rate_limit.clone();
        if page.not_modified {
            return Ok(GithubCollectionResponse {
                bodies: Vec::new().into(),
                etag: page.etag.or_else(|| etag.map(String::from)),
                rate_limit: latest_rate_limit,
                not_modified: true,
            });
        }
        if bodies.is_empty() {
            first_etag = page.etag.clone();
        }
        bodies.push(page.body);
        next_url = page.next_url;
    }

    Ok(GithubCollectionResponse {
        bodies: bodies.into(),
        etag: first_etag,
        rate_limit: latest_rate_limit,
        not_modified: false,
    })
}

fn parse_rate_limit(headers: &http_client::http::HeaderMap) -> GithubRateLimit {
    let parse = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    };
    GithubRateLimit {
        remaining: parse("x-ratelimit-remaining"),
        reset_epoch_seconds: parse("x-ratelimit-reset"),
        retry_after_seconds: parse("retry-after"),
    }
}

fn format_rate_limited_error(message: String, rate_limit: &GithubRateLimit) -> String {
    if rate_limit.remaining == Some(0) {
        if let Some(reset) = rate_limit.reset_epoch_seconds {
            return format!("{message}. GitHub rate limit resets at Unix timestamp {reset}");
        }
        return format!("{message}. GitHub API rate limit exhausted");
    }
    if let Some(retry_after) = rate_limit.retry_after_seconds {
        return format!("{message}. Retry after {retry_after} seconds");
    }
    message
}

fn is_supported_github_api_url(url: &str) -> bool {
    url.starts_with("https://api.github.com/")
}

fn parse_next_link(value: &str) -> Option<String> {
    value.split(',').find_map(|part| {
        let mut segments = part.trim().split(';');
        let url = segments
            .next()?
            .trim()
            .strip_prefix('<')?
            .strip_suffix('>')?;
        let is_next = segments.any(|segment| segment.trim() == r#"rel="next""#);
        is_next.then(|| url.to_string())
    })
}

fn github_error_detail(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
}

/// Recognise GitHub's verbose 403 messages for org-level access restrictions and
/// rewrite them into a short, actionable instruction. Returns `None` if the message
/// does not match a known pattern.
fn rewrite_org_restriction(msg: &str, owner: Option<&str>) -> Option<String> {
    let org_label = owner
        .map(|o| format!("'{}'", o))
        .unwrap_or_else(|| "this".into());

    if msg.contains("OAuth App access restrictions") {
        return Some(format!(
            "The {} org restricts third-party app access. Ask an admin to approve this PAT in org Settings -> Third-party Access -> Personal access tokens, or use a fine-grained PAT approved by the org.",
            org_label
        ));
    }

    if msg.contains("SAML enforcement") || msg.contains("SAML single sign-on") {
        return Some(format!(
            "Your token needs SAML SSO authorization for the {} org. Open your PAT in GitHub settings and click 'Configure SSO' -> 'Authorize' for this org.",
            org_label
        ));
    }

    None
}

pub(crate) fn format_github_collection_error(
    status: StatusCode,
    body: &str,
    owner: &str,
    repo: &str,
) -> String {
    let detail = github_error_detail(body);
    match (status.as_u16(), detail) {
        (404, _) => format!(
            "Repository not found -- your token may not have access to {}/{}",
            owner, repo
        ),
        (401, _) => "GitHub token is invalid or expired".into(),
        (403, Some(msg)) => rewrite_org_restriction(&msg, Some(owner))
            .unwrap_or_else(|| format!("Access denied: {}", msg)),
        (_, Some(msg)) => format!("GitHub API error {}: {}", status, msg),
        (_, None) => format!("GitHub API error: {}", status),
    }
}

pub(crate) fn format_github_detail_error(status: StatusCode, body: &str) -> String {
    match github_error_detail(body) {
        Some(msg) => {
            if status.as_u16() == 403 {
                if let Some(rewritten) = rewrite_org_restriction(&msg, None) {
                    return rewritten;
                }
            }
            format!("GitHub API error {}: {}", status, msg)
        }
        None => format!("GitHub API error: {}", status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_next_link_from_multi_relation_header() {
        let header = concat!(
            "<https://api.github.com/repositories/1/issues?page=1>; rel=\"prev\", ",
            "<https://api.github.com/repositories/1/issues?page=3>; rel=\"next\", ",
            "<https://api.github.com/repositories/1/issues?page=8>; rel=\"last\""
        );
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("https://api.github.com/repositories/1/issues?page=3")
        );
    }

    #[test]
    fn pagination_rejects_non_github_api_urls() {
        assert!(is_supported_github_api_url(
            "https://api.github.com/repos/owner/repo/issues?page=2"
        ));
        assert!(!is_supported_github_api_url(
            "https://example.com/steal-token?page=2"
        ));
        assert!(!is_supported_github_api_url(
            "http://api.github.com/repos/x/y"
        ));
    }

    #[test]
    fn parses_rate_limit_and_retry_headers() {
        let mut headers = http_client::http::HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        headers.insert("x-ratelimit-reset", "1750000000".parse().unwrap());
        headers.insert("retry-after", "60".parse().unwrap());

        assert_eq!(
            parse_rate_limit(&headers),
            GithubRateLimit {
                remaining: Some(0),
                reset_epoch_seconds: Some(1_750_000_000),
                retry_after_seconds: Some(60),
            }
        );
    }

    #[test]
    fn exhausted_rate_limit_error_surfaces_reset_time() {
        let message = format_rate_limited_error(
            "Access denied".into(),
            &GithubRateLimit {
                remaining: Some(0),
                reset_epoch_seconds: Some(1_750_000_000),
                retry_after_seconds: None,
            },
        );
        assert!(message.contains("rate limit resets"));
        assert!(message.contains("1750000000"));
        assert!(!github_auth_required(
            StatusCode::FORBIDDEN,
            &GithubRateLimit {
                remaining: Some(0),
                reset_epoch_seconds: Some(1_750_000_000),
                retry_after_seconds: None,
            }
        ));
    }

    #[test]
    fn collection_error_prefers_repo_not_found_message_for_404s() {
        let error = format_github_collection_error(
            StatusCode::NOT_FOUND,
            r#"{"message":"Not Found"}"#,
            "noahbclarkson",
            "rgitui",
        );

        assert_eq!(
            error,
            "Repository not found -- your token may not have access to noahbclarkson/rgitui"
        );
    }

    #[test]
    fn collection_error_maps_unauthorized_to_token_message() {
        let error = format_github_collection_error(
            StatusCode::UNAUTHORIZED,
            r#"{"message":"Bad credentials"}"#,
            "noahbclarkson",
            "rgitui",
        );

        assert_eq!(error, "GitHub token is invalid or expired");
    }

    #[test]
    fn collection_error_includes_forbidden_detail() {
        let error = format_github_collection_error(
            StatusCode::FORBIDDEN,
            r#"{"message":"Resource not accessible by integration"}"#,
            "noahbclarkson",
            "rgitui",
        );

        assert_eq!(
            error,
            "Access denied: Resource not accessible by integration"
        );
    }

    #[test]
    fn collection_error_rewrites_oauth_app_restrictions() {
        let body = r#"{"message":"Although you appear to have the correct authorization credentials, the `acme` organization has enabled OAuth App access restrictions, meaning that data access to third-parties is limited. For more information on these restrictions, including how to enable this app, visit https://docs.github.com/articles/restricting-access-to-your-organizations-data/"}"#;
        let error = format_github_collection_error(StatusCode::FORBIDDEN, body, "acme", "widgets");
        assert!(error.contains("'acme' org restricts third-party app access"));
        assert!(error.contains("Personal access tokens"));
        assert!(!error.contains("OAuth App access restrictions"));
    }

    #[test]
    fn collection_error_rewrites_saml_sso_enforcement() {
        let body = r#"{"message":"Resource protected by organization SAML enforcement. You must grant your personal token access to this organization."}"#;
        let error = format_github_collection_error(StatusCode::FORBIDDEN, body, "acme", "widgets");
        assert!(error.contains("SAML SSO authorization"));
        assert!(error.contains("'acme' org"));
        assert!(error.contains("Configure SSO"));
    }

    #[test]
    fn detail_error_rewrites_oauth_app_restrictions_without_owner() {
        let body = r#"{"message":"the `acme` organization has enabled OAuth App access restrictions, meaning that data access to third-parties is limited."}"#;
        let error = format_github_detail_error(StatusCode::FORBIDDEN, body);
        assert!(error.contains("third-party app access"));
        assert!(error.contains("Personal access tokens"));
    }

    #[test]
    fn detail_error_rewrites_saml_sso_without_owner() {
        let body = r#"{"message":"Resource protected by organization SAML enforcement."}"#;
        let error = format_github_detail_error(StatusCode::FORBIDDEN, body);
        assert!(error.contains("SAML SSO authorization"));
        assert!(error.contains("Configure SSO"));
    }

    #[test]
    fn detail_error_falls_back_to_plain_status_without_json_message() {
        let error = format_github_detail_error(StatusCode::BAD_GATEWAY, "upstream exploded");

        assert_eq!(error, "GitHub API error: 502 Bad Gateway");
    }

    #[test]
    fn error_detail_returns_none_when_json_has_no_message_field() {
        let result = github_error_detail(r#"{"error":"rate limit"}"#);
        assert_eq!(result, None);
    }

    #[test]
    fn error_detail_returns_none_for_non_json_body() {
        let result = github_error_detail("Internal Server Error");
        assert_eq!(result, None);
    }

    #[test]
    fn error_detail_returns_none_for_empty_body() {
        let result = github_error_detail("");
        assert_eq!(result, None);
    }

    #[test]
    fn error_detail_extracts_message_from_valid_json() {
        let result =
            github_error_detail(r#"{"message":"semi-authenticated request limit reached"}"#);
        assert_eq!(
            result,
            Some("semi-authenticated request limit reached".into())
        );
    }

    #[test]
    fn collection_error_forbidden_without_detail_message() {
        // 403 with no body / no message key should fall through to the catch-all
        let error = format_github_collection_error(
            StatusCode::FORBIDDEN,
            r#"{"error":"forbidden"}"#,
            "owner",
            "repo",
        );
        assert_eq!(error, "GitHub API error: 403 Forbidden");
    }

    #[test]
    fn collection_error_server_error_with_message() {
        let error = format_github_collection_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"message":"Something went wrong"}"#,
            "owner",
            "repo",
        );
        assert_eq!(
            error,
            "GitHub API error 500 Internal Server Error: Something went wrong"
        );
    }

    #[test]
    fn collection_error_server_error_without_message() {
        let error = format_github_collection_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "retry later",
            "owner",
            "repo",
        );
        assert_eq!(error, "GitHub API error: 503 Service Unavailable");
    }

    #[test]
    fn detail_error_server_error_with_json_message() {
        let error = format_github_detail_error(
            StatusCode::GATEWAY_TIMEOUT,
            r#"{"message":"Request timed out"}"#,
        );
        assert_eq!(
            error,
            "GitHub API error 504 Gateway Timeout: Request timed out"
        );
    }

    #[test]
    fn detail_error_non_json_body_falls_back_to_status() {
        let error = format_github_detail_error(StatusCode::NOT_IMPLEMENTED, "NotImplemented");
        assert_eq!(error, "GitHub API error: 501 Not Implemented");
    }
}
