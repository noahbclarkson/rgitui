use anyhow::{Context as _, Result};
use chrono::{TimeZone, Utc};
use git2::{Repository, StatusOptions};
use std::path::{Path, PathBuf};

use crate::types::*;

use super::diff::batch_diff_stats;
use super::git_command;
use super::GitProject;
use super::GitProjectEvent;
use super::RefreshData;

/// Remove valid Co-Authored-By trailer lines from a commit message.
/// Only strips lines that have a parseable `Name <email>` format.
pub(super) fn clean_co_author_lines(message: &str) -> String {
    let prefix = "co-authored-by:";
    let cleaned_lines: Vec<&str> = message
        .lines()
        .filter(|line| {
            let lower = line.trim().to_ascii_lowercase();
            if !lower.starts_with(prefix) {
                return true;
            }
            let rest = &line.trim()[prefix.len()..].trim();
            let has_email = rest.contains('<') && rest.contains('>');
            if !has_email {
                return true;
            }
            if let Some(start) = rest.find('<') {
                if let Some(end) = rest.find('>') {
                    let name = rest[..start].trim();
                    let email = rest[start + 1..end].trim();
                    return name.is_empty() || email.is_empty();
                }
            }
            true
        })
        .collect();
    let cleaned = cleaned_lines.join("\n");
    cleaned.trim_end().to_string()
}

/// Extract Co-Authored-By signatures from a commit message.
pub fn extract_co_authors(message: &str) -> Vec<Signature> {
    let prefix = "co-authored-by:";
    let mut co_authors = Vec::new();
    for line in message.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with(prefix) {
            let rest = &trimmed[prefix.len()..].trim();
            if let Some(email_start) = rest.find('<') {
                if let Some(email_end) = rest.find('>') {
                    let name = rest[..email_start].trim().to_string();
                    let email = rest[email_start + 1..email_end].trim().to_string();
                    if !name.is_empty() && !email.is_empty() {
                        co_authors.push(Signature { name, email });
                    }
                }
            }
        }
    }
    co_authors
}

/// Combined: clean message and extract co-authors in one pass.
#[cfg(test)]
fn parse_co_authors(message: &str) -> (String, Vec<Signature>) {
    let co_authors = extract_co_authors(message);
    let cleaned = clean_co_author_lines(message);
    (cleaned, co_authors)
}

/// Gather information about all worktrees attached to this repository.
fn gather_worktrees(repo: &Repository) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();

    // Current (main) worktree
    let workdir = repo.workdir().unwrap_or_else(|| repo.path());
    let current_name = workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("main")
        .to_string();
    worktrees.push(WorktreeInfo {
        name: current_name,
        path: workdir.to_path_buf(),
        is_locked: false,
        is_current: true,
        branch: repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from)),
        head_oid: repo.head().ok().and_then(|h| h.target()),
        status: None,
    });

    // List all worktrees from the main repo
    if let Ok(names) = repo.worktrees() {
        for name in names.iter().flatten() {
            if name.is_empty() {
                continue;
            }
            if let Ok(wt) = repo.find_worktree(name) {
                let path = wt.path().to_path_buf();
                let is_locked = match wt.is_locked() {
                    Ok(git2::WorktreeLockStatus::Locked(_)) => true,
                    Ok(git2::WorktreeLockStatus::Unlocked) | Err(_) => false,
                };

                // Try to open the worktree repo to get branch/HEAD info
                let wt_repo = Repository::open(&path).ok();
                let branch = wt_repo
                    .as_ref()
                    .and_then(|r| r.head().ok().and_then(|h| h.shorthand().map(String::from)));
                let head_oid = wt_repo
                    .as_ref()
                    .and_then(|r| r.head().ok().and_then(|h| h.target()));

                worktrees.push(WorktreeInfo {
                    name: name.to_string(),
                    path,
                    is_locked,
                    is_current: false,
                    branch,
                    head_oid,
                    status: None,
                });
            }
        }
    }

    // Sort: current worktree first, then alphabetically by name
    worktrees.sort_by(|a, b| {
        if a.is_current != b.is_current {
            return b.is_current.cmp(&a.is_current);
        }
        a.name.cmp(&b.name)
    });

    worktrees
}

