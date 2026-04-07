use anyhow::{Context as _, Result};
use rgitui_settings::{current_git_auth_runtime, GitAuthRuntime, GitProviderRuntime};
use std::path::Path;
use std::process::Command;

/// Run a git network command via system git.
///
/// If the command fails with an SSH auth error and HTTPS credentials are
/// available (provider token, system credential helper like `gh`), the
/// command is automatically retried with SSH-to-HTTPS URL rewriting.
pub(crate) fn run_git_network_command(repo_path: &Path, args: &[&str]) -> Result<Option<String>> {
    let auth = current_git_auth_runtime();
    let remote_url = resolve_remote_url(repo_path, args);

    // --- First attempt: run as-is (with SSH key / HTTPS token if configured) ---
    let result = run_git_command(repo_path, args, &auth, &remote_url, false);

    // --- If SSH auth failed, retry with HTTPS rewriting ---
    if let Err(ref e) = result {
        let err_msg = e.to_string();
        if is_ssh_auth_error(&err_msg) {
            if let Some(ref url) = remote_url {
                if is_ssh_url(url) && can_use_https(repo_path, &auth, url) {
                    log::info!("SSH auth failed, retrying with HTTPS rewriting for {}", url);
                    return run_git_command(repo_path, args, &auth, &remote_url, true);
                }
            }
        }
    }

    result
}

/// Build and execute a single git command.
/// When `force_https` is true, SSH URLs are rewritten to HTTPS.
fn run_git_command(
    repo_path: &Path,
    args: &[&str],
    auth: &GitAuthRuntime,
    remote_url: &Option<String>,
    force_https: bool,
) -> Result<Option<String>> {
    let mut config_args: Vec<String> = Vec::new();
    let mut cmd = super::git_command();
    cmd.current_dir(repo_path).env("GIT_TERMINAL_PROMPT", "0");

    if let Some(ref url) = remote_url {
        if force_https && is_ssh_url(url) {
            // Rewrite SSH to HTTPS.
            if let Some((ssh_prefix, https_prefix)) = insteadof_pair(url) {
                config_args.push(format!("url.{https_prefix}.insteadOf={ssh_prefix}"));
            }
            // Inject HTTPS credentials if we have our own.
            if let Some(https_url) = ssh_to_https(url) {
                inject_https_credentials(&mut cmd, auth, &https_url);
            }
        } else if !force_https {
            // Normal mode: use SSH key or HTTPS credentials as appropriate.
            if is_ssh_url(url) {
                if let Some(ref ssh_key_path) = auth.ssh_key_path {
                    if ssh_key_path.exists() {
                        cmd.env(
                            "GIT_SSH_COMMAND",
                            format!(
                                "ssh -i {} -o IdentitiesOnly=yes",
                                shell_escape(ssh_key_path.to_string_lossy().as_ref())
                            ),
                        );
                    }
                }
            } else {
                // HTTPS remote - inject credentials.
                inject_https_credentials(&mut cmd, auth, url);
            }
        }
    }

    // -c flags MUST come before the subcommand.
    for cfg in &config_args {
        cmd.arg("-c").arg(cfg);
    }
    cmd.args(args);

    let output = cmd
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

/// Check if an error message indicates SSH authentication failure.
fn is_ssh_auth_error(msg: &str) -> bool {
    msg.contains("Permission denied (publickey)")
        || msg.contains("Host key verification failed")
        || msg.contains("Could not read from remote repository")
}

/// Check if we have HTTPS credentials available for this SSH URL.
fn can_use_https(repo_path: &Path, auth: &GitAuthRuntime, ssh_url: &str) -> bool {
    let Some(https_url) = ssh_to_https(ssh_url) else {
        return false;
    };
    find_https_credentials(auth, &https_url).is_some()
        || has_credential_helper_for_host(repo_path, &https_url)
}

fn inject_https_credentials(cmd: &mut Command, auth: &GitAuthRuntime, remote_url: &str) {
    let (username, token) = match find_https_credentials(auth, remote_url) {
        Some(creds) => creds,
        None => return,
    };

    let askpass_path = match ensure_askpass_script() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Could not create HTTPS askpass helper: {e}");
            return;
        }
    };

    cmd.env("GIT_ASKPASS", &askpass_path);
    cmd.env("RGITUI_GIT_USER", username);
    cmd.env("RGITUI_GIT_TOKEN", token);
}

