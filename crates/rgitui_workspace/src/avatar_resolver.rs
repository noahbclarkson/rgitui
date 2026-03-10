use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::{App, AsyncApp};
use http_client::HttpClient;
use rgitui_ui::AvatarCache;

/// Resolve avatar URLs for a batch of (name, email) pairs.
///
/// Spawns background tasks that try multiple sources and updates the
/// global `AvatarCache`. Views re-render automatically via `cx.refresh()`.
pub fn resolve_avatars(authors: Vec<(String, String)>, cx: &mut App) {
    if !cx.has_global::<AvatarCache>() {
        cx.set_global(AvatarCache::new());
    }

    // Collect entries that need fetching (dedupe by email)
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

    if to_fetch.is_empty() {
        return;
    }

    let http = cx.http_client();

    for (name, email) in to_fetch {
        let http = http.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = resolve_single(&name, &email, &http).await;
            cx.update(|cx: &mut App| {
                if cx.has_global::<AvatarCache>() {
                    let cache = cx.global_mut::<AvatarCache>();
                    match result {
                        Some(url) => cache.set_resolved(email, url),
                        None => cache.set_not_found(email),
                    }
                }
            });
            cx.refresh();
        })
        .detach();
    }
}

/// Try to resolve an avatar URL for a single author.
/// Tries multiple sources in order of reliability.
/// Returns Some(url) on success, None if no avatar found.
async fn resolve_single(name: &str, email: &str, http: &Arc<dyn HttpClient>) -> Option<String> {
    // 1. Parse GitHub noreply emails: {id}+{username}@users.noreply.github.com
    //    These give us the username directly.
    if let Some(github_url) = parse_github_noreply(email) {
        if check_url_exists(http, &github_url).await {
            return Some(github_url);
        }
    }

    // 2. Try the git author name as a GitHub username.
    //    Many developers use their GitHub username as their git author name.
    if !name.is_empty() && !name.contains(' ') {
        let name_url = format!("https://github.com/{}.png?size=256", name);
        if check_url_exists(http, &name_url).await {
            return Some(name_url);
        }
    }

    // 3. Try GitHub API search by email (works for public emails)
    if let Some(url) = github_api_search(http, email).await {
        if check_url_exists(http, &url).await {
            return Some(url);
        }
    }

    // 4. Try Gravatar as last resort
    let gravatar = gravatar_url(email, 256);
    if check_url_exists(http, &gravatar).await {
        return Some(gravatar);
    }

    None
}

/// Parse GitHub noreply email format: {id}+{username}@users.noreply.github.com
/// Uses https://github.com/{username}.png which redirects to the actual avatar.
fn parse_github_noreply(email: &str) -> Option<String> {
    let email_lower = email.to_lowercase();
    if !email_lower.ends_with("@users.noreply.github.com") {
        return None;
    }

    let local = email_lower.strip_suffix("@users.noreply.github.com")?;

    // Format can be "{id}+{username}" or just "{username}"
    let username = if let Some((_id, username)) = local.split_once('+') {
        username
    } else {
        local
    };

    if username.is_empty() {
        return None;
    }

    Some(format!("https://github.com/{}.png?size=256", username))
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

    // Parse JSON — prefer login (username) to construct github.com/{user}.png URL
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let items = json.get("items")?.as_array()?;
    let first = items.first()?;

    // Use login for the direct .png URL (most reliable, handles redirects)
    if let Some(login) = first.get("login").and_then(|v| v.as_str()) {
        return Some(format!("https://github.com/{}.png?size=256", login));
    }

    // Fallback to avatar_url from API
    let avatar_url = first.get("avatar_url")?.as_str()?;
    Some(format!("{}?s=256", avatar_url))
}

fn gravatar_url(email: &str, size: u32) -> String {
    let hash = md5::compute(email.trim().to_lowercase().as_bytes());
    format!(
        "https://www.gravatar.com/avatar/{:x}?s={}&d=404",
        hash, size
    )
}
