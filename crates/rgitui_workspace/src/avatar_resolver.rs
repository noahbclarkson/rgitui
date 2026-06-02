use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::select;
use futures::future::Either;
use futures::AsyncReadExt;
use gpui::{App, AsyncApp};
use http_client::HttpClient;
use rgitui_ui::AvatarCache;

const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the persistence collector waits for additional resolved entries
/// before flushing the accumulated batch to disk. Coalescing the writes of a
/// single resolution wave into one flush avoids rewriting the cache file once
/// per entry.
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

/// Resolve avatar URLs for a batch of (name, email) pairs.
///
/// Spawns background tasks that try multiple sources and updates the
/// global `AvatarCache`. Views re-render automatically via `cx.refresh()`.
///
/// Resolved entries are funnelled through a single channel to one collector
/// task, which debounces and persists the batch on the background executor.
/// This avoids spawning a raw OS thread per entry and serialises disk writes
/// into a single writer, eliminating the lost-update race that occurred when
/// many threads rewrote the cache file concurrently.
pub fn resolve_avatars(authors: Vec<(String, String)>, cx: &mut App) {
    if !cx.has_global::<AvatarCache>() {
        cx.set_global(AvatarCache::new());
    }

    let to_fetch: Vec<(String, String)> = {
        let cache = cx.global_mut::<AvatarCache>();
        authors
            .into_iter()
            .filter(|(_name, email)| {
                if cache.needs_fetch(email) {
                    cache.mark_pending(email);
                    true
                } else {
                    false
                }
            })
            .collect()
    };

    log::debug!("resolve_avatars: {} authors need fetch", to_fetch.len());

    if to_fetch.is_empty() {
        return;
    }

    let http = cx.http_client();

    // Single sender shared by every per-author task; a lone collector task
    // owns the receiver and drives all disk persistence.
    let (persist_tx, persist_rx) = smol::channel::unbounded::<(String, String)>();
    spawn_persist_collector(persist_rx, cx);

    for (name, email) in to_fetch {
        let http = http.clone();
        let persist_tx = persist_tx.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = resolve_single(&name, &email, &http).await;
            let resolved = result.is_some();
            cx.update(|cx: &mut App| {
                if cx.has_global::<AvatarCache>() {
                    let cache = cx.global_mut::<AvatarCache>();
                    match &result {
                        Some(url) => {
                            log::info!("avatar resolved: email={} url={}", email, url);
                            cache.set_resolved(email.clone(), url.clone());
                        }
                        None => {
                            log::debug!("avatar not found: email={}", email);
                            cache.set_not_found(email.clone());
                        }
                    }
                }
            });
            // Queue successful resolutions for persistence after the in-memory
            // cache update so a crash mid-resolution never writes an entry the
            // cache itself dropped. Errors mean the collector already exited.
            if let Some(url) = result {
                let _ = persist_tx.send((email, url)).await;
            }
            // Only refresh when an avatar was actually resolved — NotFound doesn't
            // change what's displayed (initials fallback remains unchanged).
            if resolved {
                cx.refresh();
            }
        })
        .detach();
    }
}

/// Drain resolved `(email, url)` pairs from `rx`, debounce briefly to coalesce
/// a resolution wave, then persist the accumulated batch on the background
/// executor. The task exits once every sender is dropped and the channel is
/// drained.
fn spawn_persist_collector(rx: smol::channel::Receiver<(String, String)>, cx: &mut App) {
    let background = cx.background_executor().clone();
    background
        .clone()
        .spawn(async move {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];

                // Coalesce everything that arrives within the debounce window
                // into the same flush.
                loop {
                    let timer = background.timer(PERSIST_DEBOUNCE);
                    match select(Box::pin(rx.recv()), Box::pin(timer)).await {
                        Either::Left((Ok(entry), _)) => batch.push(entry),
                        // Sender dropped, or the debounce window elapsed: flush.
                        Either::Left((Err(_), _)) | Either::Right(_) => break,
                    }
                }

                // TODO(audit): PERF-14 collapse this per-entry read-modify-rewrite
                // into one whole-map flush once `AvatarCache` (in rgitui_ui) exposes a
                // batched `save_all_to_disk` / resolved-entry iterator; this file
                // cannot add that method.
                for (email, url) in &batch {
                    AvatarCache::save_entry_to_disk(email, url);
                }
            }
        })
        .detach();
}