fn find_https_credentials(auth: &GitAuthRuntime, remote_url: &str) -> Option<(String, String)> {
    let host = remote_url
        .strip_prefix("https://")
        .or_else(|| remote_url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");

    let matching_provider: Option<&GitProviderRuntime> = auth
        .providers
        .iter()
        .find(|p| p.use_for_https && p.token.is_some() && host_matches(&p.host, host));

    if let Some(provider) = matching_provider {
        if let Some(ref token) = provider.token {
            let username = if provider.username.is_empty() {
                "x-access-token".to_string()
            } else {
                provider.username.clone()
            };
            return Some((username, token.clone()));
        }
    }

    if let Some(ref token) = auth.default_https_token {
        return Some(("x-access-token".to_string(), token.clone()));
    }

    None
}

fn host_matches(provider_host: &str, remote_host: &str) -> bool {
    let ph = provider_host.trim().to_lowercase();
    let rh = remote_host.trim().to_lowercase();
    ph == rh || rh.ends_with(&format!(".{ph}")) || ph.ends_with(&format!(".{rh}"))
}

fn resolve_remote_url(repo_path: &Path, args: &[&str]) -> Option<String> {
    let remote_name = extract_remote_name(args)?;
    let repo = git2::Repository::open(repo_path).ok()?;
    let remote = repo.find_remote(&remote_name).ok()?;
    remote.url().map(|s| s.to_string())
}

fn extract_remote_name(args: &[&str]) -> Option<String> {
    let mut iter = args.iter();
    let subcmd = iter.next()?;
    match *subcmd {
        "fetch" | "pull" | "push" => {
            for arg in iter {
                if !arg.starts_with('-') {
                    return Some(arg.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

fn ensure_askpass_script() -> Result<std::path::PathBuf> {
    let dir = rgitui_settings::config_dir();
    std::fs::create_dir_all(&dir)?;
    let script_path = dir.join("askpass.sh");

    let needs_write = match std::fs::metadata(&script_path) {
        Ok(m) => m.len() == 0,
        Err(_) => true,
    };

    if needs_write {
        let script = "#!/usr/bin/env bash\n\
            case \"$1\" in\n\
            \x20   Username*|username*) echo \"$RGITUI_GIT_USER\" ;;\n\
            \x20   *) echo \"$RGITUI_GIT_TOKEN\" ;;\n\
            esac\n";
        std::fs::write(&script_path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    Ok(script_path)
}

fn is_ssh_url(url: &str) -> bool {
    url.starts_with("git@") || url.starts_with("ssh://")
}

fn ssh_to_https(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        Some(format!("https://{host}/{path}"))
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        Some(format!("https://{rest}"))
    } else {
        None
    }
}

fn insteadof_pair(ssh_url: &str) -> Option<(String, String)> {
    if let Some(rest) = ssh_url.strip_prefix("git@") {
        let host = rest.split_once(':')?.0;
        Some((format!("git@{host}:"), format!("https://{host}/")))
    } else if let Some(rest) = ssh_url.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        let host = rest.split_once('/')?.0;
        Some((format!("ssh://git@{host}/"), format!("https://{host}/")))
    } else {
        None
    }
}

fn has_credential_helper_for_host(repo_path: &Path, https_url: &str) -> bool {
    let host = https_url
        .strip_prefix("https://")
        .or_else(|| https_url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if host.is_empty() {
        return false;
    }

    let output = super::git_command()
        .args(["config", "--get-regexp", "credential"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(out) => {
            let config = String::from_utf8_lossy(&out.stdout);
            config.contains(&format!("credential.https://{host}"))
                || config.contains("credential.helper")
        }
        Err(_) => false,
    }
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
