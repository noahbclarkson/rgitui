use anyhow::{Context as _, Result};
use chrono::{TimeZone, Utc};
use git2::{
    Cred, DiffOptions, FetchOptions, PushOptions, RemoteCallbacks, Repository, StatusOptions,
};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rgitui_settings::current_git_auth_runtime;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::types::*;

/// Create RemoteCallbacks with credential support.
///
/// Auth strategy (in order):
/// 1. SSH agent (for SSH URLs)
/// 2. Custom SSH key from settings (for SSH URLs)
/// 3. Default SSH keys (~/.ssh/id_ed25519, id_rsa, id_ecdsa)
/// 4. Personal access token from settings (for HTTPS URLs)
/// 5. Git credential helper (for HTTPS URLs)
/// 6. Anonymous access (for public HTTPS repos)
fn make_remote_callbacks<'a>() -> RemoteCallbacks<'a> {
    let auth_config = current_git_auth_runtime();
    let mut callbacks = RemoteCallbacks::new();
    let attempts = std::cell::Cell::new(0u32);
    let tried_anonymous = std::cell::Cell::new(false);

    callbacks.credentials(move |url, username_from_url, allowed_types| {
        let attempt = attempts.get();
        attempts.set(attempt + 1);

        // libgit2 retries credentials on failure — bail after reasonable attempts
        if attempt > 6 {
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
            let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
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

fn is_ssh_remote_url(url: &str) -> bool {
    if url.starts_with("ssh://") {
        return true;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    url.contains('@') && url.contains(':')
}

fn remote_uses_ssh(repo: &Repository, remote_name: &str) -> Result<bool> {
    let remote = repo.find_remote(remote_name)?;
    let url = remote.url().context(format!(
        "Remote '{remote_name}' does not have a URL configured"
    ))?;
    Ok(is_ssh_remote_url(url))
}

fn run_git_network_command(repo_path: &Path, args: &[&str]) -> Result<Option<String>> {
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

fn hosts_match(configured_host: &str, actual_host: &str) -> bool {
    let configured = configured_host
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let configured = configured.split('/').next().unwrap_or(configured);
    configured.eq_ignore_ascii_case(actual_host)
}

fn remote_host(url: &str) -> Option<String> {
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
fn make_fetch_options<'a>() -> FetchOptions<'a> {
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(make_remote_callbacks());
    fetch_opts
}

/// Create PushOptions with auth callbacks.
fn make_push_options<'a>() -> PushOptions<'a> {
    let mut push_opts = PushOptions::new();
    push_opts.remote_callbacks(make_remote_callbacks());
    push_opts
}

fn parse_remote_tracking_ref(name: &str) -> Option<(String, String)> {
    let trimmed = name.strip_prefix("refs/remotes/").unwrap_or(name);
    let mut parts = trimmed.splitn(2, '/');
    let remote = parts.next()?.trim();
    let branch = parts.next()?.trim();
    if remote.is_empty() || branch.is_empty() {
        return None;
    }
    Some((remote.to_string(), branch.to_string()))
}

fn head_branch_name(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    if !head.is_branch() {
        anyhow::bail!("HEAD is detached. Switch to a branch before running this operation.");
    }
    head.shorthand()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Failed to determine the current branch name"))
}

fn repo_has_worktree_changes(repo: &Repository) -> Result<bool> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_unmodified(false);
    Ok(!repo.statuses(Some(&mut opts))?.is_empty())
}

fn ensure_clean_worktree(repo: &Repository, operation: &str) -> Result<()> {
    if repo_has_worktree_changes(repo)? {
        anyhow::bail!(
            "{} requires a clean working tree. Commit, stash, or discard your changes first.",
            operation
        );
    }
    Ok(())
}

fn default_remote_name(repo: &Repository) -> Result<String> {
    if let Ok(branch_name) = head_branch_name(repo) {
        if let Ok(branch) = repo.find_branch(&branch_name, git2::BranchType::Local) {
            if let Ok(upstream) = branch.upstream() {
                if let Some(upstream_name) = upstream.name()?.and_then(parse_remote_tracking_ref) {
                    return Ok(upstream_name.0);
                }
            }
        }
    }

    let remote_names = repo.remotes()?;
    if remote_names.is_empty() {
        anyhow::bail!("No remotes configured. Add one with: git remote add origin <url>")
    }

    if remote_names.iter().flatten().any(|name| name == "origin") {
        return Ok("origin".to_string());
    }

    remote_names
        .iter()
        .flatten()
        .next()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("No usable git remotes are configured."))
}

fn pull_target(repo: &Repository, preferred_remote: Option<&str>) -> Result<(String, String)> {
    let branch_name = head_branch_name(repo)?;
    if let Ok(branch) = repo.find_branch(&branch_name, git2::BranchType::Local) {
        if let Ok(upstream) = branch.upstream() {
            if let Some(upstream_name) = upstream.name()?.and_then(parse_remote_tracking_ref) {
                if preferred_remote
                    .map(|remote| remote == upstream_name.0)
                    .unwrap_or(true)
                {
                    return Ok(upstream_name);
                }
            }
        }
    }

    let remote_name = preferred_remote
        .map(str::to_string)
        .unwrap_or(default_remote_name(repo)?);
    Ok((remote_name, branch_name))
}

fn push_target(
    repo: &Repository,
    preferred_remote: Option<&str>,
) -> Result<(String, String, bool)> {
    let branch_name = head_branch_name(repo)?;
    if let Ok(branch) = repo.find_branch(&branch_name, git2::BranchType::Local) {
        if let Ok(upstream) = branch.upstream() {
            if let Some((remote_name, remote_branch)) =
                upstream.name()?.and_then(parse_remote_tracking_ref)
            {
                if preferred_remote
                    .map(|remote| remote == remote_name)
                    .unwrap_or(true)
                {
                    return Ok((remote_name, remote_branch, false));
                }
            }
        }
    }

    let remote_name = preferred_remote
        .map(str::to_string)
        .unwrap_or(default_remote_name(repo)?);
    Ok((remote_name, branch_name, true))
}

/// All the data gathered during a refresh, designed to be Send so it can
/// be computed on a background thread and then applied on the main thread.
pub struct RefreshData {
    pub head_branch: Option<String>,
    pub head_detached: bool,
    pub repo_state: RepoState,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub stashes: Vec<StashEntry>,
    pub status: WorkingTreeStatus,
    pub recent_commits: Vec<CommitInfo>,
    /// Whether there are more commits beyond the loaded limit.
    pub has_more_commits: bool,
}

/// Compute line-level diff stats (additions/deletions) for a single file.
/// For staged files, diffs HEAD vs index. For unstaged files, diffs index vs workdir.
fn batch_diff_stats(repo: &Repository, staged: bool) -> std::collections::HashMap<PathBuf, (usize, usize)> {
    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    let diff_result = if staged {
        let head_tree = repo.head().ok().and_then(|r| r.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))
    } else {
        repo.diff_index_to_workdir(None, Some(&mut opts))
    };
    let mut stats_map = std::collections::HashMap::new();
    if let Ok(diff) = diff_result {
        let num_deltas = diff.deltas().len();
        for i in 0..num_deltas {
            if let Ok(Some(patch)) = git2::Patch::from_diff(&diff, i) {
                let (_, adds, dels) = patch.line_stats().unwrap_or((0, 0, 0));
                if let Some(path) = patch.delta().new_file().path() {
                    stats_map.insert(path.to_path_buf(), (adds, dels));
                }
            }
        }
    }
    stats_map
}

fn generate_hunk_patch_for_repo(
    repo: &Repository,
    file_path: &Path,
    hunk_index: usize,
    staged: bool,
) -> Result<String> {
    let mut diff_opts = DiffOptions::new();
    diff_opts.pathspec(file_path);

    let diff = if staged {
        let head_tree = repo.head()?.peel_to_tree().ok();
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?
    } else {
        repo.diff_index_to_workdir(None, Some(&mut diff_opts))?
    };

    let mut patch_text = String::new();
    let mut current_hunk_idx: i32 = -1;
    let mut file_header_written = false;

    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        if hunk.is_none() {
            if !file_header_written {
                let content = String::from_utf8_lossy(line.content());
                match line.origin() {
                    'F' => patch_text.push_str(&content),
                    _ => {
                        let prefix = match line.origin() {
                            '+' | '-' | ' ' | '>' | '<' => String::from(line.origin()),
                            _ => String::new(),
                        };
                        patch_text.push_str(&prefix);
                        patch_text.push_str(&content);
                    }
                }
            }
            return true;
        }

        let hunk = hunk.unwrap();
        let header = String::from_utf8_lossy(hunk.header()).to_string();

        let is_new_hunk = if current_hunk_idx < 0 {
            true
        } else {
            current_hunk_idx >= 0 && !patch_text.contains(&header)
        };

        if is_new_hunk || current_hunk_idx < 0 {
            current_hunk_idx += 1;
        }

        if current_hunk_idx as usize == hunk_index {
            if !file_header_written {
                let old_path = delta.old_file().path().unwrap_or(Path::new(""));
                let new_path = delta.new_file().path().unwrap_or(Path::new(""));
                patch_text.clear();
                patch_text.push_str(&format!("--- a/{}\n", old_path.display()));
                patch_text.push_str(&format!("+++ b/{}\n", new_path.display()));
                file_header_written = true;
            }

            let content = String::from_utf8_lossy(line.content());
            match line.origin() {
                'H' => patch_text.push_str(&content),
                '+' => {
                    patch_text.push('+');
                    patch_text.push_str(&content);
                }
                '-' => {
                    patch_text.push('-');
                    patch_text.push_str(&content);
                }
                ' ' => {
                    patch_text.push(' ');
                    patch_text.push_str(&content);
                }
                _ => {}
            }
        }

        true
    })?;

    if patch_text.is_empty() {
        anyhow::bail!("Could not generate patch for hunk {}", hunk_index);
    }

    if !patch_text.ends_with('\n') {
        patch_text.push('\n');
    }

    Ok(patch_text)
}

fn parse_co_authors(message: &str) -> (String, Vec<Signature>) {
    let mut co_authors = Vec::new();
    let mut cleaned_lines = Vec::new();
    let prefix = "co-authored-by:";

    for line in message.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest_lower) = lower.strip_prefix(prefix) {
            let rest = &trimmed[prefix.len()..];
            let rest = rest.trim();
            if let Some(email_start) = rest.find('<') {
                if let Some(email_end) = rest.find('>') {
                    let name = rest[..email_start].trim().to_string();
                    let email = rest[email_start + 1..email_end].trim().to_string();
                    if !name.is_empty() && !email.is_empty() {
                        co_authors.push(Signature { name, email });
                        continue;
                    }
                }
            }
            let _ = rest_lower;
        }
        cleaned_lines.push(line);
    }

    let cleaned = cleaned_lines.join("\n");
    let cleaned = cleaned.trim_end().to_string();

    (cleaned, co_authors)
}

