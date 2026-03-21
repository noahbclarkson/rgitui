//! App update checker.
//!
//! Checks GitHub releases API on startup to see if a newer version is available.
//! Runs in background, non-blocking. Respects rate limits by only checking once per session.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as AnyhowContext;
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use gpui::{AsyncApp, Context, WeakEntity};

use crate::{ToastKind, Workspace};

/// Current app version. Should match Cargo.toml.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repo for release checks.
const GITHUB_REPO: &str = "noahbclarkson/rgitui";

/// Check for app updates in the background.
///
/// This spawns a background task that:
/// 1. Fetches the latest release from GitHub
/// 2. Compares versions
/// 3. Shows a toast if an update is available
///
/// It respects GitHub's rate limits by only checking once per session.
pub fn check_for_updates(workspace: WeakEntity<Workspace>, cx: &mut Context<Workspace>) {
    let http_client = cx.http_client();

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        // Small delay to let the app fully start up first
        cx.background_executor().timer(Duration::from_secs(5)).await;

        // Fetch latest release from GitHub
        let release_info = match fetch_latest_release(http_client).await {
            Ok(info) => info,
            Err(e) => {
                log::debug!("Update check failed: {}", e);
                return;
            }
        };

        // Compare versions
        if is_newer_version(APP_VERSION, &release_info.version) {
            let msg = if let Some(url) = release_info.html_url {
                format!(
                    "Update available: v{} (you have v{}). {}",
                    release_info.version, APP_VERSION, url
                )
            } else {
                format!(
                    "Update available: v{} (you have v{})",
                    release_info.version, APP_VERSION
                )
            };

            let _ = cx.update(|cx| {
                workspace.update(cx, |ws, cx| {
                    ws.show_toast(msg, ToastKind::Info, cx);
                })
            });
        }
    })
    .detach();
}

/// Release info from GitHub API.
struct ReleaseInfo {
    version: String,
    html_url: Option<String>,
}

/// Fetch the latest release from GitHub using the GPUI HTTP client.
async fn fetch_latest_release(client: Arc<dyn HttpClient>) -> anyhow::Result<ReleaseInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let request = Request::builder()
        .method(Method::GET)
        .uri(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", format!("rgitui/{}", APP_VERSION))
        .body(AsyncBody::empty())
        .context("Failed to build GitHub API request")?;

    let mut response = client
        .send(request)
        .await
        .context("Failed to send request to GitHub API")?;

    if !response.status().is_success() {
        anyhow::bail!("GitHub API returned status {}", response.status());
    }

    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .context("Failed to read response body")?;

    let json: serde_json::Value =
        serde_json::from_slice(&body).context("Failed to parse GitHub response")?;

    let tag_name = json["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No tag_name in release response"))?;

    // Strip 'v' prefix if present (e.g., "v0.2.0" -> "0.2.0")
    let version = tag_name.strip_prefix('v').unwrap_or(tag_name).to_string();

    let html_url = json["html_url"].as_str().map(|s| s.to_string());

    Ok(ReleaseInfo { version, html_url })
}

/// Compare two semver versions. Returns true if `latest` is newer than `current`.
fn is_newer_version(current: &str, latest: &str) -> bool {
    let current_parts: Vec<u64> = current.split('.').filter_map(|s| s.parse().ok()).collect();
    let latest_parts: Vec<u64> = latest.split('.').filter_map(|s| s.parse().ok()).collect();

    // Pad with zeros to ensure equal length
    let max_len = current_parts.len().max(latest_parts.len());
    let mut current_padded = current_parts.clone();
    let mut latest_padded = latest_parts.clone();

    while current_padded.len() < max_len {
        current_padded.push(0);
    }
    while latest_padded.len() < max_len {
        latest_padded.push(0);
    }

    // Compare version parts
    for (c, l) in current_padded.iter().zip(latest_padded.iter()) {
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.1.0", "0.1.1"));
        assert!(is_newer_version("0.1.0", "1.0.0"));
        assert!(!is_newer_version("0.2.0", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(is_newer_version("0.1.0", "0.1.0.1")); // handles extra parts
    }
}