/// Gather all refresh data from a repository at the given path.
/// This is a standalone function (no `&self`) so it can run on a background thread.
fn compute_working_tree_status(repo_path: &Path) -> Result<WorkingTreeStatus> {
    let repo = Repository::open(repo_path)?;
    let mut wt_status = WorkingTreeStatus::default();

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_unmodified(false);

    let statuses = repo.statuses(Some(&mut opts))?;
    let (staged_stats, unstaged_stats) = std::thread::scope(|s| {
        let staged_handle = s.spawn(|| {
            let repo = Repository::open(repo_path).ok();
            repo.as_ref()
                .map(|r| batch_diff_stats(r, true))
                .unwrap_or_default()
        });
        let unstaged_handle = s.spawn(|| {
            let repo = Repository::open(repo_path).ok();
            repo.as_ref()
                .map(|r| batch_diff_stats(r, false))
                .unwrap_or_default()
        });
        (
            staged_handle.join().unwrap_or_default(),
            unstaged_handle.join().unwrap_or_default(),
        )
    });

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

    Ok(wt_status)
}

pub fn gather_refresh_data(repo_path: &Path, commit_limit: usize) -> Result<RefreshData> {
    log::debug!("gather_refresh_data: repo={}", repo_path.display());
    gather_refresh_data_internal(repo_path, true, commit_limit)
}

/// Gather refresh data without computing ahead/behind for every branch.
///
/// Use this for filesystem watcher events where only file status needs updating.
/// Ahead/behind values will be (0, 0) — they'll be recomputed on the next
/// full refresh from a git operation (fetch/push/pull) or explicit user refresh.
pub fn gather_refresh_data_lightweight(
    repo_path: &Path,
    commit_limit: usize,
) -> Result<RefreshData> {
    log::debug!(
        "gather_refresh_data_lightweight: repo={}",
        repo_path.display()
    );
    gather_refresh_data_internal(repo_path, false, commit_limit)
}

