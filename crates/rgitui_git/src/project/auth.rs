use anyhow::{Context as _, Result};
use git2::{Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository};
use rgitui_settings::current_git_auth_runtime;
use std::path::Path;
use std::process::Command;

pub(crate) const MAX_AUTH_ATTEMPTS: u32 = 6;

/// Create RemoteCallbacks with credential support.
///
/// Auth strategy (in order):
/// 1. SSH agent (for SSH URLs)
/// 2. Custom SSH key from settings (for SSH URLs)
/// 3. Default SSH keys (~/.ssh/id_ed25519, id_rsa, id_ecdsa)
/// 4. Personal access token from settings (for HTTPS URLs)
/// 5. Git credential helper (for HTTPS URLs)
/// 6. Anonymous access (for public HTTPS repos)
pub(crate) fn make_remote_callbacks<'a>() -> RemoteCallbacks<'a> {
    let auth_config = current_git_auth_runtime();
    let mut callbacks = RemoteCallbacks::new();
    let attempts = std::cell::Cell::new(0u32);
    let tried_anonymous = std::cell::Cell::new(false);

    callbacks.credentials(move |url, username_from_url, allowed_types| {
        let attempt = attempts.get();
        attempts.set(attempt + 1);

        // libgit2 retries credentials on failure — bail after reasonable attempts
        if attempt > MAX_AUTH_ATTEMPTS {
            return Err(git2::Error::from_str(
                "authentication failed after multiple attempts",
            ));
        }

        log::debug!(
            "git auth attempt {} for url={} user={:?} types={:?}",
            attempt,
            url,
            username_from_url,
            allowed_types
        );

        let user = username_from_url.unwrap_or("git");
        let is_ssh = is_ssh_remote_url(url);

        // ── SSH authentication ─────────────────────────────────────
        if allowed_types.contains(git2::CredentialType::SSH_KEY) {
            // 1. Try SSH agent first
            if let Ok(cred) = Cred::ssh_key_from_agent(user) {
                log::debug!("Using SSH agent credentials");
                return Ok(cred);
            }

            // 2. Try custom SSH key from settings
            if let Some(ref key_path) = auth_config.ssh_key_path {
                if key_path.exists() {
                    let pub_path = key_path.with_extension("pub");
                    let pub_key = if pub_path.exists() {
                        Some(pub_path.as_path())
                    } else {
                        None
                    };
                    if let Ok(cred) = Cred::ssh_key(user, pub_key, key_path, None) {
                        log::debug!("Using custom SSH key: {}", key_path.display());
                        return Ok(cred);
                    }
                }
            }

            // 3. Fallback: try common SSH key paths
            let home = dirs::home_dir().unwrap_or_default();
            for key_name in &["id_ed25519", "id_rsa", "id_ecdsa"] {
                let key_path = home.join(".ssh").join(key_name);
                if key_path.exists() {
                    let pub_path = home.join(".ssh").join(format!("{}.pub", key_name));
                    let pub_key = if pub_path.exists() {
                        Some(pub_path.as_path())
                    } else {
                        None
                    };
                    if let Ok(cred) = Cred::ssh_key(user, pub_key, &key_path, None) {
                        log::debug!("Using SSH key: {}", key_path.display());
                        return Ok(cred);
                    }
                }
            }
        }

        // ── HTTPS authentication ───────────────────────────────────
        if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            // 4. Try personal access token from settings
            if let Some(provider) = matching_provider(url, &auth_config) {
                if let Some(token) = provider.token.as_ref() {
                    let username = if provider.username.trim().is_empty() {
                        default_https_username_for(&provider.kind, username_from_url)
                    } else {
                        provider.username.as_str()
                    };
                    log::debug!(
                        "Using provider credentials for host={} provider={}",
                        provider.host,
                        provider.display_name
                    );
                    return Cred::userpass_plaintext(username, token);
                }
            }

            if let Some(ref token) = auth_config.default_https_token {
                let username = default_https_username_for("generic", username_from_url);
                log::debug!("Using default HTTPS personal access token");
                return Cred::userpass_plaintext(username, token);
            }

            // 5. Try git credential helper
            if let Ok(cfg) = git2::Config::open_default() {
                if let Ok(cred) = Cred::credential_helper(&cfg, url, username_from_url) {
                    log::debug!("Using git credential helper");
                    return Ok(cred);
                }
            }

            // 6. For HTTPS URLs, try anonymous access (public repos)
            //    Only try this once to avoid infinite retry loops
            if !is_ssh && !tried_anonymous.get() {
                tried_anonymous.set(true);
                log::debug!("Trying anonymous HTTPS access (public repo)");
                return Cred::userpass_plaintext("", "");
            }
        }

        // Username-only auth (first step of SSH negotiation)
        if allowed_types.contains(git2::CredentialType::USERNAME) {
            return Cred::username(user);
        }

        Err(git2::Error::from_str(
            "no suitable authentication method found",
        ))
    });
    callbacks
}

fn matching_provider<'a>(
    url: &str,
    auth_config: &'a rgitui_settings::GitAuthRuntime,
) -> Option<&'a rgitui_settings::GitProviderRuntime> {
    let host = remote_host(url)?;
    auth_config.providers.iter().find(|provider| {
        provider.use_for_https
            && !provider.host.trim().is_empty()
            && hosts_match(&provider.host, &host)
    })
}

