use anyhow::Result;
use chrono::{TimeZone, Utc};
use git2::Repository;
use std::path::Path;

use crate::types::{CommitInfo, Signature};

use super::refresh::parse_co_authors;

/// Maximum number of commits to return for file history.
const MAX_FILE_HISTORY: usize = 100;

/// Get commit history for a specific file.
pub fn compute_file_history(
    repo_path: &Path,
    file_path: &Path,
    limit: usize,
) -> Result<Vec<CommitInfo>> {
    let repo = Repository::open(repo_path)?;

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;
    revwalk.push_head()?;

    let mut commits = Vec::new();
    let limit = limit.min(MAX_FILE_HISTORY);

    for oid_result in revwalk {
        if commits.len() >= limit {
            break;
        }

        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        // Check if this commit touches the file
        let tree = commit.tree()?;
        let touches_file = if let Ok(parent) = commit.parent(0) {
            let parent_tree = parent.tree()?;
            let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)?;

            // Check if the file is in the diff
            let mut found = false;
            diff.foreach(
                &mut |delta, _| {
                    if let Some(new_path) = delta.new_file().path() {
                        if new_path == file_path {
                            found = true;
                            return false; // Stop iteration
                        }
                    }
                    if let Some(old_path) = delta.old_file().path() {
                        if old_path == file_path {
                            found = true;
                            return false; // Stop iteration
                        }
                    }
                    true
                },
                None,
                None,
                None,
            )?;
            found
        } else {
            // First commit - check if file exists in tree
            tree.get_path(file_path).is_ok()
        };

        if !touches_file {
            continue;
        }

        let author = commit.author();
        let committer = commit.committer();
        let time = Utc.timestamp_opt(commit.time().seconds(), 0).single();

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
            refs: Vec::new(), // No refs needed for file history
        });
    }

    Ok(commits)
}

use gpui::{AsyncApp, Context, Task, WeakEntity};

use super::GitProject;

impl GitProject {
    /// Get commit history for a specific file.
    pub fn file_history(&self, file_path: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
        compute_file_history(self.repo_path(), file_path, limit)
    }

    /// Get commit history for a file asynchronously.
    pub fn file_history_async(
        &self,
        file_path: &Path,
        limit: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<CommitInfo>>> {
        let repo_path = self.repo_path().to_path_buf();
        let file_path = file_path.to_path_buf();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .spawn(async move { compute_file_history(&repo_path, &file_path, limit) })
                .await
        })
    }
}