fn gather_refresh_data_internal(
    repo_path: &Path,
    compute_ahead_behind: bool,
    commit_limit: usize,
) -> Result<RefreshData> {
    let refresh_timer = std::time::Instant::now();
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {}", repo_path.display()))?;

    // Head
    let head_branch = repo
        .head()
        .ok()
        .and_then(|r| r.shorthand().map(String::from));
    let head_detached = repo.head_detached().unwrap_or(false);
    let repo_state = RepoState::from_git2(repo.state());

    // Current user email (for "My Branches" / "My Commits" filtering)
    let current_user_email = repo
        .config()
        .ok()
        .and_then(|cfg| cfg.get_string("user.email").ok());

    // Branches
    // Branches — two-pass approach:
    // Pass 1: collect basic info + last_commit_time
    // Then find main branch tip OID
    // Pass 2: compute is_merged_into_main using git ancestry check
    let mut branches: Vec<BranchInfo> = Vec::new();
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

            let (ahead, behind) = if compute_ahead_behind {
                if let (Some(local_oid), Ok(upstream_ref)) = (tip_oid, branch.upstream()) {
                    if let Some(remote_oid) = upstream_ref.get().target() {
                        repo.graph_ahead_behind(local_oid, remote_oid)
                            .unwrap_or((0, 0))
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

            let last_commit_time =
                tip_oid.and_then(|oid| repo.find_commit(oid).ok().map(|c| c.time().seconds()));

            let author_email = tip_oid.and_then(|oid| {
                repo.find_commit(oid)
                    .ok()
                    .and_then(|c| c.author().email().map(String::from))
            });

            branches.push(BranchInfo {
                name,
                is_head,
                is_remote,
                upstream,
                ahead,
                behind,
                tip_oid,
                author_email,
                last_commit_time,
                is_merged_into_main: None,
            });
        }

        branches.sort_by(|a, b| {
            b.is_head
                .cmp(&a.is_head)
                .then(a.is_remote.cmp(&b.is_remote))
                .then(a.name.cmp(&b.name))
        });

        // Find the main branch tip OID.
        // Priority: local main > local master > remote origin/main > remote origin/master
        let main_tip = branches
            .iter()
            .find(|b| !b.is_remote && b.name == "main")
            .and_then(|b| b.tip_oid)
            .or_else(|| {
                branches
                    .iter()
                    .find(|b| !b.is_remote && b.name == "master")
                    .and_then(|b| b.tip_oid)
            })
            .or_else(|| {
                branches
                    .iter()
                    .find(|b| b.is_remote && b.name == "origin/main")
                    .and_then(|b| b.tip_oid)
            })
            .or_else(|| {
                branches
                    .iter()
                    .find(|b| b.is_remote && b.name == "origin/master")
                    .and_then(|b| b.tip_oid)
            });

        // Pass 2: compute is_merged_into_main for local branches
        if let Some(main_tip_oid) = main_tip {
            for branch in branches.iter_mut() {
                if branch.is_remote {
                    continue;
                }
                if branch.is_head {
                    // The HEAD branch: merged if it points to the same commit as main
                    branch.is_merged_into_main = Some(branch.tip_oid == main_tip);
                    continue;
                }
                // A branch is merged into main if the branch tip is an ancestor of main_tip.
                // Use merge_base: if merge_base(branch_tip, main_tip) == branch_tip,
                // then branch_tip is an ancestor of main_tip (branch was merged/fast-forwarded).
                let is_merged = branch.tip_oid.is_some_and(|tip_oid| {
                    repo.merge_base(tip_oid, main_tip_oid)
                        .map(|mb| mb == tip_oid)
                        .unwrap_or(false)
                });
                branch.is_merged_into_main = Some(is_merged);
            }
        }
    }

    // Tags
    let mut tags = Vec::new();
    if let Err(e) = repo.tag_foreach(|oid, name_bytes| {
        if let Ok(name) = std::str::from_utf8(name_bytes) {
            let name = name.strip_prefix("refs/tags/").unwrap_or(name).to_string();
            tags.push(TagInfo {
                name,
                oid,
                message: None,
            });
        }
        true
    }) {
        log::warn!("Failed to enumerate repository tags: {}", e);
    }
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

    // Run status, stashes, worktrees in parallel.
    // Status and stashes open their own repos; worktrees and revwalk use &repo.
    let (status, stashes, worktrees) = std::thread::scope(|s| {
        let status_handle = s.spawn(|| compute_working_tree_status(repo_path));

        let stash_handle = s.spawn(|| {
            let mut stashes = Vec::new();
            if let Ok(mut repo_mut) = Repository::open(repo_path) {
                let _ = repo_mut.stash_foreach(|stash_index, message, oid| {
                    stashes.push(StashEntry {
                        index: stash_index,
                        message: message.to_string(),
                        oid: *oid,
                    });
                    true
                });
            }
            stashes
        });

        let mut worktrees = gather_worktrees(&repo);
        let mut worktree_status_handles = Vec::new();
        for (idx, worktree) in worktrees.iter().enumerate() {
            if worktree.is_current {
                continue;
            }
            let worktree_path = worktree.path.clone();
            worktree_status_handles.push((
                idx,
                s.spawn(move || compute_working_tree_status(&worktree_path)),
            ));
        }

        let status = status_handle.join().unwrap().unwrap_or_default();
        if let Some(current_worktree) = worktrees.iter_mut().find(|wt| wt.is_current) {
            current_worktree.status = Some(status.clone());
        }

        for (idx, handle) in worktree_status_handles {
            worktrees[idx].status = handle
                .join()
                .unwrap_or_else(|_| Ok(Default::default()))
                .ok();
        }

        let stashes = stash_handle.join().unwrap_or_default();
        (status, stashes, worktrees)
    });

    // Recent commits — use git log subprocess for commit-graph acceleration.
    // libgit2's revwalk doesn't use .git/objects/info/commit-graph, making it
    // orders of magnitude slower on large repos like the Linux kernel.
    let (recent_commits, has_more_commits) = {
        let t_log = std::time::Instant::now();
        let limit = commit_limit;

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

        // Record separator that won't appear in commit messages
        const RS: &str = "\x1e";
        const GS: &str = "\x1d";
        // Format: oid, short_id, author_name, author_email, committer_name, committer_email, timestamp, parent_oids, summary, body
        let format = format!(
            "{}%H{}%h{}%an{}%ae{}%cn{}%ce{}%ct{}%P{}%s{}%b{}",
            RS, GS, GS, GS, GS, GS, GS, GS, GS, GS, GS
        );

        let output = git_command()
            .current_dir(repo_path)
            .args([
                "log",
                "--all",
                &format!("--format={}", format),
                &format!("-{}", limit + 1),
            ])
            .output()
            .with_context(|| "Failed to run git log")?;

        if !output.status.success() {
            anyhow::bail!(
                "git log failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        let mut has_more = false;

        for record in stdout.split(RS) {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            if commits.len() >= limit {
                has_more = true;
                break;
            }

            let fields: Vec<&str> = record.splitn(11, GS).collect();
            if fields.len() < 10 {
                continue;
            }

            let oid = match git2::Oid::from_str(fields[0]) {
                Ok(o) => o,
                Err(_) => continue,
            };
            let short_id = fields[1].to_string();
            let author_name = fields[2].to_string();
            let author_email = fields[3].to_string();
            let committer_name = fields[4].to_string();
            let committer_email = fields[5].to_string();
            let timestamp: i64 = fields[6].parse().unwrap_or(0);
            let parent_oids: Vec<git2::Oid> = fields[7]
                .split_whitespace()
                .filter_map(|s| git2::Oid::from_str(s).ok())
                .collect();
            let summary = fields[8].to_string();
            let body = if fields.len() > 9 {
                fields[9].trim()
            } else {
                ""
            };
            let message = if body.is_empty() {
                summary.clone()
            } else {
                format!("{}\n\n{}", summary, clean_co_author_lines(body))
            };

            let time = Utc.timestamp_opt(timestamp, 0).single();
            let refs = ref_map.remove(&oid).unwrap_or_default();

            commits.push(CommitInfo {
                oid,
                short_id,
                summary,
                message,
                author: Signature {
                    name: author_name,
                    email: author_email,
                },
                committer: Signature {
                    name: committer_name,
                    email: committer_email,
                },
                co_authors: Vec::new(),
                time: time.unwrap_or_else(Utc::now),
                parent_oids,
                refs,
                is_signed: false,
            });
        }

        log::debug!(
            "git log completed in {:?}: {} commits",
            t_log.elapsed(),
            commits.len()
        );
        (commits, has_more)
    };

    log::info!(
        "gather_refresh_data_internal complete in {:?}: {} commits, {} branches, staged={} unstaged={}",
        refresh_timer.elapsed(),
        recent_commits.len(),
        branches.len(),
        status.staged.len(),
        status.unstaged.len()
    );
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
        worktrees,
        default_branch: None,
        current_user_email,
    })
}

/// Enrich a commit with is_signed and co_authors (deferred from the revwalk).
pub fn enrich_commit_info(repo_path: &Path, oid: git2::Oid) -> Result<(bool, Vec<Signature>)> {
    let repo = Repository::open(repo_path)?;
    let commit = repo.find_commit(oid)?;
    let is_signed = commit.header_field_bytes("gpgsig").is_ok();
    let raw_message = commit.message().unwrap_or("");
    let co_authors = extract_co_authors(raw_message);
    Ok((is_signed, co_authors))
}

use gpui::{AsyncApp, Context, Task, WeakEntity};
use std::sync::Arc;

const FIRST_BATCH_SIZE: usize = 100;

impl GitProject {
    /// Refresh all state asynchronously on a background thread.
    /// Uses two-phase loading: the first batch of commits loads quickly so the
    /// UI appears populated during the splash animation, then the remainder
    /// loads in the background.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let commit_limit = self.commit_limit;
        let first_batch = FIRST_BATCH_SIZE.min(commit_limit);
        let t = std::time::Instant::now();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // Phase 1: lightweight refresh (skip ahead/behind) with a small commit batch
            let repo_path_p1 = repo_path.clone();
            let data = cx
                .background_executor()
                .spawn(async move { gather_refresh_data_lightweight(&repo_path_p1, first_batch) })
                .await?;

            let needs_more = data.has_more_commits && first_batch < commit_limit;
            let branch_tips: Vec<(git2::Oid, bool, String)> = data
                .branches
                .iter()
                .filter_map(|b| b.tip_oid.map(|oid| (oid, b.is_remote, b.name.clone())))
                .collect();
            let tag_tips: Vec<(git2::Oid, String)> =
                data.tags.iter().map(|t| (t.oid, t.name.clone())).collect();

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.apply_refresh_data(data);
                    log::info!(
                        "refresh phase 1 applied in {:?}: {} commits",
                        t.elapsed(),
                        this.recent_commits.len()
                    );
                    cx.emit(GitProjectEvent::StatusChanged);
                    // Fire ahead/behind computation in the background (deferred from lightweight refresh)
                    this.refresh_ahead_behind(cx);
                    cx.notify();
                    Ok::<(), anyhow::Error>(())
                })
            })??;

            // Phase 2: load remaining commits
            if needs_more {
                let remaining = commit_limit - first_batch;
                let repo_path_p2 = repo_path.clone();
                let (more_commits, has_more) = cx
                    .background_executor()
                    .spawn(async move {
                        load_more_commits_from_repo(
                            &repo_path_p2,
                            first_batch,
                            remaining,
                            &branch_tips,
                            &tag_tips,
                            None,
                        )
                    })
                    .await?;

                cx.update(|cx| {
                    this.update(cx, |this, cx| {
                        let existing_oids: std::collections::HashSet<git2::Oid> =
                            this.recent_commits.iter().map(|c| c.oid).collect();
                        let mut combined: Vec<CommitInfo> = (*this.recent_commits).clone();
                        for commit in more_commits {
                            if !existing_oids.contains(&commit.oid) {
                                combined.push(commit);
                            }
                        }
                        this.commit_offset = combined.len();
                        this.has_more_commits = has_more;
                        this.recent_commits = Arc::new(combined);
                        log::info!(
                            "refresh phase 2 applied in {:?}: {} commits total",
                            t.elapsed(),
                            this.recent_commits.len()
                        );
                        cx.emit(GitProjectEvent::StatusChanged);
                        cx.notify();
                        Ok(())
                    })
                })?
            } else {
                Ok(())
            }
        })
    }
}

