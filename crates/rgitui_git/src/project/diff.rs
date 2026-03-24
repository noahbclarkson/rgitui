use anyhow::Result;
use git2::{DiffOptions, Repository};
use std::path::{Path, PathBuf};

use crate::types::*;

use super::gather_refresh_data;
use super::RefreshData;

/// Compute line-level diff stats (additions/deletions) for a single file.
/// For staged files, diffs HEAD vs index. For unstaged files, diffs index vs workdir.
pub(crate) fn batch_diff_stats(
    repo: &Repository,
    staged: bool,
) -> std::collections::HashMap<PathBuf, (usize, usize)> {
    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);
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

pub(crate) fn generate_hunk_patch_for_repo(
    repo: &Repository,
    file_path: &Path,
    hunk_index: usize,
    staged: bool,
) -> Result<String> {
    let mut diff_opts = DiffOptions::new();
    diff_opts.pathspec(file_path);
    diff_opts.include_untracked(true);
    diff_opts.show_untracked_content(true);
    diff_opts.recurse_untracked_dirs(true);

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
        let Some(hunk) = hunk else {
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
        };
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

pub(crate) fn parse_multi_file_diff(diff: &git2::Diff) -> Result<CommitDiff> {
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
            let needs_new = current_hunks
                .last()
                .is_none_or(|h| h.new_start != expected_start || h.header != header);
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

pub(crate) fn delta_to_file_change_kind(delta: git2::Delta) -> FileChangeKind {
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
pub(crate) fn parse_file_diff(path: &Path, diff: &git2::Diff) -> Result<FileDiff> {
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
            let needs_new = file_diff
                .hunks
                .last()
                .is_none_or(|h| h.new_start != expected_start || h.header != header);
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
    diff_opts.recurse_untracked_dirs(true);
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
    parse_multi_file_diff(&diff)
}

pub fn compute_stash_diff(repo_path: &Path, index: usize) -> Result<CommitDiff> {
    let mut repo = Repository::open(repo_path)?;
    let mut stash_oids: Vec<git2::Oid> = Vec::new();
    repo.stash_foreach(|_idx, _msg, oid| {
        stash_oids.push(*oid);
        true
    })?;
    let stash_oid = stash_oids
        .get(index)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("Stash index {} out of range", index))?;
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

use gpui::{AsyncApp, Context, Task, WeakEntity};

use super::GitProject;
use super::GitProjectEvent;

impl GitProject {
    /// Get diff for a specific file (staged or unstaged).
    pub fn diff_file(&self, path: &Path, staged: bool) -> Result<FileDiff> {
        let repo = self.open_repo()?;
        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(path);
        diff_opts.include_untracked(true);
        diff_opts.show_untracked_content(true);
        diff_opts.recurse_untracked_dirs(true);

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
        parse_multi_file_diff(&diff)
    }

    /// Get the diff for a stash entry at the given index.
    pub fn diff_stash(&self, index: usize) -> Result<CommitDiff> {
        let mut repo = self.open_repo()?;

        let mut stash_oids: Vec<git2::Oid> = Vec::new();
        repo.stash_foreach(|_idx, _msg, oid| {
            stash_oids.push(*oid);
            true
        })?;

        let oid = *stash_oids
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Stash index {} out of range", index))?;

        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        parse_multi_file_diff(&diff)
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── delta_to_file_change_kind ──────────────────────────────────

    #[test]
    fn delta_added() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Added),
            FileChangeKind::Added
        ));
    }

    #[test]
    fn delta_untracked_maps_to_added() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Untracked),
            FileChangeKind::Added
        ));
    }

    #[test]
    fn delta_deleted() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Deleted),
            FileChangeKind::Deleted
        ));
    }

    #[test]
    fn delta_modified() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Modified),
            FileChangeKind::Modified
        ));
    }

    #[test]
    fn delta_typechange_maps_to_modified() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Typechange),
            FileChangeKind::Modified
        ));
    }

    #[test]
    fn delta_renamed() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Renamed),
            FileChangeKind::Renamed
        ));
    }

    #[test]
    fn delta_copied_maps_to_modified() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Copied),
            FileChangeKind::Modified
        ));
    }

    #[test]
    fn delta_conflicted_maps_to_modified() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Conflicted),
            FileChangeKind::Modified
        ));
    }

    #[test]
    fn delta_ignored_maps_to_modified() {
        assert!(matches!(
            delta_to_file_change_kind(git2::Delta::Ignored),
            FileChangeKind::Modified
        ));
    }
}

// ── Integration-level diff tests ────────────────────────────────