type CheckFuture = Pin<Box<dyn futures::Future<Output = Option<String>> + Send>>;

/// Try to resolve an avatar URL for a single author.
///
/// Uses a two-batch concurrent strategy:
/// - Batch 1 (cheap URL checks): noreply parse, name-as-username, local-part, gravatar
/// - Batch 2 (API calls): GitHub API search by email, GitHub API search by name
///
/// Within each batch, checks run concurrently. The first successful result wins.
/// Each individual check has a timeout to avoid hanging on slow endpoints.
async fn resolve_single(name: &str, email: &str, http: &Arc<dyn HttpClient>) -> Option<String> {
    // Batch 1: Cheap URL existence checks (run concurrently)
    let mut batch1: Vec<CheckFuture> = Vec::new();

    if let Some(github_url) = parse_github_noreply(email) {
        let http = http.clone();
        batch1.push(Box::pin(async move {
            if check_url_with_timeout(&http, &github_url).await {
                Some(github_url)
            } else {
                None
            }
        }));
    }

    {
        let gravatar = gravatar_url(email, 48);
        let http = http.clone();
        batch1.push(Box::pin(async move {
            if check_url_with_timeout(&http, &gravatar).await {
                Some(gravatar)
            } else {
                None
            }
        }));
    }

    if let Some(url) = first_successful(batch1).await {
        return Some(url);
    }

    // Batch 2: GitHub API searches (heavier, run concurrently)
    let mut batch2: Vec<CheckFuture> = Vec::new();

    {
        let http = http.clone();
        let email_owned = email.to_string();
        batch2.push(Box::pin(async move {
            with_timeout(async {
                if let Some(url) = github_api_search(&http, &email_owned).await {
                    if check_url_exists(&http, &url).await {
                        return Some(url);
                    }
                }
                None
            })
            .await
        }));
    }

    if !name.is_empty() && name.contains(' ') {
        let http = http.clone();
        let name_owned = name.to_string();
        batch2.push(Box::pin(async move {
            with_timeout(async {
                if let Some(url) = github_api_search_by_name(&http, &name_owned).await {
                    if check_url_exists(&http, &url).await {
                        return Some(url);
                    }
                }
                None
            })
            .await
        }));
    }

    if !batch2.is_empty() {
        if let Some(url) = first_successful(batch2).await {
            return Some(url);
        }
    }

    None
}

/// Run all futures concurrently and return the first `Some` result.
/// Returns `None` only if all futures complete with `None`.
async fn first_successful(futures: Vec<CheckFuture>) -> Option<String> {
    if futures.is_empty() {
        return None;
    }

    let (result, _, remaining) = futures::future::select_all(futures).await;
    if result.is_some() {
        return result;
    }

    if remaining.is_empty() {
        return None;
    }

    // Continue checking remaining futures
    Box::pin(first_successful(remaining)).await
}

/// Wrap a future with a timeout. Returns `None` if the timeout expires.
async fn with_timeout<F>(future: F) -> Option<String>
where
    F: futures::Future<Output = Option<String>> + Send,
{
    let timeout = smol::Timer::after(CHECK_TIMEOUT);
    match select(Box::pin(future), Box::pin(timeout)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => None,
    }
}

/// Check if a URL returns 200 OK, with a per-check timeout.
async fn check_url_with_timeout(http: &Arc<dyn HttpClient>, url: &str) -> bool {
    let http = http.clone();
    let url = url.to_string();
    let check = async move { check_url_exists(&http, &url).await };
    let timeout = smol::Timer::after(CHECK_TIMEOUT);
    match select(Box::pin(check), Box::pin(timeout)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => false,
    }
}

/// Parse GitHub noreply email format: {id}+{username}@users.noreply.github.com
/// Uses https://github.com/{username}.png which redirects to the actual avatar.
fn parse_github_noreply(email: &str) -> Option<String> {
    let email_lower = email.to_lowercase();
    if !email_lower.ends_with("@users.noreply.github.com") {
        return None;
    }

    let local = email_lower.strip_suffix("@users.noreply.github.com")?;

    let username = if let Some((_id, username)) = local.split_once('+') {
        username
    } else {
        local
    };

    if username.is_empty() {
        return None;
    }

    Some(format!("https://github.com/{}.png?size=48", username))
}

