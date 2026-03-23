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

pub async fn request_device_code(
    http: Arc<dyn HttpClient>,
) -> Result<DeviceCodeResponse, String> {
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

pub enum PollResult {
    Pending,
    SlowDown,
    Token(String),
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