/// Load the next batch of commits starting at `skip`, without re-fetching branches/status/etc.
/// Returns `(new_commits, has_more)`.
pub(super) fn load_more_commits_from_repo(
    repo_path: &Path,
    skip: usize,
    limit: usize,
    branch_tips: &[(git2::Oid, bool, String)],
    tag_tips: &[(git2::Oid, String)],
    author_filter: Option<&str>,
) -> Result<(Vec<CommitInfo>, bool)> {
    // Build ref-label map from the caller-supplied tips.
    let mut ref_map = std::collections::HashMap::<git2::Oid, Vec<RefLabel>>::new();
    if let Ok(repo) = Repository::open(repo_path) {
        if let Ok(head) = repo.head() {
            if let Some(oid) = head.target() {
                ref_map.entry(oid).or_default().push(RefLabel::Head);
            }
        }
    }
    for (oid, is_remote, name) in branch_tips {
        let label = if *is_remote {
            RefLabel::RemoteBranch(name.clone())
        } else {
            RefLabel::LocalBranch(name.clone())
        };
        ref_map.entry(*oid).or_default().push(label);
    }
    for (oid, name) in tag_tips {
        ref_map
            .entry(*oid)
            .or_default()
            .push(RefLabel::Tag(name.clone()));
    }

    const RS: &str = "\x1e";
    const GS: &str = "\x1d";
    let format = format!(
        "{}%H{}%h{}%an{}%ae{}%cn{}%ce{}%ct{}%P{}%s{}%b{}",
        RS, GS, GS, GS, GS, GS, GS, GS, GS, GS, GS
    );

    let mut cmd = git_command();
    cmd.current_dir(repo_path)
        .args(["log", "--all", &format!("--format={}", format)]);
    if let Some(author) = author_filter {
        cmd.arg(format!("--author={}", author));
    }
    cmd.args([&format!("--skip={}", skip), &format!("-{}", limit + 1)]);
    let output = cmd.output().with_context(|| "Failed to run git log")?;

    if !output.status.success() {
        anyhow::bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    let mut has_more = false;

    for record in stdout.split(RS) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        if commits.len() >= limit {
            has_more = true;
            break;
        }

        let fields: Vec<&str> = record.splitn(11, GS).collect();
        if fields.len() < 10 {
            continue;
        }

        let oid = match git2::Oid::from_str(fields[0]) {
            Ok(o) => o,
            Err(_) => continue,
        };
        let short_id = fields[1].to_string();
        let author_name = fields[2].to_string();
        let author_email = fields[3].to_string();
        let committer_name = fields[4].to_string();
        let committer_email = fields[5].to_string();
        let timestamp: i64 = fields[6].parse().unwrap_or(0);
        let parent_oids: Vec<git2::Oid> = fields[7]
            .split_whitespace()
            .filter_map(|s| git2::Oid::from_str(s).ok())
            .collect();
        let summary = fields[8].to_string();
        let body = if fields.len() > 9 {
            fields[9].trim()
        } else {
            ""
        };
        let message = if body.is_empty() {
            summary.clone()
        } else {
            format!("{}\n\n{}", summary, clean_co_author_lines(body))
        };

        let time = Utc.timestamp_opt(timestamp, 0).single();
        let refs = ref_map.remove(&oid).unwrap_or_default();

        commits.push(CommitInfo {
            oid,
            short_id,
            summary,
            message,
            author: Signature {
                name: author_name,
                email: author_email,
            },
            committer: Signature {
                name: committer_name,
                email: committer_email,
            },
            co_authors: Vec::new(),
            time: time.unwrap_or_else(Utc::now),
            parent_oids,
            refs,
            is_signed: false,
        });
    }

    Ok((commits, has_more))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_co_authors ───────────────────────────────────────────

    #[test]
    fn parse_single_co_author() {
        let message = "Fix a bug\n\nCo-Authored-By: Alice Smith <alice@example.com>";
        let (cleaned, authors) = parse_co_authors(message);
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "Alice Smith");
        assert_eq!(authors[0].email, "alice@example.com");
        assert_eq!(cleaned, "Fix a bug");
    }

    #[test]
    fn parse_multiple_co_authors() {
        let message = "Refactor module\n\n\
            Co-Authored-By: Alice <alice@example.com>\n\
            Co-Authored-By: Bob Jones <bob@example.com>";
        let (cleaned, authors) = parse_co_authors(message);
        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "Alice");
        assert_eq!(authors[1].name, "Bob Jones");
        assert_eq!(cleaned, "Refactor module");
    }

    #[test]
    fn parse_co_author_case_insensitive() {
        let message = "Fix\n\nco-authored-by: Alice <alice@example.com>";
        let (_, authors) = parse_co_authors(message);
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "Alice");
    }

    #[test]
    fn parse_no_co_authors() {
        let message = "Just a normal commit message\n\nWith a body.";
        let (cleaned, authors) = parse_co_authors(message);
        assert!(authors.is_empty());
        assert_eq!(cleaned, "Just a normal commit message\n\nWith a body.");
    }

    #[test]
    fn parse_co_author_missing_email() {
        let message = "Fix\n\nCo-Authored-By: Alice";
        let (cleaned, authors) = parse_co_authors(message);
        assert!(authors.is_empty());
        assert!(cleaned.contains("Co-Authored-By: Alice"));
    }

    #[test]
    fn parse_co_author_empty_name() {
        let message = "Fix\n\nCo-Authored-By: <alice@example.com>";
        let (_, authors) = parse_co_authors(message);
        assert!(authors.is_empty());
    }

    #[test]
    fn parse_co_author_empty_email() {
        let message = "Fix\n\nCo-Authored-By: Alice <>";
        let (_, authors) = parse_co_authors(message);
        assert!(authors.is_empty());
    }

    #[test]
    fn parse_co_author_empty_message() {
        let (cleaned, authors) = parse_co_authors("");
        assert!(authors.is_empty());
        assert_eq!(cleaned, "");
    }

    #[test]
    fn parse_co_author_preserves_body_lines() {
        let message = "Title\n\nBody line 1\nBody line 2\n\nCo-Authored-By: A <a@b.com>";
        let (cleaned, authors) = parse_co_authors(message);
        assert_eq!(authors.len(), 1);
        assert!(cleaned.contains("Body line 1"));
        assert!(cleaned.contains("Body line 2"));
    }
}

