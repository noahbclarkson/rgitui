use anyhow::{Context as _, Result};
use chrono::{TimeZone, Utc};
use git2::{Repository, StatusOptions};
use std::path::{Path, PathBuf};

use crate::types::*;

use super::diff::batch_diff_stats;
use super::GitProject;
use super::GitProjectEvent;
use super::RefreshData;
use super::DEFAULT_COMMIT_LIMIT;

pub(super) fn parse_co_authors(message: &str) -> (String, Vec<Signature>) {
    let mut co_authors = Vec::new();
    let mut cleaned_lines = Vec::new();
    let prefix = "co-authored-by:";

    for line in message.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.strip_prefix(prefix).is_some() {
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
        }
        cleaned_lines.push(line);
    }

    let cleaned = cleaned_lines.join("\n");
    let cleaned = cleaned.trim_end().to_string();

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

    // Worktrees
    let worktrees = gather_worktrees(&repo);

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
        let limit = DEFAULT_COMMIT_LIMIT;
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
                worktrees,
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
                is_signed: commit.header_field_bytes("gpgsig").is_ok(),
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
        worktrees,
    })
}

use gpui::{AsyncApp, Context, Task, WeakEntity};

impl GitProject {
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
}

/// Load the next batch of commits starting at `skip`, without re-fetching branches/status/etc.
/// Returns `(new_commits, has_more)`.
pub(super) fn load_more_commits_from_repo(
    repo_path: &Path,
    skip: usize,
    limit: usize,
    branch_tips: &[(git2::Oid, bool, String)],
    tag_tips: &[(git2::Oid, String)],
) -> Result<(Vec<CommitInfo>, bool)> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {}", repo_path.display()))?;

    // Build ref-label map from the caller-supplied tips (avoids re-enumerating branches).
    let mut ref_map = std::collections::HashMap::<git2::Oid, Vec<RefLabel>>::new();
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.target() {
            ref_map.entry(oid).or_default().push(RefLabel::Head);
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

    let mut revwalk = repo.revwalk()?;
    let has_head = revwalk.push_head().is_ok();
    for (oid, _, _) in branch_tips {
        revwalk.push(*oid).ok();
    }
    if !has_head && branch_tips.is_empty() {
        return Ok((Vec::new(), false));
    }
    revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;

    let mut commits = Vec::new();
    let mut has_more = false;
    for (count, oid_result) in revwalk.enumerate() {
        let oid = oid_result?;
        if count < skip {
            continue;
        }
        if count >= skip + limit {
            has_more = true;
            break;
        }
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
            is_signed: commit.header_field_bytes("gpgsig").is_ok(),
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
            load_more_commits_from_repo(&path, 3, 2, &branch_tips, &[]).unwrap();
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
            load_more_commits_from_repo(&path, 0, 3, &branch_tips, &[]).unwrap();
        assert_eq!(commits.len(), 3);
        assert!(has_more);
    }

    #[test]
    fn load_more_empty_past_end() {
        let (_dir, path, tip) = make_repo_with_commits(3);
        let branch_tips = vec![(tip, false, "main".to_string())];
        // skip past all commits
        let (commits, has_more) =
            load_more_commits_from_repo(&path, 10, 5, &branch_tips, &[]).unwrap();
        assert!(commits.is_empty());
        assert!(!has_more);
    }
}