#[cfg(test)]
mod diff_integration_tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Set up a git repo with two commits; returns (TempDir, path, commit_oid).
    /// File content:
    ///   initial  → "line1\nline2\nline3\n"
    ///   amended  → "line1\nLINE2_CHANGED\nline3\n"
    fn make_two_commit_repo() -> (TempDir, std::path::PathBuf, git2::Oid) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();

        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Test").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);

        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        // First commit — write initial content
        let file = path.join("hello.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("hello.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let first = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let first_commit = repo.find_commit(first).unwrap();

        // Second commit — change line2
        std::fs::write(&file, "line1\nLINE2_CHANGED\nline3\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("hello.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let second = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "change line2",
                &tree,
                &[&first_commit],
            )
            .unwrap();

        (dir, path, second)
    }

    /// Build a repo where one file is staged (index differs from HEAD).
    fn make_staged_change_repo() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();

        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Test").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);

        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        // First commit
        let file = path.join("data.txt");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("data.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let first = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let first_commit = repo.find_commit(first).unwrap();
        let _ = first_commit;

        // Modify file and stage it — don't commit
        std::fs::write(&file, "alpha\nbeta_modified\ngamma\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("data.txt")).unwrap();
        idx.write().unwrap();

        (dir, path)
    }

    // ── compute_commit_diff ───────────────────────────────────────

    #[test]
    fn commit_diff_returns_changed_file() {
        let (_dir, path, oid) = make_two_commit_repo();
        let diff = compute_commit_diff(&path, oid).unwrap();
        assert_eq!(diff.files.len(), 1, "should have exactly one changed file");
        assert_eq!(diff.files[0].path, Path::new("hello.txt"));
    }

    #[test]
    fn commit_diff_counts_additions_and_deletions() {
        let (_dir, path, oid) = make_two_commit_repo();
        let diff = compute_commit_diff(&path, oid).unwrap();
        // "line2" → "LINE2_CHANGED": one deletion + one addition
        assert_eq!(diff.total_additions, 1);
        assert_eq!(diff.total_deletions, 1);
    }

    #[test]
    fn commit_diff_hunk_has_lines() {
        let (_dir, path, oid) = make_two_commit_repo();
        let diff = compute_commit_diff(&path, oid).unwrap();
        let file = &diff.files[0];
        assert!(!file.hunks.is_empty(), "should have at least one hunk");
        let hunk = &file.hunks[0];
        // Should contain an addition and a deletion
        let has_addition = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Addition(_)));
        let has_deletion = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Deletion(_)));
        assert!(has_addition, "hunk should have an addition line");
        assert!(has_deletion, "hunk should have a deletion line");
    }

    #[test]
    fn commit_diff_invalid_oid_returns_err() {
        let (_dir, path, _) = make_two_commit_repo();
        let fake_oid = git2::Oid::from_str("0000000000000000000000000000000000000000").unwrap();
        assert!(compute_commit_diff(&path, fake_oid).is_err());
    }

    #[test]
    fn commit_diff_first_commit_no_parent() {
        // The first commit has no parent; compute_commit_diff should handle it
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);
        let sig = git2::Signature::now("T", "t@t.com").unwrap();
        std::fs::write(path.join("f.txt"), "hello\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "root", &tree, &[])
            .unwrap();
        let diff = compute_commit_diff(&path, oid).unwrap();
        // Root commit: diff against empty tree → f.txt is added
        assert_eq!(diff.files.len(), 1);
        assert!(matches!(diff.files[0].kind, FileChangeKind::Added));
    }

    // ── compute_file_diff (unstaged) ─────────────────────────────

    #[test]
    fn file_diff_unstaged_no_changes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);
        let sig = git2::Signature::now("T", "t@t.com").unwrap();
        let f = path.join("clean.txt");
        std::fs::write(&f, "no changes\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("clean.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        // No modifications — should return empty hunk list
        let diff = compute_file_diff(&path, Path::new("clean.txt"), false).unwrap();
        assert!(diff.hunks.is_empty(), "no unstaged changes expected");
    }

    // ── compute_staged_diff_text ──────────────────────────────────

    #[test]
    fn staged_diff_text_contains_change() {
        let (_dir, path) = make_staged_change_repo();
        let text = compute_staged_diff_text(&path).unwrap();
        assert!(
            text.contains("beta_modified") || text.contains("+beta_modified"),
            "staged diff should include modified content"
        );
        assert!(
            text.contains("-beta") || text.contains("beta"),
            "staged diff should include original content"
        );
    }

    #[test]
    fn staged_diff_text_has_diff_markers() {
        let (_dir, path) = make_staged_change_repo();
        let text = compute_staged_diff_text(&path).unwrap();
        // compute_staged_diff_text prefixes each line with origin char
        assert!(
            text.contains('+') || text.contains('-'),
            "should have diff markers"
        );
    }

    // ── batch_diff_stats ─────────────────────────────────────────

    #[test]
    fn batch_diff_stats_staged_detects_changed_file() {
        let (_dir, path) = make_staged_change_repo();
        let repo = git2::Repository::open(&path).unwrap();
        let stats = batch_diff_stats(&repo, true);
        assert!(
            stats.contains_key(Path::new("data.txt")),
            "staged stats should include data.txt"
        );
        let (adds, dels) = stats[Path::new("data.txt")];
        assert_eq!(adds, 1, "one line added");
        assert_eq!(dels, 1, "one line deleted");
    }

    #[test]
    fn batch_diff_stats_unstaged_empty_when_clean() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);
        let sig = git2::Signature::now("T", "t@t.com").unwrap();
        let f = path.join("x.txt");
        std::fs::write(&f, "clean\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("x.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let stats = batch_diff_stats(&repo, false);
        // Working tree matches index → no unstaged changes
        assert!(stats.is_empty(), "no unstaged changes on clean repo");
    }

    // ── parse_multi_file_diff ─────────────────────────────────────

    #[test]
    fn parse_multi_file_diff_aggregates_files() {
        let (_dir, path, oid) = make_two_commit_repo();
        let repo = git2::Repository::open(&path).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        let tree = commit.tree().unwrap();
        let parent = commit.parent(0).unwrap();
        let parent_tree = parent.tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)
            .unwrap();
        let result = parse_multi_file_diff(&diff).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.total_additions, 1);
        assert_eq!(result.total_deletions, 1);
    }
}