/// Gather all refresh data from a repository at the given path.
/// This is a standalone function (no `&self`) so it can run on a background thread.
pub fn gather_refresh_data(repo_path: &Path) -> Result<RefreshData> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {}", repo_path.display()))?;

    // Head
    let head_branch = repo
        .head()
        .ok()
        .and_then(|r| r.shorthand().map(String::from));
    let head_detached = repo.head_detached().unwrap_or(false);
    let repo_state = RepoState::from_git2(repo.state());

    // Branches
    let mut branches = Vec::new();
    {
        let branch_iter = repo.branches(None)?;
        for branch_result in branch_iter {
            let (branch, branch_type) = branch_result?;
            let name = branch.name()?.unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }

            let is_head = branch.is_head();
            let is_remote = branch_type == git2::BranchType::Remote;
            let tip_oid = branch.get().target();

            let upstream = branch
                .upstream()
                .ok()
                .and_then(|u| u.name().ok().flatten().map(String::from));

            let (ahead, behind) =
                if let (Some(local_oid), Ok(upstream_ref)) = (tip_oid, branch.upstream()) {
                    if let Some(remote_oid) = upstream_ref.get().target() {
                        repo.graph_ahead_behind(local_oid, remote_oid)
                            .unwrap_or((0, 0))
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };

            branches.push(BranchInfo {
                name,
                is_head,
                is_remote,
                upstream,
                ahead,
                behind,
                tip_oid,
            });
        }

        branches.sort_by(|a, b| {
            b.is_head
                .cmp(&a.is_head)
                .then(a.is_remote.cmp(&b.is_remote))
                .then(a.name.cmp(&b.name))
        });
    }

    // Tags
    let mut tags = Vec::new();
    let _ = repo.tag_foreach(|oid, name_bytes| {
        if let Ok(name) = std::str::from_utf8(name_bytes) {
            let name = name.strip_prefix("refs/tags/").unwrap_or(name).to_string();
            tags.push(TagInfo {
                name,
                oid,
                message: None,
            });
        }
        true
    });
    tags.sort_by(|a, b| a.name.cmp(&b.name));

    // Remotes
    let mut remotes = Vec::new();
    {
        let remote_names = repo.remotes()?;
        for name in remote_names.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name) {
                remotes.push(RemoteInfo {
                    name: name.to_string(),
                    url: remote.url().map(String::from),
                    push_url: remote.pushurl().map(String::from),
                });
            }
        }
    }

    // Stashes
    let mut stashes = Vec::new();
    {
        let mut repo_mut = Repository::open(repo_path)?;
        repo_mut.stash_foreach(|stash_index, message, oid| {
            stashes.push(StashEntry {
                index: stash_index,
                message: message.to_string(),
                oid: *oid,
            });
            true
        })?;
    }

    // Status
    let status = {
        let mut wt_status = WorkingTreeStatus::default();

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_unmodified(false);

        let statuses = repo.statuses(Some(&mut opts))?;
        let staged_stats = batch_diff_stats(&repo, true);
        let unstaged_stats = batch_diff_stats(&repo, false);
        for entry in statuses.iter() {
            let path = PathBuf::from(entry.path().unwrap_or(""));
            let st = entry.status();

            if st.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                let kind = if st.contains(git2::Status::INDEX_NEW) {
                    FileChangeKind::Added
                } else if st.contains(git2::Status::INDEX_MODIFIED) {
                    FileChangeKind::Modified
                } else if st.contains(git2::Status::INDEX_DELETED) {
                    FileChangeKind::Deleted
                } else if st.contains(git2::Status::INDEX_RENAMED) {
                    FileChangeKind::Renamed
                } else {
                    FileChangeKind::TypeChange
                };
                let &(additions, deletions) = staged_stats.get(&path).unwrap_or(&(0, 0));
                wt_status.staged.push(FileStatus {
                    path: path.clone(),
                    kind,
                    old_path: None,
                    additions,
                    deletions,
                });
            }

            if st.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            ) {
                let kind = if st.contains(git2::Status::WT_NEW) {
                    FileChangeKind::Untracked
                } else if st.contains(git2::Status::WT_MODIFIED) {
                    FileChangeKind::Modified
                } else if st.contains(git2::Status::WT_DELETED) {
                    FileChangeKind::Deleted
                } else if st.contains(git2::Status::WT_RENAMED) {
                    FileChangeKind::Renamed
                } else {
                    FileChangeKind::TypeChange
                };
                let &(additions, deletions) = unstaged_stats.get(&path).unwrap_or(&(0, 0));
                wt_status.unstaged.push(FileStatus {
                    path: path.clone(),
                    kind,
                    old_path: None,
                    additions,
                    deletions,
                });
            }

            if st.contains(git2::Status::CONFLICTED) {
                wt_status.unstaged.push(FileStatus {
                    path,
                    kind: FileChangeKind::Conflicted,
                    old_path: None,
                    additions: 0,
                    deletions: 0,
                });
            }
        }

        wt_status
    };

    // Recent commits (uses branches and tags for ref labels)
    let (recent_commits, has_more_commits) = {
        let limit = 1000;
        let mut commits = Vec::new();

        let mut ref_map = std::collections::HashMap::<git2::Oid, Vec<RefLabel>>::new();

        if let Ok(head) = repo.head() {
            if let Some(oid) = head.target() {
                ref_map.entry(oid).or_default().push(RefLabel::Head);
            }
        }

        for branch in &branches {
            if let Some(oid) = branch.tip_oid {
                let label = if branch.is_remote {
                    RefLabel::RemoteBranch(branch.name.clone())
                } else {
                    RefLabel::LocalBranch(branch.name.clone())
                };
                ref_map.entry(oid).or_default().push(label);
            }
        }

        for tag in &tags {
            ref_map
                .entry(tag.oid)
                .or_default()
                .push(RefLabel::Tag(tag.name.clone()));
        }

        let mut revwalk = repo.revwalk()?;
        let has_head = revwalk.push_head().is_ok();
        for branch in &branches {
            if let Some(oid) = branch.tip_oid {
                revwalk.push(oid).ok();
            }
        }
        if !has_head && branches.is_empty() {
            return Ok(RefreshData {
                head_branch,
                head_detached,
                repo_state,
                branches,
                tags,
                remotes,
                stashes,
                status,
                recent_commits: commits,
                has_more_commits: false,
            });
        }
        revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;

        let mut has_more = false;
        for (count, oid_result) in revwalk.enumerate() {
            if count >= limit {
                has_more = true;
                break;
            }
            let oid = oid_result?;
            let commit = repo.find_commit(oid)?;

            let author = commit.author();
            let committer = commit.committer();
            let time = Utc.timestamp_opt(commit.time().seconds(), 0).single();

            let refs = ref_map.remove(&oid).unwrap_or_default();

            let raw_message = commit.message().unwrap_or("").to_string();
            let (message, co_authors) = parse_co_authors(&raw_message);

            commits.push(CommitInfo {
                oid,
                short_id: format!("{:.7}", oid),
                summary: commit.summary().unwrap_or("").to_string(),
                message,
                author: Signature {
                    name: author.name().unwrap_or("").to_string(),
                    email: author.email().unwrap_or("").to_string(),
                },
                committer: Signature {
                    name: committer.name().unwrap_or("").to_string(),
                    email: committer.email().unwrap_or("").to_string(),
                },
                co_authors,
                time: time.unwrap_or_else(Utc::now),
                parent_oids: commit.parent_ids().collect(),
                refs,
            });
        }

        (commits, has_more)
    };

    Ok(RefreshData {
        head_branch,
        head_detached,
        repo_state,
        branches,
        tags,
        remotes,
        stashes,
        status,
        recent_commits,
        has_more_commits,
    })
}

/// Events emitted by GitProject.
#[derive(Debug, Clone)]
pub enum GitProjectEvent {
    StatusChanged,
    HeadChanged,
    RefsChanged,
    OperationUpdated(GitOperationUpdate),
}

/// The core Git project state holder.
pub struct GitProject {
    repo_path: PathBuf,

    // Cached state
    head_branch: Option<String>,
    head_detached: bool,
    repo_state: RepoState,
    branches: Vec<BranchInfo>,
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    stashes: Vec<StashEntry>,
    status: WorkingTreeStatus,
    recent_commits: Vec<CommitInfo>,
    /// Whether the repository has more commits beyond the loaded set.
    has_more_commits: bool,
    next_operation_id: u64,