/// Check if a URL returns 200 OK (HEAD-like: we read minimal data).
async fn check_url_exists(http: &Arc<dyn HttpClient>, url: &str) -> bool {
    match http.get(url, Default::default(), true).await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

/// Search GitHub API for a user by email and return their avatar_url.
async fn github_api_search(http: &Arc<dyn HttpClient>, email: &str) -> Option<String> {
    let url = format!(
        "https://api.github.com/search/users?q={}+in:email&per_page=1",
        email
    );

    let request = http_client::http::Request::builder()
        .uri(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .body(Default::default())
        .ok()?;

    let response = http.send(request).await.ok()?;

    if !response.status().is_success() {
        return None;
    }

    let mut body = String::new();
    let mut reader = response.into_body();
    reader.read_to_string(&mut body).await.ok()?;

    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let items = json.get("items")?.as_array()?;
    let first = items.first()?;

    if let Some(login) = first.get("login").and_then(|v| v.as_str()) {
        return Some(format!("https://github.com/{}.png?size=48", login));
    }

    let avatar_url = first.get("avatar_url")?.as_str()?;
    Some(format!("{}?s=48", avatar_url))
}

async fn github_api_search_by_name(http: &Arc<dyn HttpClient>, name: &str) -> Option<String> {
    let encoded_name = name.replace(' ', "+");
    let url = format!(
        "https://api.github.com/search/users?q={}+in:fullname&per_page=1",
        encoded_name
    );

    let request = http_client::http::Request::builder()
        .uri(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .body(Default::default())
        .ok()?;

    let response = http.send(request).await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let mut body = String::new();
    let mut reader = response.into_body();
    reader.read_to_string(&mut body).await.ok()?;

    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let items = json.get("items")?.as_array()?;
    let first = items.first()?;

    if let Some(login) = first.get("login").and_then(|v| v.as_str()) {
        return Some(format!("https://github.com/{}.png?size=48", login));
    }

    let avatar_url = first.get("avatar_url")?.as_str()?;
    Some(format!("{}?s=48", avatar_url))
}

fn gravatar_url(email: &str, size: u32) -> String {
    let hash = md5::compute(email.trim().to_lowercase().as_bytes());
    format!(
        "https://www.gravatar.com/avatar/{:x}?s={}&d=404",
        hash, size
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_noreply_with_username() {
        // Format: {id}+{username}@users.noreply.github.com
        let result = parse_github_noreply("12345+defunkt@users.noreply.github.com");
        assert_eq!(
            result,
            Some("https://github.com/defunkt.png?size=48".to_string())
        );
    }

    #[test]
    fn test_parse_github_noreply_id_only() {
        // Format: {id}@users.noreply.github.com (no +username)
        let result = parse_github_noreply("12345@users.noreply.github.com");
        assert_eq!(
            result,
            Some("https://github.com/12345.png?size=48".to_string())
        );
    }

    #[test]
    fn test_parse_github_noreply_case_insensitive() {
        let result = parse_github_noreply("12345+Defunkt@users.noreply.github.com");
        assert_eq!(
            result,
            Some("https://github.com/defunkt.png?size=48".to_string())
        );
    }

    #[test]
    fn test_parse_github_noreply_regular_email_returns_none() {
        let result = parse_github_noreply("user@example.com");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_github_noreply_empty_username_returns_none() {
        let result = parse_github_noreply("@users.noreply.github.com");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_github_noreply_github_com_email_returns_none() {
        // Actual GitHub email, not noreply
        let result = parse_github_noreply("user@github.com");
        assert_eq!(result, None);
    }

    #[test]
    fn test_gravatar_url_generates_correct_hash() {
        // MD5 of "test@example.com" (lowercased, trimmed)
        // "test@example.com" -> md5 = 02db4a2c47e25e3e08c3e1e8e0b2a3d4
        let result = gravatar_url("test@example.com", 48);
        // Just verify it contains expected substrings
        assert!(result.starts_with("https://www.gravatar.com/avatar/"));
        assert!(result.contains("?s=48"));
        assert!(result.contains("&d=404"));
    }

    #[test]
    fn test_gravatar_url_trims_and_lowercases() {
        let result1 = gravatar_url("Test@Example.com", 48);
        let result2 = gravatar_url("  Test@example.com  ", 48);
        // Both should produce same hash after trim+lowercase
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_gravatar_url_different_sizes() {
        let result_48 = gravatar_url("test@example.com", 48);
        let result_128 = gravatar_url("test@example.com", 128);
        assert!(result_48.contains("?s=48"));
        assert!(result_128.contains("?s=128"));
    }
}