// ── load_more_commits_from_repo ──────────────────────────────────

#[cfg(test)]
mod load_more_tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a bare temporary git repo with `n` commits on main.
    fn make_repo_with_commits(n: usize) -> (TempDir, std::path::PathBuf, git2::Oid) {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Configure minimal identity so commits succeed.
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
        drop(config);

        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };

        let mut last_oid = git2::Oid::zero();
        for i in 0..n {
            let tree = repo.find_tree(tree_oid).unwrap();
            let parents: Vec<git2::Commit> = if i == 0 {
                vec![]
            } else {
                vec![repo.find_commit(last_oid).unwrap()]
            };
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            last_oid = repo
                .commit(
                    Some("refs/heads/main"),
                    &sig,
                    &sig,
                    &format!("commit {}", i),
                    &tree,
                    &parent_refs,
                )
                .unwrap();
        }
        let path = dir.path().to_path_buf();
        (dir, path, last_oid)
    }

    #[test]
    fn load_more_returns_next_page() {
        let (_dir, path, tip) = make_repo_with_commits(5);
        // Load page 2 (skip first 3, take up to 2)
        let branch_tips = vec![(tip, false, "main".to_string())];
        let (commits, has_more) =
            load_more_commits_from_repo(&path, 3, 2, &branch_tips, &[], None).unwrap();
        // 5 commits total, skip 3 → 2 remaining, no more after that
        assert_eq!(commits.len(), 2);
        assert!(!has_more);
    }

    #[test]
    fn load_more_detects_has_more() {
        let (_dir, path, tip) = make_repo_with_commits(5);
        let branch_tips = vec![(tip, false, "main".to_string())];
        // skip 0, limit 3 → should have more
        let (commits, has_more) =
            load_more_commits_from_repo(&path, 0, 3, &branch_tips, &[], None).unwrap();
        assert_eq!(commits.len(), 3);
        assert!(has_more);
    }

    #[test]
    fn load_more_empty_past_end() {
        let (_dir, path, tip) = make_repo_with_commits(3);
        let branch_tips = vec![(tip, false, "main".to_string())];
        // skip past all commits
        let (commits, has_more) =
            load_more_commits_from_repo(&path, 10, 5, &branch_tips, &[], None).unwrap();
        assert!(commits.is_empty());
        assert!(!has_more);
    }
}

