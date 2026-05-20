use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use serde::Deserialize;
use std::sync::Arc;

const GITHUB_CLIENT_ID: &str = "Ov23liZmUKi2hDtDEiWq";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_SCOPES: &str = "repo";

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceFlowStatus {
    Idle,
    WaitingForUser {
        user_code: String,
        verification_uri: String,
    },
    Success,
    Error(String),
}

pub async fn request_device_code(http: Arc<dyn HttpClient>) -> Result<DeviceCodeResponse, String> {
    let body = format!("client_id={GITHUB_CLIENT_ID}&scope={GITHUB_SCOPES}");
    let request = Request::builder()
        .method(Method::POST)
        .uri(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(AsyncBody::from(body.into_bytes()))
        .map_err(|e| e.to_string())?;

    let mut response = http.send(request).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| format!("Failed to parse device code response: {e}"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollResult {
    Pending,
    SlowDown,
    Token(String),
}

fn token_poll_response_to_result(poll: TokenPollResponse) -> Result<PollResult, String> {
    if let Some(token) = poll.access_token {
        if poll.token_type.is_some() && !token.is_empty() {
            return Ok(PollResult::Token(token));
        }
    }

    match poll.error.as_deref() {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some("expired_token") => Err("Device code expired. Please try again.".into()),
        Some("access_denied") => Err("Access was denied. Please try again.".into()),
        Some(err) => Err(format!("GitHub error: {err}")),
        None => Err("Unexpected response from GitHub".into()),
    }
}

pub async fn poll_for_token(
    http: Arc<dyn HttpClient>,
    device_code: &str,
) -> Result<PollResult, String> {
    let body = format!(
        "client_id={GITHUB_CLIENT_ID}&device_code={device_code}&grant_type=urn:ietf:params:oauth:grant-type:device_code"
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri(GITHUB_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(AsyncBody::from(body.into_bytes()))
        .map_err(|e| e.to_string())?;

    let mut response = http.send(request).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut buf)
        .await
        .map_err(|e| e.to_string())?;

    let poll: TokenPollResponse =
        serde_json::from_slice(&buf).map_err(|e| format!("Failed to parse token response: {e}"))?;

    token_poll_response_to_result(poll)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_code_response_deserialize() {
        let json = r#"{
            "device_code": "ABCD1234",
            "user_code": "EFGH-5678",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "ABCD1234");
        assert_eq!(resp.user_code, "EFGH-5678");
        assert_eq!(resp.verification_uri, "https://github.com/login/device");
        assert_eq!(resp.expires_in, 900);
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn test_device_code_response_requires_interval() {
        let json = r#"{
            "device_code": "ABCD1234",
            "user_code": "EFGH-5678",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900
        }"#;
        let result: Result<DeviceCodeResponse, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_poll_result_pending() {
        let json = r#"{
            "error": "authorization_pending",
            "access_token": null
        }"#;
        let poll: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(poll.error.as_deref(), Some("authorization_pending"));
        assert!(poll.access_token.is_none());
    }

    #[test]
    fn test_poll_result_slow_down() {
        let json = r#"{
            "error": "slow_down",
            "access_token": null
        }"#;
        let poll: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(poll.error.as_deref(), Some("slow_down"));
    }

    #[test]
    fn test_poll_result_token() {
        let json = r#"{
            "access_token": "gho_sekret123",
            "token_type": "Bearer"
        }"#;
        let poll: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(poll.access_token.as_deref(), Some("gho_sekret123"));
        assert_eq!(poll.token_type.as_deref(), Some("Bearer"));
    }

    #[test]
    fn test_poll_result_empty_token_is_rejected() {
        // Empty string token should be treated as no token (logic in poll_for_token)
        let poll = TokenPollResponse {
            access_token: Some("".to_string()),
            token_type: Some("Bearer".to_string()),
            error: None,
        };
        assert_eq!(
            token_poll_response_to_result(poll),
            Err("Unexpected response from GitHub".to_string())
        );
    }

    #[test]
    fn test_poll_result_missing_token_type_is_rejected() {
        let poll = TokenPollResponse {
            access_token: Some("gho_sekret123".to_string()),
            token_type: None,
            error: None,
        };
        assert_eq!(
            token_poll_response_to_result(poll),
            Err("Unexpected response from GitHub".to_string())
        );
    }

    #[test]
    fn test_poll_result_expired_token_error() {
        let json = r#"{
            "error": "expired_token"
        }"#;
        let poll: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(poll.error.as_deref(), Some("expired_token"));
    }

    #[test]
    fn test_poll_result_access_denied() {
        let json = r#"{
            "error": "access_denied"
        }"#;
        let poll: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(poll.error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn test_device_flow_status_variants() {
        let idle = DeviceFlowStatus::Idle;
        assert!(matches!(idle, DeviceFlowStatus::Idle));

        let waiting = DeviceFlowStatus::WaitingForUser {
            user_code: "EFGH-5678".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
        };
        if let DeviceFlowStatus::WaitingForUser { user_code, .. } = &waiting {
            assert_eq!(user_code, "EFGH-5678");
        }

        let success = DeviceFlowStatus::Success;
        assert!(matches!(success, DeviceFlowStatus::Success));

        let error = DeviceFlowStatus::Error("rate limit".to_string());
        if let DeviceFlowStatus::Error(msg) = &error {
            assert_eq!(msg, "rate limit");
        }
    }

    #[test]
    fn test_device_flow_status_debug() {
        let status = DeviceFlowStatus::WaitingForUser {
            user_code: "EFGH-5678".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
        };
        let repr = format!("{:?}", status);
        assert!(repr.contains("WaitingForUser"));
    }

    #[test]
    fn test_device_flow_status_clone_eq() {
        let a = DeviceFlowStatus::Error("test".to_string());
        let b = a.clone();
        assert_eq!(a, b);
    }
}
