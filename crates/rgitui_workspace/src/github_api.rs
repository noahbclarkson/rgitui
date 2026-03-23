use http_client::http::StatusCode;

fn github_error_detail(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
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
            "Repository not found — your token may not have access to {}/{}",
            owner, repo
        ),
        (401, _) => "GitHub token is invalid or expired".into(),
        (403, Some(msg)) => format!("Access denied: {}", msg),
        (_, Some(msg)) => format!("GitHub API error {}: {}", status, msg),
        (_, None) => format!("GitHub API error: {}", status),
    }
}

pub(crate) fn format_github_detail_error(status: StatusCode, body: &str) -> String {
    match github_error_detail(body) {
        Some(msg) => format!("GitHub API error {}: {}", status, msg),
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
            "Repository not found — your token may not have access to noahbclarkson/rgitui"
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
    fn detail_error_falls_back_to_plain_status_without_json_message() {
        let error = format_github_detail_error(StatusCode::BAD_GATEWAY, "upstream exploded");

        assert_eq!(error, "GitHub API error: 502 Bad Gateway");
    }
}