// ── is_merged_into_main via graph_descendant_of ──────────────────
//
// graph_descendant_of(a, b) in libgit2 means:
//   "is b reachable from a by following parent pointers?"
//   i.e. "is a an ancestor of b?"
//
// To check "branch is merged into main":
//   We need: "is branch_tip an ancestor of main_tip?"
//   I.e., can we reach main_tip by following parent pointers from branch_tip?
//   Answer: graph_descendant_of(branch_tip, main_tip)

#[cfg(test)]
mod is_merged_tests {
    use super::*;
    use tempfile::TempDir;

    fn configure_signature(repo: &Repository) {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
    }

    fn empty_tree_oid(repo: &Repository) -> git2::Oid {
        repo.index().unwrap().write_tree().unwrap()
    }

    fn commit(
        repo: &Repository,
        refname: &str,
        message: &str,
        parent: Option<git2::Oid>,
    ) -> git2::Oid {
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = empty_tree_oid(repo);
        let tree = repo.find_tree(tree_oid).unwrap();
        let mut parents: Vec<git2::Commit> = Vec::new();
        if let Some(p) = parent {
            parents.push(repo.find_commit(p).unwrap());
        }
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some(refname), &sig, &sig, message, &tree, &parent_refs)
            .unwrap()
    }

    fn merge(repo: &Repository, into_ref: &str, from_ref: &str, message: &str) -> git2::Oid {
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = empty_tree_oid(repo);
        let tree = repo.find_tree(tree_oid).unwrap();
        let main_commit = repo
            .revparse_single(into_ref)
            .unwrap()
            .peel_to_commit()
            .unwrap();
        let branch_commit = repo
            .revparse_single(from_ref)
            .unwrap()
            .peel_to_commit()
            .unwrap();
        repo.commit(
            Some(into_ref),
            &sig,
            &sig,
            message,
            &tree,
            &[&main_commit, &branch_commit],
        )
        .unwrap()
    }

    /// Repo structure:
    ///   A (main, branch)
    /// After: A --- M (main, merged)
    ///                 \
    ///                  B (branch)
    /// merge_base(branch_tip=B, main_tip=M) should equal B (B is ancestor of M).
    #[test]
    fn branch_merged_into_main_returns_true() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        configure_signature(&repo);

        let a = commit(&repo, "refs/heads/main", "A", None);
        let b = commit(&repo, "refs/heads/branch", "B", Some(a));
        let m = merge(&repo, "refs/heads/main", "refs/heads/branch", "Merge");

        // Verify B is a parent of M
        let m_commit = repo.find_commit(m).unwrap();
        let m_parents: Vec<_> = m_commit.parent_ids().collect();
        assert!(m_parents.contains(&b), "B should be a parent of M");

        // is_merged = merge_base(branch_tip, main_tip) == branch_tip
        let mb = repo.merge_base(b, m).unwrap();
        assert_eq!(mb, b, "merge_base(branch=B, main=M) should equal B");
    }

    /// Repo structure:
    ///   A --- C (main, diverged)
    ///    \
    ///     B (branch, never merged)
    /// merge_base(branch_tip=B, main_tip=C) should NOT equal B.
    #[test]
    fn branch_not_merged_returns_false() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        configure_signature(&repo);

        let a = commit(&repo, "refs/heads/main", "A", None);
        let b = commit(&repo, "refs/heads/branch", "B", Some(a));
        let c = commit(&repo, "refs/heads/main", "C", Some(a));

        let mb = repo.merge_base(b, c).unwrap();
        assert_ne!(
            mb, b,
            "merge_base(branch=B, main=C) should NOT equal B for diverged branches"
        );
    }

    /// Repo structure (fast-forward):
    ///   A (both main and branch)
    /// Then branch advances:
    ///   A --- B (branch)
    /// (main still at A)
    /// After fast-forward of main to branch tip B:
    ///   A --- B (main, branch — same commit)
    /// merge_base(branch_tip=B, main_tip=B) == B (same commit).
    #[test]
    fn fast_forward_merged_returns_true() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        configure_signature(&repo);

        let a = commit(&repo, "refs/heads/main", "A", None);
        // Create branch at A (same as main)
        repo.branch("branch", &repo.find_commit(a).unwrap(), false)
            .unwrap();
        // Advance branch to B
        let b = commit(&repo, "refs/heads/branch", "B", Some(a));

        // Fast-forward: move main ref to branch tip
        let mut main_ref = repo.find_reference("refs/heads/main").unwrap();
        main_ref
            .set_target(b, "fast-forward main to branch")
            .unwrap();

        let main_tip = repo
            .revparse_single("refs/heads/main")
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id();
        // Both point to B now
        assert_eq!(main_tip, b, "main should now point to B");

        // is_merged = merge_base(branch_tip, main_tip) == branch_tip
        // When both point to the same commit, merge_base equals that commit
        let mb = repo.merge_base(b, main_tip).unwrap();
        assert_eq!(mb, b, "merge_base of same commit should equal that commit");
    }

    /// Repo structure:
    ///   A --- B --- C (main advanced after branch split)
    ///    \
    ///     D (branch, still at D)
    /// This is the key bug scenario: main advanced past the branch commit,
    /// but the branch was never merged into main.
    /// merge_base(branch_tip=D, main_tip=C) should NOT equal D.
    #[test]
    fn main_advanced_after_branch_returns_false() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        configure_signature(&repo);

        let a = commit(&repo, "refs/heads/main", "A", None);
        let d = commit(&repo, "refs/heads/branch", "D", Some(a));
        let _b = commit(&repo, "refs/heads/main", "B", Some(a));
        let c = commit(
            &repo,
            "refs/heads/main",
            "C",
            Some(
                repo.revparse_single("refs/heads/main")
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .id(),
            ),
        );

        // branch still at D, main at C. D is NOT an ancestor of C.
        let mb = repo.merge_base(d, c).unwrap();
        assert_ne!(
            mb, d,
            "merge_base(branch=D, main=C) should NOT equal D when main advanced independently"
        );
    }
}