pub(crate) fn is_ssh_remote_url(url: &str) -> bool {
    if url.starts_with("ssh://") {
        return true;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    url.contains('@') && url.contains(':')
}

pub(crate) fn remote_uses_ssh(repo: &Repository, remote_name: &str) -> Result<bool> {
    let remote = repo.find_remote(remote_name)?;
    let url = remote.url().context(format!(
        "Remote '{remote_name}' does not have a URL configured"
    ))?;
    Ok(is_ssh_remote_url(url))
}

pub(crate) fn run_git_network_command(repo_path: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("Failed to run system git command: git {}", args.join(" ")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stderr}\n{stdout}")
    };

    if output.status.success() {
        let detail = detail.trim().to_string();
        Ok((!detail.is_empty()).then_some(detail))
    } else {
        anyhow::bail!(
            "{}",
            if detail.is_empty() {
                format!(
                    "git {} failed with exit status {}",
                    args.join(" "),
                    output.status
                )
            } else {
                detail
            }
        );
    }
}

pub(crate) fn hosts_match(configured_host: &str, actual_host: &str) -> bool {
    let configured = configured_host
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let configured = configured.split('/').next().unwrap_or(configured);
    configured.eq_ignore_ascii_case(actual_host)
}

pub(crate) fn remote_host(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("ssh://") {
        return Some(
            rest.split('@')
                .next_back()
                .and_then(|s| s.split('/').next())
                .unwrap_or(rest)
                .trim_end_matches(':')
                .to_string(),
        );
    }

    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        return Some(rest.split('/').next()?.to_string());
    }

    if let Some(rest) = url.split('@').nth(1) {
        return Some(rest.split(':').next()?.to_string());
    }

    None
}

fn default_https_username_for<'a>(kind: &str, username_from_url: Option<&'a str>) -> &'a str {
    if let Some(username) = username_from_url {
        return username;
    }

    match kind {
        "github" => "x-access-token",
        "gitlab" => "oauth2",
        _ => "git",
    }
}

/// Create FetchOptions with auth callbacks.
pub(crate) fn make_fetch_options<'a>() -> FetchOptions<'a> {
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(make_remote_callbacks());
    fetch_opts
}

/// Create PushOptions with auth callbacks.
pub(crate) fn make_push_options<'a>() -> PushOptions<'a> {
    let mut push_opts = PushOptions::new();
    push_opts.remote_callbacks(make_remote_callbacks());
    push_opts
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_ssh_remote_url ──────────────────────────────────────────

    #[test]
    fn ssh_protocol_prefix() {
        assert!(is_ssh_remote_url("ssh://git@github.com/user/repo.git"));
    }

    #[test]
    fn scp_style_ssh_url() {
        assert!(is_ssh_remote_url("git@github.com:user/repo.git"));
    }

    #[test]
    fn ssh_custom_host() {
        assert!(is_ssh_remote_url("git@gitlab.example.com:group/repo.git"));
    }

    #[test]
    fn https_url_is_not_ssh() {
        assert!(!is_ssh_remote_url("https://github.com/user/repo.git"));
    }

    #[test]
    fn http_url_is_not_ssh() {
        assert!(!is_ssh_remote_url("http://github.com/user/repo.git"));
    }

    #[test]
    fn empty_string_is_not_ssh() {
        assert!(!is_ssh_remote_url(""));
    }

    #[test]
    fn plain_path_is_not_ssh() {
        assert!(!is_ssh_remote_url("/home/user/repo"));
    }

    #[test]
    fn at_sign_only_without_colon_is_not_ssh() {
        assert!(!is_ssh_remote_url("user@host/path"));
    }

    #[test]
    fn colon_only_without_at_is_not_ssh() {
        assert!(!is_ssh_remote_url("host:path"));
    }

    // ── remote_host ────────────────────────────────────────────────

    #[test]
    fn remote_host_https() {
        assert_eq!(
            remote_host("https://github.com/user/repo.git"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn remote_host_http() {
        assert_eq!(
            remote_host("http://gitlab.example.com/group/project.git"),
            Some("gitlab.example.com".to_string())
        );
    }

    #[test]
    fn remote_host_scp_style() {
        assert_eq!(
            remote_host("git@github.com:user/repo.git"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn remote_host_ssh_protocol() {
        assert_eq!(
            remote_host("ssh://git@github.com/user/repo.git"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn remote_host_empty_string() {
        assert_eq!(remote_host(""), None);
    }

    #[test]
    fn remote_host_plain_path() {
        assert_eq!(remote_host("/home/user/repo"), None);
    }

    #[test]
    fn remote_host_https_with_port() {
        assert_eq!(
            remote_host("https://git.example.com:8443/repo.git"),
            Some("git.example.com:8443".to_string())
        );
    }

    // ── hosts_match ────────────────────────────────────────────────

    #[test]
    fn hosts_match_exact() {
        assert!(hosts_match("github.com", "github.com"));
    }

    #[test]
    fn hosts_match_case_insensitive() {
        assert!(hosts_match("GitHub.COM", "github.com"));
    }

    #[test]
    fn hosts_match_strips_https_prefix() {
        assert!(hosts_match("https://github.com", "github.com"));
    }

    #[test]
    fn hosts_match_strips_http_prefix() {
        assert!(hosts_match("http://github.com", "github.com"));
    }

    #[test]
    fn hosts_match_strips_trailing_path() {
        assert!(hosts_match("https://github.com/user", "github.com"));
    }

    #[test]
    fn hosts_match_with_whitespace() {
        assert!(hosts_match("  github.com  ", "github.com"));
    }

    #[test]
    fn hosts_do_not_match_different_hosts() {
        assert!(!hosts_match("github.com", "gitlab.com"));
    }

    #[test]
    fn hosts_do_not_match_empty() {
        assert!(!hosts_match("", "github.com"));
    }
}
