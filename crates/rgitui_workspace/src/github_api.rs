use http_client::http::StatusCode;

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
