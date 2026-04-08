//! App update checker.
//!
//! Checks the GitHub releases API on startup to see if a newer version is
//! available. Runs in the background, non-blocking, and is gated by:
//!
//! 1. `settings.auto_check_updates` — users can opt out entirely.
//! 2. `settings.last_update_check_at` — a 24 hour cooldown throttles API
//!    traffic across restarts so frequent launches don't spam GitHub.
//!
//! When a newer release is detected the workspace shows a persistent banner
//! with a clickable "Download" button rather than a transient toast, so the
//! user can still act on it after returning to the app.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as AnyhowContext;
use chrono::Utc;
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use gpui::{AsyncApp, BorrowAppContext, Context, WeakEntity};
use semver::Version;

use crate::Workspace;

use super::UpdateNotification;

/// Current app version. Matches Cargo.toml at build time.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The repository URL declared in the workspace Cargo.toml. We parse
/// `owner/repo` out of it at runtime so there is one source of truth.
const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// How long to wait between background update checks across restarts.
const UPDATE_CHECK_INTERVAL: chrono::Duration = chrono::Duration::hours(24);

/// Check for app updates in the background.
///
/// This spawns a background task that:
/// 1. Returns immediately if the user disabled update checks.
/// 2. Returns immediately if the last successful check was < 24h ago.
/// 3. Fetches the latest release from GitHub.
/// 4. Compares versions using full semver ordering (including pre-releases).
/// 5. Shows a persistent banner if an update is available.
pub fn check_for_updates(workspace: WeakEntity<Workspace>, cx: &mut Context<Workspace>) {
    // Read settings on the main thread before we jump to the background.
    let (enabled, last_check_at) = cx
        .try_global::<rgitui_settings::SettingsState>()
        .map(|state| {
            let settings = state.settings();
            (settings.auto_check_updates, settings.last_update_check_at)
        })
        .unwrap_or((true, None));

    if !enabled {
        log::debug!("Update check skipped: disabled in settings");
        return;
    }

    if let Some(last_check) = last_check_at {
        let elapsed = Utc::now().signed_duration_since(last_check);
        if elapsed < UPDATE_CHECK_INTERVAL {
            let hours_left = (UPDATE_CHECK_INTERVAL - elapsed).num_hours();
            log::debug!(
                "Update check skipped: last check was {} hours ago (next in ~{}h)",
                elapsed.num_hours(),
                hours_left
            );
            return;
        }
    }

    let Some(repo) = parse_github_repo(REPOSITORY_URL) else {
        log::debug!(
            "Update check skipped: could not parse GitHub repo from '{}'",
            REPOSITORY_URL
        );
        return;
    };

    let http_client = cx.http_client();

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        // Small delay to let the app fully start up first.
        cx.background_executor().timer(Duration::from_secs(5)).await;

        let release_info = match fetch_latest_release(http_client, &repo).await {
            Ok(info) => info,
            Err(e) => {
                log::debug!("Update check failed: {}", e);
                return;
            }
        };

        // Record the successful check so we throttle subsequent restarts.
        cx.update(|cx| {
            if cx.try_global::<rgitui_settings::SettingsState>().is_some() {
                cx.update_global::<rgitui_settings::SettingsState, _>(|state, _| {
                    state.mark_update_check_completed();
                });
            }
        });

        let current = match Version::parse(APP_VERSION) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "Update check aborted: current version '{}' is not valid semver: {}",
                    APP_VERSION,
                    e
                );
                return;
            }
        };

        let latest = match Version::parse(&release_info.version) {
            Ok(v) => v,
            Err(e) => {
                log::debug!(
                    "Update check: latest tag '{}' is not valid semver: {}",
                    release_info.version,
                    e
                );
                return;
            }
        };

        if latest <= current {
            log::debug!(
                "Update check: running {}, latest is {} — no update needed",
                current,
                latest
            );
            return;
        }

        log::info!(
            "Update available: rgitui {} is out (you have {})",
            latest,
            current
        );

        let notification = UpdateNotification {
            latest_version: latest.to_string(),
            current_version: current.to_string(),
            release_url: release_info
                .html_url
                .unwrap_or_else(|| format!("https://github.com/{}/releases/latest", repo)),
        };

        cx.update(|cx| {
            workspace
                .update(cx, |ws, cx| {
                    ws.set_update_notification(notification, cx);
                })
                .ok();
        });
    })
    .detach();
}

/// Release info extracted from GitHub's /releases/latest response.
struct ReleaseInfo {
    version: String,
    html_url: Option<String>,
}

/// Parse `owner/repo` out of a Cargo `repository` URL. Accepts forms like:
///
/// * `https://github.com/owner/repo`
/// * `https://github.com/owner/repo.git`
/// * `https://github.com/owner/repo/` (trailing slash)
///
/// Returns `None` for anything that is not a github.com URL we recognise.
fn parse_github_repo(url: &str) -> Option<String> {
    let stripped = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let stripped = stripped.trim_end_matches('/');
    let stripped = stripped.strip_suffix(".git").unwrap_or(stripped);
    // Expect exactly owner/repo (two segments).
    let mut parts = stripped.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{}/{}", owner, repo))
}

/// Fetch the latest release from GitHub using the GPUI HTTP client.
async fn fetch_latest_release(
    client: Arc<dyn HttpClient>,
    repo: &str,
) -> anyhow::Result<ReleaseInfo> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_comparison() {
        assert!(Version::parse("0.2.0").unwrap() > Version::parse("0.1.0").unwrap());
        assert!(Version::parse("0.1.1").unwrap() > Version::parse("0.1.0").unwrap());
        assert!(Version::parse("1.0.0").unwrap() > Version::parse("0.9.9").unwrap());
        // Pre-releases are strictly less than their associated release.
        assert!(Version::parse("0.1.0-rc1").unwrap() < Version::parse("0.1.0").unwrap());
        // Pre-releases order within the same base.
        assert!(Version::parse("0.1.0-rc1").unwrap() < Version::parse("0.1.0-rc2").unwrap());
    }

    #[test]
    fn test_parse_github_repo_https() {
        assert_eq!(
            parse_github_repo("https://github.com/noahbclarkson/rgitui"),
            Some("noahbclarkson/rgitui".to_string())
        );
    }

    #[test]
    fn test_parse_github_repo_https_dotgit() {
        assert_eq!(
            parse_github_repo("https://github.com/noahbclarkson/rgitui.git"),
            Some("noahbclarkson/rgitui".to_string())
        );
    }

    #[test]
    fn test_parse_github_repo_https_trailing_slash() {
        assert_eq!(
            parse_github_repo("https://github.com/owner/repo/"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_repo_ssh() {
        assert_eq!(
            parse_github_repo("git@github.com:owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_repo_rejects_subpath() {
        assert_eq!(
            parse_github_repo("https://github.com/owner/repo/tree/main"),
            None
        );
    }

    #[test]
    fn test_parse_github_repo_rejects_non_github() {
        assert_eq!(parse_github_repo("https://gitlab.com/owner/repo"), None);
    }

    #[test]
    fn test_parse_github_repo_rejects_missing_repo() {
        assert_eq!(parse_github_repo("https://github.com/owner"), None);
    }

    #[test]
    fn test_parse_github_repo_rejects_empty() {
        assert_eq!(parse_github_repo(""), None);
    }
}