    // Filesystem watcher (kept alive)
    _watcher: Option<RecommendedWatcher>,
}

impl EventEmitter<GitProjectEvent> for GitProject {}

impl GitProject {
    /// Open a repository at the given path.
    pub fn open(path: PathBuf, cx: &mut Context<Self>) -> Result<Self> {
        let repo = Repository::open(&path)
            .with_context(|| format!("Failed to open repository at {}", path.display()))?;

        let repo_path = repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf();

        let mut project = Self {
            repo_path,
            head_branch: None,
            head_detached: false,
            repo_state: RepoState::Clean,
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            status: WorkingTreeStatus::default(),
            recent_commits: Vec::new(),
            has_more_commits: false,
            next_operation_id: 1,
            _watcher: None,
        };

        // Set up filesystem watching
        project.start_watcher(cx);

        Ok(project)
    }

    /// Start watching the repository for filesystem changes.
    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let dirty = Arc::new(AtomicBool::new(false));
        let dirty_flag = dirty.clone();

        let watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Ignore .git directory internal changes to avoid feedback loops
                    let dominated_by_git = event
                        .paths
                        .iter()
                        .all(|p| p.components().any(|c| c.as_os_str() == ".git"));
                    if dominated_by_git {
                        return;
                    }
                    match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_) => {
                            dirty_flag.store(true, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            });

        if let Ok(mut watcher) = watcher {
            let _ = watcher.watch(&repo_path, RecursiveMode::Recursive);
            self._watcher = Some(watcher);

            // Poll the dirty flag from async executor — never blocks the UI thread
            let watcher_repo_path = repo_path.clone();
            cx.spawn(async move |weak, cx: &mut AsyncApp| {
                loop {
                    // Async sleep — yields to executor, keeps UI responsive
                    smol::Timer::after(Duration::from_millis(300)).await;

                    if dirty.swap(false, Ordering::Relaxed) {
                        // Debounce: wait a bit more for quiet
                        smol::Timer::after(Duration::from_millis(200)).await;
                        dirty.store(false, Ordering::Relaxed);

                        let path = watcher_repo_path.clone();
                        let data = cx
                            .background_executor()
                            .spawn(async move { gather_refresh_data(&path) })
                            .await;

                        let data = match data {
                            Ok(d) => d,
                            Err(_) => continue,
                        };

                        let result = cx.update(|cx| {
                            weak.update(cx, |this, cx| {
                                this.apply_refresh_data(data);
                                cx.emit(GitProjectEvent::StatusChanged);
                                cx.notify();
                            })
                        });

                        if result.is_err() {
                            break;
                        }
                    }
                }
            })
            .detach();
        }
    }

    fn begin_operation(
        &mut self,
        kind: GitOperationKind,
        summary: impl Into<String>,
        remote_name: Option<String>,
        branch_name: Option<String>,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_operation_id;
        self.next_operation_id += 1;
        cx.emit(GitProjectEvent::OperationUpdated(GitOperationUpdate {
            id,
            kind,
            state: GitOperationState::Running,
            summary: summary.into(),
            details: None,
            remote_name,
            branch_name,
            retryable: false,
        }));
        cx.notify();
        id
    }

    fn complete_op(
        &self,
        id: u64,
        kind: GitOperationKind,
        summary: impl Into<String>,
        names: (Option<String>, Option<String>, Option<String>),
        cx: &mut Context<Self>,
    ) {
        cx.emit(GitProjectEvent::OperationUpdated(GitOperationUpdate {
            id,
            kind,
            state: GitOperationState::Succeeded,
            summary: summary.into(),
            details: names.0,
            remote_name: names.1,
            branch_name: names.2,
            retryable: false,
        }));
    }

    fn fail_op(
        &self,
        id: u64,
        kind: GitOperationKind,
        summary: impl Into<String>,
        error: impl Into<String>,
        names: (Option<String>, Option<String>, bool),
        cx: &mut Context<Self>,
    ) {
        cx.emit(GitProjectEvent::OperationUpdated(GitOperationUpdate {
            id,
            kind,
            state: GitOperationState::Failed,
            summary: summary.into(),
            details: Some(error.into()),
            remote_name: names.0,
            branch_name: names.1,
            retryable: names.2,
        }));
    }

    fn fail_to_start_task(
        &mut self,
        kind: GitOperationKind,
        summary: impl Into<String>,
        error: anyhow::Error,
        retryable: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let summary = summary.into();
        let operation_id =
            self.begin_operation(kind, summary.clone(), None, self.head_branch.clone(), cx);
        self.fail_op(
            operation_id,
            kind,
            summary,
            error.to_string(),
            (None, self.head_branch.clone(), retryable),
            cx,
        );
        cx.spawn(async move |_this: WeakEntity<Self>, _cx: &mut AsyncApp| Err(error))
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn repo_name(&self) -> &str {
        self.repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    }

    pub fn head_branch(&self) -> Option<&str> {
        self.head_branch.as_deref()
    }

    pub fn is_head_detached(&self) -> bool {
        self.head_detached
    }

    pub fn repo_state(&self) -> RepoState {
        self.repo_state
    }

    pub fn branches(&self) -> &[BranchInfo] {
        &self.branches
    }

    pub fn tags(&self) -> &[TagInfo] {
        &self.tags
    }

    pub fn remotes(&self) -> &[RemoteInfo] {
        &self.remotes
    }

    pub fn preferred_remote_name(&self) -> Result<String> {
        let repo = self.open_repo()?;
        default_remote_name(&repo)
    }

    pub fn fetch_default(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let remote_name = match self.preferred_remote_name() {
            Ok(remote_name) => remote_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Fetch,
                    "Fetch could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.fetch(&remote_name, cx)
    }

    pub fn pull_default(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let (remote_name, branch_name) =
            match self.open_repo().and_then(|repo| pull_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::Pull,
                        "Pull could not start",
                        error,
                        true,
                        cx,
                    )
                }
            };
        self.pull_from(&remote_name, &branch_name, cx)
    }

    pub fn push_default(&mut self, force: bool, cx: &mut Context<Self>) -> Task<Result<()>> {
        let (remote_name, branch_name, set_upstream) =
            match self.open_repo().and_then(|repo| push_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::Push,
                        "Push could not start",
                        error,
                        true,
                        cx,
                    )
                }
            };
        self.push_to(&remote_name, &branch_name, force, set_upstream, cx)
    }

    /// Resolve a tag name to the commit OID it points to.
    /// Handles both lightweight and annotated tags by peeling to the commit.
    pub fn resolve_tag_to_oid(&self, tag_name: &str) -> Result<git2::Oid> {
        let repo = self.open_repo()?;
        let obj = repo
            .revparse_single(&format!("refs/tags/{}", tag_name))
            .with_context(|| format!("Failed to resolve tag '{}'", tag_name))?;
        let commit = obj
            .peel_to_commit()
            .with_context(|| format!("Tag '{}' does not point to a commit", tag_name))?;
        Ok(commit.id())
    }

    /// Resolve a branch name to the commit OID it points to.
    /// Tries local branch first, then remote, then raw revparse.
    pub fn resolve_branch_to_oid(&self, branch_name: &str) -> Result<git2::Oid> {
        let repo = self.open_repo()?;
        let refs_to_try = [
            format!("refs/heads/{}", branch_name),
            format!("refs/remotes/{}", branch_name),
            branch_name.to_string(),
        ];
        for refspec in &refs_to_try {
            if let Ok(obj) = repo.revparse_single(refspec) {
                if let Ok(commit) = obj.peel_to_commit() {
                    return Ok(commit.id());
                }
            }
        }
        anyhow::bail!("Failed to resolve branch '{}' to a commit", branch_name)
    }

    pub fn stashes(&self) -> &[StashEntry] {
        &self.stashes
    }

    pub fn status(&self) -> &WorkingTreeStatus {
        &self.status
    }

    pub fn recent_commits(&self) -> &[CommitInfo] {
        &self.recent_commits
    }

    pub fn has_changes(&self) -> bool {
        !self.status.staged.is_empty() || !self.status.unstaged.is_empty()
    }

    /// Returns the list of conflicted file paths from the unstaged changes.
    pub fn conflicted_files(&self) -> Vec<&FileStatus> {
        self.status
            .unstaged
            .iter()
            .filter(|f| f.kind == FileChangeKind::Conflicted)
            .collect()
    }

    /// Whether the working tree has any conflicted files.
    pub fn has_conflicts(&self) -> bool {
        self.status
            .unstaged
            .iter()
            .any(|f| f.kind == FileChangeKind::Conflicted)
    }

    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.repo_path)
            .with_context(|| format!("Failed to open repository at {}", self.repo_path.display()))
    }

    /// Apply pre-gathered refresh data to self.
    fn apply_refresh_data(&mut self, data: RefreshData) {
        self.head_branch = data.head_branch;
        self.head_detached = data.head_detached;
        self.repo_state = data.repo_state;
        self.branches = data.branches;
        self.tags = data.tags;
        self.remotes = data.remotes;
        self.stashes = data.stashes;
        self.status = data.status;
        self.recent_commits = data.recent_commits;
        self.has_more_commits = data.has_more_commits;
    }

    /// Whether there are more commits beyond the currently loaded set.
    pub fn has_more_commits(&self) -> bool {
        self.has_more_commits
    }

    /// Refresh all state asynchronously on a background thread.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let data = cx
                .background_executor()
                .spawn(async move { gather_refresh_data(&repo_path) })
                .await?;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.apply_refresh_data(data);
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    // -- Write operations --

    /// Stage specific files.
    pub fn stage_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let task_paths = paths.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            if paths.len() == 1 {
                format!("Staging {}...", paths[0].display())
            } else {
                format!("Staging {} files...", paths.len())
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut index = repo.index()?;
                    for path in &task_paths {
                        if repo_path.join(path).exists() {
                            index.add_path(path)?;
                        } else {
                            index.remove_path(path)?;
                        }
                    }
                    index.write()?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stage,
                                if paths.len() == 1 {
                                    format!("Staged {}", paths[0].display())
                                } else {
                                    format!("Staged {} files", paths.len())
                                },
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Stage failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Unstage specific files.
    pub fn unstage_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let task_paths = paths.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            if paths.len() == 1 {
                format!("Unstaging {}...", paths[0].display())
            } else {
                format!("Unstaging {} files...", paths.len())
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    if let Ok(head_tree) = repo.head().and_then(|h| h.peel_to_tree()) {
                        repo.reset_default(Some(&head_tree.into_object()), &task_paths)?;
                    } else {
                        let mut index = repo.index()?;
                        for path in &task_paths {
                            let _ = index.remove_path(path);
                        }
                        index.write()?;
                    }
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                if paths.len() == 1 {
                                    format!("Unstaged {}", paths[0].display())
                                } else {
                                    format!("Unstaged {} files", paths.len())
                                },
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstage failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Stage all changes.
    pub fn stage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            "Staging all changes...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut index = repo.index()?;
                    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                    index.write()?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Staged all changes",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Stage all failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Unstage all changes.
    pub fn unstage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            "Unstaging all changes...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    if let Ok(head) = repo.head() {
                        let obj = head.peel(git2::ObjectType::Any)?;
                        repo.reset(&obj, git2::ResetType::Mixed, None)?;
                    }
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstaged all changes",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstage all failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Create a commit with the current staged changes.
    pub fn commit(
        &mut self,
        message: &str,
        amend: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<git2::Oid>> {
        let message = message.to_string();
        let task_message = message.clone();
        let commit_summary = message.lines().next().unwrap_or("").to_string();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Commit,
            if amend {
                "Amending commit..."
            } else {
                "Creating commit..."
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(git2::Oid, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let sig = repo.signature()?;
                    let mut index = repo.index()?;
                    if index.is_empty() {
                        anyhow::bail!("There are no staged changes to commit.")
                    }
                    let tree_oid = index.write_tree()?;
                    let tree = repo.find_tree(tree_oid)?;

                    let oid = if amend {
                        let head = repo.head()?.peel_to_commit()?;
                        head.amend(
                            Some("HEAD"),
                            Some(&sig),
                            Some(&sig),
                            None,
                            Some(&task_message),
                            Some(&tree),
                        )?
                    } else {
                        let parents: Vec<git2::Commit> = if let Ok(head) = repo.head() {
                            vec![head.peel_to_commit()?]
                        } else {
                            vec![]
                        };
                        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                        repo.commit(Some("HEAD"), &sig, &sig, &task_message, &tree, &parent_refs)?
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((oid, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| match result {
                    Ok((oid, data)) => {
                        this.apply_refresh_data(data);
                        this.complete_op(
                            operation_id,
                            GitOperationKind::Commit,
                            if amend {
                                format!("Amended commit {}", &oid.to_string()[..7])
                            } else {
                                format!("Created commit {}", &oid.to_string()[..7])
                            },
                            (Some(commit_summary.clone()), None, branch_name.clone()),
                            cx,
                        );
                        cx.emit(GitProjectEvent::HeadChanged);
                        cx.emit(GitProjectEvent::StatusChanged);
                        cx.notify();
                        Ok(oid)
                    }
                    Err(e) => {
                        this.fail_op(
                            operation_id,
                            GitOperationKind::Commit,
                            if amend {
                                "Amend failed"
                            } else {
                                "Commit failed"
                            },
                            e.to_string(),
                            (None, branch_name.clone(), false),
                            cx,
                        );
                        Err(e)
                    }
                })
            })?
        })
    }

    /// Get diff for a specific file (staged or unstaged).
    pub fn diff_file(&self, path: &Path, staged: bool) -> Result<FileDiff> {
        let repo = self.open_repo()?;
        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(path);
        diff_opts.include_untracked(true);
        diff_opts.show_untracked_content(true);

        let diff = if staged {
            let head_tree = repo.head()?.peel_to_tree().ok();
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?
        } else {
            repo.diff_index_to_workdir(None, Some(&mut diff_opts))?
        };

        parse_file_diff(path, &diff)
    }

    /// Get diff for a specific commit.
    pub fn diff_commit(&self, oid: git2::Oid) -> Result<CommitDiff> {
        let repo = self.open_repo()?;
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let stats = diff.stats()?;

        let mut files: Vec<FileDiff> = Vec::new();
        let mut current_hunks: Vec<DiffHunk> = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_additions: usize = 0;
        let mut current_deletions: usize = 0;
        let mut current_kind = FileChangeKind::Modified;

        diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
            let delta_path = delta
                .new_file()
                .path()
                .unwrap_or(Path::new(""))
                .to_path_buf();

            if current_path.as_ref() != Some(&delta_path) {
                if let Some(prev_path) = current_path.take() {
                    files.push(FileDiff {
                        path: prev_path,
                        hunks: std::mem::take(&mut current_hunks),
                        additions: current_additions,
                        deletions: current_deletions,
                        kind: current_kind,
                    });
                }
                current_path = Some(delta_path);
                current_additions = 0;
                current_deletions = 0;
                current_kind = delta_to_file_change_kind(delta.status());
            }

            if let Some(hunk) = hunk {
                let header = String::from_utf8_lossy(hunk.header()).to_string();
                let expected_start = hunk.new_start();
                let needs_new = current_hunks.last().is_none_or(|h| {
                    h.new_start != expected_start || h.header != header
                });
                if needs_new {
                    current_hunks.push(DiffHunk {
                        old_start: hunk.old_start(),
                        old_lines: hunk.old_lines(),
                        new_start: hunk.new_start(),
                        new_lines: hunk.new_lines(),
                        header,
                        lines: Vec::new(),
                    });
                }
            }

            let content = String::from_utf8_lossy(line.content()).to_string();
            match line.origin() {
                '+' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Addition(content));
                    }
                    current_additions += 1;
                }
                '-' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Deletion(content));
                    }
                    current_deletions += 1;
                }
                ' ' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Context(content));
                    }
                }
                _ => {}
            }

            true
        })?;

        if let Some(path) = current_path {
            files.push(FileDiff {
                path,
                hunks: current_hunks,
                additions: current_additions,
                deletions: current_deletions,
                kind: current_kind,
            });
        }

        Ok(CommitDiff {
            total_additions: stats.insertions(),
            total_deletions: stats.deletions(),
            files,
        })
    }

    /// Get the diff for a stash entry at the given index.
    pub fn diff_stash(&self, index: usize) -> Result<CommitDiff> {
        let mut repo = self.open_repo()?;

        // Collect stash OIDs using stash_foreach
        let mut stash_oids: Vec<git2::Oid> = Vec::new();
        repo.stash_foreach(|_idx, _msg, oid| {
            stash_oids.push(*oid);
            true
        })?;

        let oid = *stash_oids
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Stash index {} out of range", index))?;

        // A stash commit's first parent is the commit it was created on top of
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let stats = diff.stats()?;

        let mut files: Vec<FileDiff> = Vec::new();
        let mut current_hunks: Vec<DiffHunk> = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_additions: usize = 0;
        let mut current_deletions: usize = 0;
        let mut current_kind = FileChangeKind::Modified;

        diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
            let delta_path = delta
                .new_file()
                .path()
                .unwrap_or(Path::new(""))
                .to_path_buf();

            if current_path.as_ref() != Some(&delta_path) {
                if let Some(prev_path) = current_path.take() {
                    files.push(FileDiff {
                        path: prev_path,
                        hunks: std::mem::take(&mut current_hunks),
                        additions: current_additions,
                        deletions: current_deletions,
                        kind: current_kind,
                    });
                }
                current_path = Some(delta_path);
                current_additions = 0;
                current_deletions = 0;
                current_kind = delta_to_file_change_kind(delta.status());
            }

            if let Some(hunk) = hunk {
                let header = String::from_utf8_lossy(hunk.header()).to_string();
                let expected_start = hunk.new_start();
                let needs_new = current_hunks.last().is_none_or(|h| {
                    h.new_start != expected_start || h.header != header
                });
                if needs_new {
                    current_hunks.push(DiffHunk {
                        old_start: hunk.old_start(),
                        old_lines: hunk.old_lines(),
                        new_start: hunk.new_start(),
                        new_lines: hunk.new_lines(),
                        header,
                        lines: Vec::new(),
                    });
                }
            }

            let content = String::from_utf8_lossy(line.content()).to_string();
            match line.origin() {
                '+' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Addition(content));
                    }
                    current_additions += 1;
                }
                '-' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Deletion(content));
                    }
                    current_deletions += 1;
                }
                ' ' => {
                    if let Some(h) = current_hunks.last_mut() {
                        h.lines.push(DiffLine::Context(content));
                    }
                }
                _ => {}
            }

            true
        })?;

        if let Some(path) = current_path {
            files.push(FileDiff {
                path,
                hunks: current_hunks,
                additions: current_additions,
                deletions: current_deletions,
                kind: current_kind,
            });
        }

        Ok(CommitDiff {
            total_additions: stats.insertions(),
            total_deletions: stats.deletions(),
            files,
        })
    }

    /// Checkout a branch by name.
    pub fn checkout_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Checkout,
            format!("Switching to '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Checkout")?;
                    let current_branch = head_branch_name(&repo).ok();
                    if current_branch.as_deref() == Some(task_name.as_str()) {
                        anyhow::bail!("'{}' is already checked out.", task_name);
                    }
                    let obj = repo.revparse_single(&format!("refs/heads/{}", task_name))?;
                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.safe();
                    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                    repo.set_head(&format!("refs/heads/{}", task_name))?;
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((format!("Switched to '{}'", task_name), data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                msg,
                                (Some("Working tree updated for the selected branch.".into()), None, Some(name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checkout of '{}' failed", name),
                                e.to_string(),
                                (None, Some(name.clone()), true),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Create a new branch from HEAD.
    pub fn create_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        self.create_branch_at(name, None, cx)
    }

    /// Create a new branch, optionally at a specific commit (SHA or ref).
    /// If `base_ref` is None or empty, creates at HEAD.
    pub fn create_branch_at(
        &mut self,
        name: &str,
        base_ref: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        let base_ref = base_ref.map(|s| s.to_string());
        let task_name = name.clone();
        let task_base_ref = base_ref.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Creating branch '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let target = if let Some(ref r) = task_base_ref {
                        if r.is_empty() {
                            repo.head()?.peel_to_commit()?
                        } else {
                            let obj = repo.revparse_single(r)?;
                            obj.peel_to_commit().map_err(|_| {
                                anyhow::anyhow!("'{}' does not resolve to a commit", r)
                            })?
                        }
                    } else {
                        repo.head()?.peel_to_commit()?
                    };
                    repo.branch(&task_name, &target, false)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Created branch '{}'", name),
                                (base_ref.as_ref().map(|value| format!("Base: {}", value)), None, Some(name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Branch '{}' could not be created", name),
                                e.to_string(),
                                (None, Some(name.clone()), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Delete a local branch.
    pub fn delete_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Deleting branch '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut branch = repo.find_branch(&task_name, git2::BranchType::Local)?;
                    branch.delete()?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Deleted branch '{}'", name),
                                (None, None, Some(name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Delete branch '{}' failed", name),
                                e.to_string(),
                                (None, Some(name.clone()), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Rename a local branch.
    pub fn rename_branch(
        &mut self,
        old_name: &str,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let old_name = old_name.to_string();
        let new_name = new_name.to_string();
        let task_old_name = old_name.clone();
        let task_new_name = new_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Renaming branch '{}'...", old_name),
            None,
            Some(old_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut branch = repo.find_branch(&task_old_name, git2::BranchType::Local)?;
                    branch.rename(&task_new_name, false)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Renamed '{}' to '{}'", old_name, new_name),
                                (None, None, Some(new_name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Rename branch '{}' failed", old_name),
                                e.to_string(),
                                (None, Some(old_name.clone()), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Create a lightweight tag at the given commit.
    pub fn create_tag(
        &mut self,
        name: &str,
        target_oid: git2::Oid,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Tag,
            format!("Creating tag '{}'...", name),
            None,
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let obj = repo.find_object(target_oid, None)?;
                    repo.tag_lightweight(&task_name, &obj, false)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Created tag '{}'", name),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Tag '{}' could not be created", name),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Delete a tag by name.
    pub fn delete_tag(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Tag,
            format!("Deleting tag '{}'...", name),
            None,
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    repo.tag_delete(&task_name)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Deleted tag '{}'", name),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Delete tag '{}' failed", name),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Drop a stash entry without applying it.
    pub fn stash_drop(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Dropping stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_drop(index)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Dropped stash #{}", index),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Drop stash #{} failed", index),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Apply a stash entry without removing it from the stash list.
    pub fn stash_apply(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Applying stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_apply(index, None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Applied stash #{}", index),
                                (Some("The stash entry was kept.".into()), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Apply stash #{} failed", index),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Discard changes in specific files (restore to HEAD).
    pub fn discard_changes(
        &mut self,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let operation_id = self.begin_operation(
            GitOperationKind::Discard,
            if paths.len() == 1 {
                format!("Discarding changes in {}...", paths[0].display())
            } else {
                format!("Discarding changes in {} files...", paths.len())
            },
            None,
            self.head_branch.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move {
                let repo = Repository::open(&repo_path)?;
                let workdir = repo
                    .workdir()
                    .ok_or_else(|| anyhow::anyhow!("Bare repository has no working directory"))?
                    .to_path_buf();
                let mut checkout_opts = git2::build::CheckoutBuilder::new();
                checkout_opts.force();
                let mut has_tracked = false;
                for path in &paths {
                    let is_untracked = repo
                        .status_file(path)
                        .map(|s| s.contains(git2::Status::WT_NEW))
                        .unwrap_or(false);
                    if is_untracked {
                        let full = workdir.join(path);
                        if full.is_file() {
                            std::fs::remove_file(&full).with_context(|| format!("Failed to delete {}", full.display()))?;
                        } else if full.is_dir() {
                            std::fs::remove_dir_all(&full).with_context(|| format!("Failed to delete directory {}", full.display()))?;
                        }
                    } else {
                        checkout_opts.path(path);
                        has_tracked = true;
                    }
                }
                if has_tracked {
                    repo.checkout_head(Some(&mut checkout_opts))?;
                }
                let data = gather_refresh_data(&repo_path)?;
                Ok::<_, anyhow::Error>(data)
            }).await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Discard,
                                "Discarded changes",
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Discard,
                                "Discard changes failed",
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Hard reset to HEAD, discarding all working tree and index changes.
    pub fn reset_hard(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            "Resetting working tree to HEAD...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let head_commit = repo.head()?.peel_to_commit()?;
                    repo.reset(head_commit.as_object(), git2::ResetType::Hard, None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Reset,
                                "Reset working tree to HEAD",
                                (Some("All staged and unstaged changes were discarded.".into()), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                "Reset to HEAD failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Hard-reset the current branch to a specific commit.
    pub fn reset_to_commit(
        &mut self,
        oid: git2::Oid,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            format!("Resetting to {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let commit = repo.find_commit(oid)?;
                    repo.reset(commit.as_object(), git2::ResetType::Hard, None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Reset,
                                format!("Reset to {}", short_id),
                                (Some("Working tree reset to the selected commit.".into()), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                format!("Reset to {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Revert a commit (creates a new commit that undoes the given commit).
    pub fn revert_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Revert,
            format!("Reverting {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData, bool)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Revert")?;
                    let commit = repo.find_commit(oid)?;
                    let summary = commit.summary().unwrap_or("").to_string();
                    let mut opts = git2::RevertOptions::new();
                    repo.revert(&commit, Some(&mut opts))?;
                    let has_conflicts = repo.index()?.has_conflicts();
                    if !has_conflicts {
                        repo.cleanup_state()?;
                    }
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data, has_conflicts))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data, has_conflicts)) => {
                            this.apply_refresh_data(data);
                            cx.emit(GitProjectEvent::StatusChanged);
                            if has_conflicts {
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Revert,
                                    format!("Revert of {} needs conflict resolution", short_id),
                                    "Resolve the conflicts in the working tree, then commit the revert manually.".to_string(),
                                    (None, branch_name.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Revert,
                                    format!("Reverted {}", short_id),
                                    (Some(format!(
                                        "Revert for '{}' has been applied. Review the changes and commit them manually.",
                                        summary
                                    )), None, branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Revert,
                                format!("Revert of {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Cherry-pick a commit onto the current HEAD.
    pub fn cherry_pick(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::CherryPick,
            format!("Cherry-picking {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData, bool)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Cherry-pick")?;
                    let commit = repo.find_commit(oid)?;
                    let summary = commit.summary().unwrap_or("").to_string();
                    let mut opts = git2::CherrypickOptions::new();
                    repo.cherrypick(&commit, Some(&mut opts))?;
                    let has_conflicts = repo.index()?.has_conflicts();
                    if !has_conflicts {
                        repo.cleanup_state()?;
                    }
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data, has_conflicts))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data, has_conflicts)) => {
                            this.apply_refresh_data(data);
                            cx.emit(GitProjectEvent::StatusChanged);
                            if has_conflicts {
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::CherryPick,
                                    format!("Cherry-pick of {} needs conflict resolution", short_id),
                                    "Resolve the conflicts in the working tree, then commit the cherry-pick manually.".to_string(),
                                    (None, branch_name.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::CherryPick,
                                    format!("Cherry-picked {}", short_id),
                                    (Some(format!(
                                        "Cherry-pick for '{}' has been applied. Review the changes and commit them manually.",
                                        summary
                                    )), None, branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::CherryPick,
                                format!("Cherry-pick of {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Abort the current in-progress operation (merge, rebase, cherry-pick, revert).
    /// Resets the working tree and index to HEAD and cleans up the repo state.
    pub fn abort_operation(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let state_label = self.repo_state.label().to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge, // closest match
            format!("Aborting {}...", state_label.to_lowercase()),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    // Reset index and working tree to HEAD
                    let head = repo.head()?.peel_to_commit()?;
                    repo.reset(
                        head.as_object(),
                        git2::ResetType::Hard,
                        Some(git2::build::CheckoutBuilder::new().force()),
                    )?;
                    repo.cleanup_state()?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Merge,
                                format!("{} aborted", state_label),
                                (Some("Working tree has been reset to HEAD.".into()), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                format!("Failed to abort {}", state_label.to_lowercase()),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Continue the current merge by committing with the default merge message.
    /// This stages all files and creates the merge commit.
    pub fn continue_merge(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge,
            "Continuing merge...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;

                    // Ensure we're actually in a merge state
                    let state = repo.state();
                    if state == git2::RepositoryState::Clean {
                        anyhow::bail!("Repository is not in a merge state");
                    }

                    // Check for remaining conflicts
                    let mut index = repo.index()?;
                    if index.has_conflicts() {
                        anyhow::bail!(
                            "There are still unresolved conflicts. Resolve all conflicts before continuing."
                        );
                    }

                    // Stage everything and write tree
                    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                    index.write()?;
                    let tree_oid = index.write_tree()?;
                    let tree = repo.find_tree(tree_oid)?;

                    // Build the commit — need to find MERGE_HEAD for merge commits
                    let sig = repo.signature()?;
                    let head_commit = repo.head()?.peel_to_commit()?;

                    // Read merge message if available
                    let merge_msg_path = repo.path().join("MERGE_MSG");
                    let message = if merge_msg_path.exists() {
                        std::fs::read_to_string(&merge_msg_path)
                            .unwrap_or_else(|_| "Merge commit".to_string())
                    } else {
                        "Merge commit".to_string()
                    };

                    // Collect parent commits (HEAD + MERGE_HEAD(s))
                    let mut parents = vec![head_commit.clone()];
                    let merge_head_path = repo.path().join("MERGE_HEAD");
                    if merge_head_path.exists() {
                        let contents = std::fs::read_to_string(&merge_head_path)?;
                        for line in contents.lines() {
                            let line = line.trim();
                            if !line.is_empty() {
                                let oid = git2::Oid::from_str(line)?;
                                parents.push(repo.find_commit(oid)?);
                            }
                        }
                    }

                    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parent_refs)?;
                    repo.cleanup_state()?;

                    let summary = message.lines().next().unwrap_or("Merge commit").to_string();
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Merge,
                                "Merge completed",
                                (Some(summary), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                "Continue merge failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Perform an interactive rebase using a prepared plan.
    ///
    /// Writes the desired todo list to a temp file and invokes `git rebase -i`
    /// with `GIT_SEQUENCE_EDITOR` set to a command that replaces the editor file
    /// with the prepared plan. For reword actions, uses `exec git commit --amend`
    /// lines after the pick.
    pub fn rebase_interactive(
        &mut self,
        entries: Vec<RebasePlanEntry>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let entry_count = entries.len();
        let operation_id = self.begin_operation(
            GitOperationKind::Rebase,
            format!("Rebasing {} commits...", entry_count),
            None,
            branch_name.clone(),
            cx,
        );

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    if entries.is_empty() {
                        anyhow::bail!("No entries provided for interactive rebase");
                    }

                    let base_oid = {
                        let repo = Repository::open(&repo_path)?;
                        ensure_clean_worktree(&repo, "Interactive rebase")?;
                        let last_oid = &entries.last().unwrap().oid;
                        let last_commit =
                            repo.find_commit(git2::Oid::from_str(last_oid)?)?;
                        let base_commit =
                            last_commit.parent(0).with_context(|| {
                                format!("Commit {} has no parent", last_oid)
                            })?;
                        base_commit.id().to_string()
                    };

                    let mut todo_lines = Vec::new();
                    for entry in entries.iter().rev() {
                        let short_oid = if entry.oid.len() >= 7 {
                            &entry.oid[..7]
                        } else {
                            &entry.oid
                        };

                        match &entry.action {
                            RebaseEntryAction::Pick => {
                                todo_lines.push(format!(
                                    "pick {} {}",
                                    short_oid, entry.message
                                ));
                            }
                            RebaseEntryAction::Reword(new_msg) => {
                                todo_lines.push(format!(
                                    "pick {} {}",
                                    short_oid, entry.message
                                ));
                                let escaped = new_msg
                                    .replace('\\', "\\\\")
                                    .replace('"', "\\\"");
                                todo_lines.push(format!(
                                    "exec git commit --amend -m \"{}\"",
                                    escaped
                                ));
                            }
                            RebaseEntryAction::Squash => {
                                todo_lines.push(format!(
                                    "squash {} {}",
                                    short_oid, entry.message
                                ));
                            }
                            RebaseEntryAction::Fixup => {
                                todo_lines.push(format!(
                                    "fixup {} {}",
                                    short_oid, entry.message
                                ));
                            }
                            RebaseEntryAction::Drop => {
                                todo_lines.push(format!(
                                    "drop {} {}",
                                    short_oid, entry.message
                                ));
                            }
                        }
                    }

                    let todo_content = todo_lines.join("\n") + "\n";

                    let temp_dir = std::env::temp_dir();
                    let todo_file = temp_dir.join(format!(
                        "rgitui_rebase_todo_{}",
                        std::process::id()
                    ));
                    std::fs::write(&todo_file, &todo_content)?;

                    let sequence_editor = if cfg!(windows) {
                        format!(
                            "cmd /c copy /y \"{}\" ",
                            todo_file.to_string_lossy().replace('/', "\\")
                        )
                    } else {
                        format!(
                            "cp \"{}\" ",
                            todo_file.to_string_lossy()
                        )
                    };

                    let output = Command::new("git")
                        .current_dir(&repo_path)
                        .env("GIT_SEQUENCE_EDITOR", &sequence_editor)
                        .env("GIT_EDITOR", "true")
                        .args(["rebase", "-i", &base_oid])
                        .output()
                        .with_context(|| "Failed to execute git rebase -i")?;

                    let _ = std::fs::remove_file(&todo_file);

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if output.status.success() {
                        let data = gather_refresh_data(&repo_path)?;
                        Ok((
                            format!(
                                "Rebased {} commits successfully",
                                entry_count
                            ),
                            data,
                        ))
                    } else if stderr.contains("CONFLICT")
                        || stderr.contains("could not apply")
                    {
                        let data = gather_refresh_data(&repo_path)?;
                        let detail: String = stderr
                            .lines()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(" ");
                        Ok((
                            format!(
                                "CONFLICT:Rebase paused due to conflicts. \
                                 Resolve conflicts and use Continue or Abort. {}",
                                detail
                            ),
                            data,
                        ))
                    } else {
                        anyhow::bail!(
                            "git rebase -i failed:\nstdout: {}\nstderr: {}",
                            stdout.trim(),
                            stderr.trim()
                        );
                    }
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, data)) => {
                            let is_conflict = msg.starts_with("CONFLICT:");
                            this.apply_refresh_data(data);
                            if is_conflict {
                                let user_msg = msg
                                    .trim_start_matches("CONFLICT:")
                                    .to_string();
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Rebase,
                                    "Rebase paused due to conflicts",
                                    user_msg,
                                    (None, branch_name.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Rebase,
                                    msg,
                                    (Some(
                                        "Repository state refreshed after rebase."
                                            .into(),
                                    ), None, branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Rebase,
                                "Interactive rebase failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Checkout a specific commit (detached HEAD).
    pub fn checkout_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Checkout,
            format!("Checking out {}...", short_id),
            None,
            Some(short_id.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Checkout")?;
                    let commit = repo.find_commit(oid)?;
                    let obj = commit.into_object();
                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.safe();
                    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                    repo.set_head_detached(oid)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checked out {}", short_id),
                                (Some("HEAD is now detached at the selected commit.".into()), None, Some(short_id.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checkout of {} failed", short_id),
                                e.to_string(),
                                (None, Some(short_id.clone()), true),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Remove a remote by name.
    pub fn remove_remote(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::RemoveRemote,
            format!("Removing remote '{}'...", name),
            Some(name.clone()),
            branch_name.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move {
                let repo = Repository::open(&repo_path)?;
                repo.remote_delete(&name)?;
                let data = gather_refresh_data(&repo_path)?;
                Ok::<_, anyhow::Error>((data, name))
            }).await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((data, name)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::RemoveRemote,
                                format!("Removed remote '{}'", name),
                                (Some("Remote list refreshed.".into()), Some(name.clone()), branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::RemoveRemote,
                                "Removing remote failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Fetch from a remote.
    pub fn fetch(&mut self, remote_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        let task_remote_name = remote_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Fetch,
            format!("Fetching from '{}'...", remote_name),
            Some(remote_name.clone()),
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let details = {
                        let repo = Repository::open(&repo_path)?;
                        if remote_uses_ssh(&repo, &task_remote_name)? {
                            drop(repo);
                            run_git_network_command(
                                &repo_path,
                                &["fetch", "--prune", &task_remote_name],
                            )?
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let mut fetch_opts = make_fetch_options();
                            remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
                            None
                        }
                    };
                    let data = gather_refresh_data(&repo_path)?;
                    anyhow::Ok((details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((details, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Fetch,
                                format!("Fetched from '{}'", remote_name),
                                (details.or(Some("Remote refs refreshed.".into())), Some(remote_name.clone()), this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            log::error!("Fetch failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Fetch,
                                format!("Fetch from '{}' failed", remote_name),
                                e.to_string(),
                                (Some(remote_name.clone()), this.head_branch.clone(), true),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Pull from a remote (fetch + merge).
    pub fn pull(&mut self, remote_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let branch_name = match self
            .open_repo()
            .and_then(|repo| pull_target(&repo, Some(remote_name)).map(|(_, branch)| branch))
        {
            Ok(branch_name) => branch_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Pull,
                    "Pull could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.pull_from(remote_name, &branch_name, cx)
    }

    pub fn pull_from(
        &mut self,
        remote_name: &str,
        branch_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        let branch_name = branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_branch_name = branch_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Pull,
            format!("Pulling '{}' from '{}'...", branch_name, remote_name),
            Some(remote_name.clone()),
            Some(branch_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, details) = {
                        let repo = Repository::open(&repo_path)?;
                        ensure_clean_worktree(&repo, "Pull")?;

                        if remote_uses_ssh(&repo, &task_remote_name)? {
                            drop(repo);
                            let details = run_git_network_command(
                                &repo_path,
                                &["pull", &task_remote_name, &task_branch_name],
                            )?;
                            let msg = if details
                                .as_deref()
                                .map(|text| text.contains("Already up to date."))
                                .unwrap_or(false)
                            {
                                format!(
                                    "'{}' is already up to date with '{}'",
                                    task_branch_name, task_remote_name
                                )
                            } else {
                                format!(
                                    "Pulled '{}' from '{}' using system Git",
                                    task_branch_name, task_remote_name
                                )
                            };
                            (msg, details)
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let mut fetch_opts = make_fetch_options();
                            remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
                            drop(remote);

                            let fetch_head = repo.find_reference("FETCH_HEAD")?;
                            let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
                            drop(fetch_head);

                            let (analysis, _pref) = repo.merge_analysis(&[&fetch_commit])?;

                            let msg = if analysis.is_up_to_date() {
                                format!(
                                    "'{}' is already up to date with '{}'",
                                    task_branch_name, task_remote_name
                                )
                            } else if analysis.is_fast_forward() {
                                let refname = format!("refs/heads/{}", task_branch_name);
                                let mut reference = repo.find_reference(&refname)?;
                                reference.set_target(fetch_commit.id(), "Fast-forward pull")?;
                                repo.set_head(&refname)?;
                                repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
                                format!(
                                    "Pulled '{}' from '{}' (fast-forward)",
                                    task_branch_name, task_remote_name
                                )
                            } else if analysis.is_normal() {
                                repo.merge(&[&fetch_commit], None, None)?;
                                let has_conflicts = repo.index()?.has_conflicts();
                                if !has_conflicts {
                                    let sig = repo.signature()?;
                                    let mut index = repo.index()?;
                                    let tree_oid = index.write_tree()?;
                                    let tree = repo.find_tree(tree_oid)?;
                                    let head_commit = repo.head()?.peel_to_commit()?;
                                    let fetch_commit_obj = repo.find_commit(fetch_commit.id())?;
                                    repo.commit(
                                        Some("HEAD"),
                                        &sig,
                                        &sig,
                                        &format!(
                                            "Merge remote-tracking branch '{}/{}'",
                                            task_remote_name, task_branch_name
                                        ),
                                        &tree,
                                        &[&head_commit, &fetch_commit_obj],
                                    )?;
                                    repo.cleanup_state()?;
                                    format!(
                                        "Pulled '{}' from '{}' with a merge commit",
                                        task_branch_name, task_remote_name
                                    )
                                } else {
                                    let conflict_count = repo
                                        .index()?
                                        .conflicts()?
                                        .count();
                                    format!(
                                        "CONFLICT:{} conflict(s) during pull from '{}/{}'. Resolve and continue.",
                                        conflict_count, task_remote_name, task_branch_name
                                    )
                                }
                            } else {
                                format!("Pulled '{}' from '{}'", task_branch_name, task_remote_name)
                            };
                            (msg, None)
                        }
                    }; // repo dropped here

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((msg, details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, details, data)) => {
                            let is_conflict = msg.starts_with("CONFLICT:");
                            this.apply_refresh_data(data);
                            if is_conflict {
                                let user_msg = msg.trim_start_matches("CONFLICT:").to_string();
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    format!("Pull from '{}' has conflicts", remote_name),
                                    user_msg,
                                    (Some(remote_name.clone()), Some(branch_name.clone()), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    msg,
                                    (details.or(Some(
                                        "Repository state refreshed after pull.".into(),
                                    )), Some(remote_name.clone()), Some(branch_name.clone())),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            log::error!("Pull failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Pull,
                                format!("Pull from '{}' failed", remote_name),
                                e.to_string(),
                                (Some(remote_name.clone()), Some(branch_name.clone()), true),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Push to a remote.
    pub fn push(
        &mut self,
        remote_name: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let (branch_name, set_upstream) = match self.open_repo().and_then(|repo| {
            push_target(&repo, Some(remote_name)).map(|(_, branch, set)| (branch, set))
        }) {
            Ok(target) => target,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Push,
                    "Push could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.push_to(remote_name, &branch_name, force, set_upstream, cx)
    }

    pub fn push_to(
        &mut self,
        remote_name: &str,
        remote_branch_name: &str,
        force: bool,
        set_upstream: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        let remote_branch_name = remote_branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_remote_branch_name = remote_branch_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Push,
            if force {
                format!(
                    "Force pushing to '{}/{}'...",
                    remote_name, remote_branch_name
                )
            } else {
                format!("Pushing to '{}/{}'...", remote_name, remote_branch_name)
            },
            Some(remote_name.clone()),
            Some(remote_branch_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, details) = {
                        let repo = Repository::open(&repo_path)?;
                        let branch_name = head_branch_name(&repo)?;

                        if remote_uses_ssh(&repo, &task_remote_name)? {
                            drop(repo);
                            let mut args = vec!["push"];
                            if force {
                                args.push("--force");
                            }
                            if set_upstream {
                                args.push("--set-upstream");
                            }
                            args.push(task_remote_name.as_str());
                            let refspec = format!("HEAD:{}", task_remote_branch_name);
                            args.push(refspec.as_str());
                            let details = run_git_network_command(&repo_path, &args)?;
                            let msg = if force {
                                format!(
                                    "Force pushed '{}' to '{}/{}' using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else if set_upstream {
                                format!(
                                    "Pushed '{}' to '{}/{}' and set upstream using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else {
                                format!(
                                    "Pushed '{}' to '{}/{}' using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            };
                            (msg, details)
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let refspec = if force {
                                format!(
                                    "+refs/heads/{}:refs/heads/{}",
                                    branch_name, task_remote_branch_name
                                )
                            } else {
                                format!(
                                    "refs/heads/{}:refs/heads/{}",
                                    branch_name, task_remote_branch_name
                                )
                            };
                            let mut push_opts = make_push_options();
                            remote.push(&[&refspec], Some(&mut push_opts))?;
                            drop(remote);

                            // Update the local remote-tracking ref to match what we pushed.
                            // libgit2's push() doesn't do this automatically.
                            if let Ok(local_branch) = repo.find_branch(&branch_name, git2::BranchType::Local) {
                                if let Some(tip) = local_branch.get().target() {
                                    let remote_ref_name = format!(
                                        "refs/remotes/{}/{}",
                                        task_remote_name, task_remote_branch_name
                                    );
                                    let _ = repo.reference(
                                        &remote_ref_name,
                                        tip,
                                        true,
                                        &format!("push: update {} after push", remote_ref_name),
                                    );
                                }
                            }

                            if set_upstream {
                                let mut branch =
                                    repo.find_branch(&branch_name, git2::BranchType::Local)?;
                                branch.set_upstream(Some(&format!(
                                    "{}/{}",
                                    task_remote_name, task_remote_branch_name
                                )))?;
                            }

                            let msg = if force {
                                format!(
                                    "Force pushed '{}' to '{}/{}'",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else if set_upstream {
                                format!(
                                    "Pushed '{}' to '{}/{}' and set upstream",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else {
                                format!(
                                    "Pushed '{}' to '{}/{}'",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            };
                            (msg, None)
                        }
                    };
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((msg, details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, details, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Push,
                                msg,
                                (details.or(Some("Remote refs refreshed after push.".into())), Some(remote_name.clone()), Some(remote_branch_name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            log::error!("Push failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Push,
                                format!("Push to '{}' failed", remote_name),
                                e.to_string(),
                                (Some(remote_name.clone()), Some(remote_branch_name.clone()), true),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Merge a branch into the current HEAD.
    pub fn merge_branch(&mut self, branch_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let branch_name = branch_name.to_string();
        let task_branch_name = branch_name.clone();
        let repo_path = self.repo_path.clone();
        let current_branch = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge,
            format!("Merging '{}'...", branch_name),
            None,
            current_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let msg = {
                        let repo = Repository::open(&repo_path)?;
                        ensure_clean_worktree(&repo, "Merge")?;

                        // Find the branch reference and its annotated commit
                        let reference = repo
                            .find_branch(&task_branch_name, git2::BranchType::Local)
                            .or_else(|_| {
                                repo.find_branch(&task_branch_name, git2::BranchType::Remote)
                            })?;
                        let annotated_commit =
                            repo.reference_to_annotated_commit(reference.get())?;

                        // Perform merge analysis
                        let (analysis, _pref) = repo.merge_analysis(&[&annotated_commit])?;

                        if analysis.is_up_to_date() {
                            "Already up to date".to_string()
                        } else if analysis.is_fast_forward() {
                            let head = repo.head()?;
                            let head_branch_name =
                                head.shorthand().unwrap_or("HEAD").to_string();
                            let refname = format!("refs/heads/{}", head_branch_name);
                            let mut reference = repo.find_reference(&refname)?;
                            reference.set_target(
                                annotated_commit.id(),
                                &format!("Fast-forward merge of '{}'", task_branch_name),
                            )?;
                            repo.set_head(&refname)?;
                            repo.checkout_head(Some(
                                git2::build::CheckoutBuilder::new().force(),
                            ))?;
                            format!("Merged '{}' (fast-forward)", task_branch_name)
                        } else if analysis.is_normal() {
                            repo.merge(&[&annotated_commit], None, None)?;

                            let has_conflicts = repo.index()?.has_conflicts();
                            if has_conflicts {
                                // Conflicts detected — repo is now in Merge state.
                                // Don't error out; instead return a conflict message
                                // and let the UI show the conflict banner.
                                let conflict_count = repo
                                    .index()?
                                    .conflicts()?
                                    .count();
                                format!(
                                    "CONFLICT:{} conflict(s) detected merging '{}'. Resolve and continue.",
                                    conflict_count, task_branch_name
                                )
                            } else {
                                let sig = repo.signature()?;
                                let mut index = repo.index()?;
                                let tree_oid = index.write_tree()?;
                                let tree = repo.find_tree(tree_oid)?;
                                let head_commit = repo.head()?.peel_to_commit()?;
                                let merge_commit =
                                    repo.find_commit(annotated_commit.id())?;
                                repo.commit(
                                    Some("HEAD"),
                                    &sig,
                                    &sig,
                                    &format!(
                                        "Merge branch '{}' into {}",
                                        task_branch_name,
                                        repo.head()?
                                            .shorthand()
                                            .unwrap_or("HEAD")
                                    ),
                                    &tree,
                                    &[&head_commit, &merge_commit],
                                )?;
                                repo.cleanup_state()?;
                                format!("Merged '{}' successfully", task_branch_name)
                            }
                        } else {
                            "Merge complete".to_string()
                        }
                    }; // repo dropped here

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((msg, data))
                })
                .await;

            // Update UI on main thread
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, data)) => {
                            let is_conflict = msg.starts_with("CONFLICT:");
                            this.apply_refresh_data(data);
                            if is_conflict {
                                // Strip the CONFLICT: prefix for the user-facing message
                                let user_msg = msg.trim_start_matches("CONFLICT:").to_string();
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Merge,
                                    format!("Merge conflicts in '{}'", branch_name),
                                    user_msg,
                                    (None, current_branch.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Merge,
                                    msg,
                                    (Some("Repository state refreshed after merge.".into()), None, current_branch.clone()),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                format!("Merge of '{}' failed", branch_name),
                                e.to_string(),
                                (None, current_branch.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Save the current working tree to a stash.
    pub fn stash_save(
        &mut self,
        message: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let message = message.map(String::from);
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            "Saving stash...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    let sig = repo.signature()?;
                    repo.stash_save(&sig, message.as_deref().unwrap_or("WIP"), None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stash,
                                "Saved stash",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                "Save stash failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Pop the top stash entry.
    pub fn stash_pop(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Popping stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_pop(index, None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Popped stash #{}", index),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Pop stash #{} failed", index),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Stage a specific hunk from a file diff.
    /// Generates a patch for just that hunk and applies it to the index.
    pub fn stage_hunk(
        &mut self,
        file_path: &Path,
        hunk_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            format!(
                "Staging hunk {} in {}...",
                hunk_index + 1,
                file_path.display()
            ),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let patch_text =
                        generate_hunk_patch_for_repo(&repo, &task_file_path, hunk_index, false)?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    repo.apply(&diff, git2::ApplyLocation::Index, None)?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stage,
                                format!(
                                    "Staged hunk {} in {}",
                                    hunk_index + 1,
                                    file_path.display()
                                ),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Stage hunk failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Unstage a specific hunk from a staged file diff.
    pub fn unstage_hunk(
        &mut self,
        file_path: &Path,
        hunk_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            format!(
                "Unstaging hunk {} in {}...",
                hunk_index + 1,
                file_path.display()
            ),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let patch_text =
                        generate_hunk_patch_for_repo(&repo, &task_file_path, hunk_index, true)?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    let mut opts = git2::ApplyOptions::new();
                    repo.apply(&diff, git2::ApplyLocation::Index, Some(&mut opts))?;
                    gather_refresh_data(&repo_path)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                format!(
                                    "Unstaged hunk {} in {}",
                                    hunk_index + 1,
                                    file_path.display()
                                ),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstage hunk failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Generate a patch for a single hunk from a file's diff.
    /// Get the staged diff as a string (for AI commit message generation).
    pub fn staged_diff_text(&self) -> Result<String> {
        let repo = self.open_repo()?;
        let head_tree = repo.head()?.peel_to_tree().ok();
        let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;

        let mut output = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                _ => " ",
            };
            let content = String::from_utf8_lossy(line.content());
            output.push_str(prefix);
            output.push_str(&content);
            true
        })?;

        Ok(output)
    }

    /// Summary of staged changes for AI context.
    pub fn staged_summary(&self) -> String {
        let mut parts = Vec::new();
        for file in &self.status.staged {
            parts.push(format!(
                "{} {}",
                file.kind.short_code(),
                file.path.display()
            ));
        }
        parts.join("\n")
    }
}

fn delta_to_file_change_kind(delta: git2::Delta) -> FileChangeKind {
    match delta {
        git2::Delta::Added | git2::Delta::Untracked => FileChangeKind::Added,
        git2::Delta::Deleted => FileChangeKind::Deleted,
        git2::Delta::Modified | git2::Delta::Typechange => FileChangeKind::Modified,
        git2::Delta::Renamed => FileChangeKind::Renamed,
        git2::Delta::Copied => FileChangeKind::Modified,
        _ => FileChangeKind::Modified,
    }
}

/// Parse a git2::Diff into a FileDiff using the print API to avoid borrow issues.
fn parse_file_diff(path: &Path, diff: &git2::Diff) -> Result<FileDiff> {
    let mut file_diff = FileDiff {
        path: path.to_path_buf(),
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
        kind: FileChangeKind::Modified,
    };

    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        file_diff.kind = delta_to_file_change_kind(delta.status());
        if let Some(hunk) = hunk {
            let header = String::from_utf8_lossy(hunk.header()).to_string();
            let expected_start = hunk.new_start();
            let needs_new = file_diff.hunks.last().is_none_or(|h| {
                h.new_start != expected_start || h.header != header
            });
            if needs_new {
                file_diff.hunks.push(DiffHunk {
                    old_start: hunk.old_start(),
                    old_lines: hunk.old_lines(),
                    new_start: hunk.new_start(),
                    new_lines: hunk.new_lines(),
                    header,
                    lines: Vec::new(),
                });
            }
        }

        let content = String::from_utf8_lossy(line.content()).to_string();
        match line.origin() {
            '+' => {
                if let Some(h) = file_diff.hunks.last_mut() {
                    h.lines.push(DiffLine::Addition(content));
                }
                file_diff.additions += 1;
            }
            '-' => {
                if let Some(h) = file_diff.hunks.last_mut() {
                    h.lines.push(DiffLine::Deletion(content));
                }
                file_diff.deletions += 1;
            }
            ' ' => {
                if let Some(h) = file_diff.hunks.last_mut() {
                    h.lines.push(DiffLine::Context(content));
                }
            }
            _ => {}
        }

        true
    })?;

    Ok(file_diff)
}

pub fn compute_file_diff(repo_path: &Path, file_path: &Path, staged: bool) -> Result<FileDiff> {
    let repo = Repository::open(repo_path)?;
    let mut diff_opts = DiffOptions::new();
    diff_opts.pathspec(file_path);
    diff_opts.include_untracked(true);
    diff_opts.show_untracked_content(true);
    let diff = if staged {
        let head_tree = repo.head()?.peel_to_tree().ok();
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?
    } else {
        repo.diff_index_to_workdir(None, Some(&mut diff_opts))?
    };
    parse_file_diff(file_path, &diff)
}

pub fn compute_commit_diff(repo_path: &Path, oid: git2::Oid) -> Result<CommitDiff> {
    let repo = Repository::open(repo_path)?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let stats = diff.stats()?;
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current_hunks: Vec<DiffHunk> = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_additions: usize = 0;
    let mut current_deletions: usize = 0;
    let mut current_kind = FileChangeKind::Modified;
    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        let delta_path = delta.new_file().path().unwrap_or(Path::new("")).to_path_buf();
        if current_path.as_ref() != Some(&delta_path) {
            if let Some(prev_path) = current_path.take() {
                files.push(FileDiff { path: prev_path, hunks: std::mem::take(&mut current_hunks), additions: current_additions, deletions: current_deletions, kind: current_kind });
            }
            current_path = Some(delta_path);
            current_additions = 0;
            current_deletions = 0;
            current_kind = delta_to_file_change_kind(delta.status());
        }
        if let Some(hunk) = hunk {
            let header = String::from_utf8_lossy(hunk.header()).to_string();
            let expected_start = hunk.new_start();
            let needs_new = current_hunks.last().is_none_or(|h| h.new_start != expected_start || h.header != header);
            if needs_new {
                current_hunks.push(DiffHunk { old_start: hunk.old_start(), old_lines: hunk.old_lines(), new_start: hunk.new_start(), new_lines: hunk.new_lines(), header, lines: Vec::new() });
            }
        }
        let content = String::from_utf8_lossy(line.content()).to_string();
        match line.origin() {
            '+' => { if let Some(h) = current_hunks.last_mut() { h.lines.push(DiffLine::Addition(content)); } current_additions += 1; }
            '-' => { if let Some(h) = current_hunks.last_mut() { h.lines.push(DiffLine::Deletion(content)); } current_deletions += 1; }
            ' ' => { if let Some(h) = current_hunks.last_mut() { h.lines.push(DiffLine::Context(content)); } }
            _ => {}
        }
        true
    })?;
    if let Some(path) = current_path {
        files.push(FileDiff { path, hunks: current_hunks, additions: current_additions, deletions: current_deletions, kind: current_kind });
    }
    Ok(CommitDiff { total_additions: stats.insertions(), total_deletions: stats.deletions(), files })
}

pub fn compute_stash_diff(repo_path: &Path, index: usize) -> Result<CommitDiff> {
    let mut repo = Repository::open(repo_path)?;
    let mut stash_oids: Vec<git2::Oid> = Vec::new();
    repo.stash_foreach(|_idx, _msg, oid| { stash_oids.push(*oid); true })?;
    let stash_oid = stash_oids.get(index).copied().ok_or_else(|| anyhow::anyhow!("Stash index {} out of range", index))?;
    compute_commit_diff(repo_path, stash_oid)
}

pub fn compute_staged_diff_text(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let head_tree = repo.head()?.peel_to_tree().ok();
    let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;
    let mut text = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if let Ok(s) = std::str::from_utf8(line.content()) {
            text.push(line.origin());
            text.push_str(s);
        }
        true
    })?;
    Ok(text)
}
