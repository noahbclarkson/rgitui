use anyhow::Result;
use git2::{DiffOptions, Repository};
use std::path::{Path, PathBuf};

use crate::types::*;

use super::refresh::gather_refresh_data_lightweight_cached;
use super::RefreshData;

/// Compute line-level diff stats (additions/deletions) for a single file.
/// For staged files, diffs HEAD vs index. For unstaged files, diffs index vs workdir.
pub(crate) fn batch_diff_stats(
    repo: &Repository,
    staged: bool,
) -> std::collections::HashMap<PathBuf, (usize, usize)> {
    let batch_timer = std::time::Instant::now();
    log::debug!("batch_diff_stats: staged={}", staged);
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
    log::debug!(
        "batch_diff_stats complete in {:?}: {} file stats, staged={}",
        batch_timer.elapsed(),
        stats_map.len(),
        staged
    );
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

        // Each hunk emits exactly one hunk-header line (origin 'H') before its
        // content lines, so advancing the counter on 'H' tracks hunk boundaries
        // structurally — no substring scan of the accumulating patch text.
        if line.origin() == 'H' {
            current_hunk_idx += 1;
        }

        if current_hunk_idx as usize == hunk_index {
            if !file_header_written {
                let old_path = delta.old_file().path().unwrap_or(Path::new(""));
                let new_path = delta.new_file().path().unwrap_or(Path::new(""));
                patch_text.clear();
                patch_text.push_str(&format!(
                    "diff --git a/{} b/{}\n",
                    old_path.display(),
                    new_path.display()
                ));
                patch_text.push_str(&format!("--- a/{}\n", old_path.display()));
                patch_text.push_str(&format!("+++ b/{}\n", new_path.display()));
                file_header_written = true;
            }

            let content = String::from_utf8_lossy(line.content());
            match line.origin() {
                'H' => {
                    let (old_start, old_lines, new_start, new_lines) = if staged {
                        (
                            hunk.new_start(),
                            hunk.new_lines(),
                            hunk.old_start(),
                            hunk.old_lines(),
                        )
                    } else {
                        (
                            hunk.old_start(),
                            hunk.old_lines(),
                            hunk.new_start(),
                            hunk.new_lines(),
                        )
                    };
                    patch_text.push_str(&format!(
                        "@@ -{},{} +{},{} @@\n",
                        old_start, old_lines, new_start, new_lines
                    ));
                }
                '+' | '-' | ' ' => {
                    if let Some(sign) = emitted_sign(line.origin(), staged) {
                        patch_text.push(sign);
                    }
                    patch_text.push_str(&content);
                }
                // EOFNL markers ('=' context, '>' added, '<' deleted) carry the
                // "\ No newline at end of file" text in their content; emit it
                // verbatim with no sign prefix, matching git's own patch output.
                '=' | '>' | '<' => patch_text.push_str(&content),
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

/// Map a diff line's origin to the sign it should carry in the generated patch.
///
/// Staging (`staged == false`, an index→workdir diff applied to the index) preserves
/// signs: a `+` workdir addition must stay `+` to add the line to the index.
/// Unstaging (`staged == true`, a HEAD→index diff applied to the index) negates them:
/// a `+` index addition becomes `-` so applying the patch removes it from the index.
/// Context lines keep their space; non-content origins return `None`.
fn emitted_sign(origin: char, staged: bool) -> Option<char> {
    match origin {
        ' ' => Some(' '),
        '+' => Some(if staged { '-' } else { '+' }),
        '-' => Some(if staged { '+' } else { '-' }),
        _ => None,
    }
}

/// Generate a patch containing only the specified line ranges from a file's diff.
///
/// `line_pairs` is `&[(Option<usize>, Option<usize>)]` — `(old_lineno, new_lineno)`
/// from the diff viewer, matching the line numbering of the underlying git2 diff.
/// The viewer emits an addition as `(None, Some(n))` and a deletion as `(Some(n), None)`.
///
/// `staged`:
/// - `false` → diff is index→workdir; the patch stages the selected lines into the
///   index, so signs are preserved (a `+` workdir addition stays `+`).
/// - `true` → diff is HEAD→index; the patch unstages the selected lines from the
///   index, so signs are negated (a `+` index addition becomes `-` to remove it).
///
/// Additions are matched by their `new_lineno` against the new-side targets, and
/// deletions by their `old_lineno` against the old-side targets. The two sides are
/// kept separate so an addition and a deletion that happen to share a line number
/// are not confused for one another.
pub(crate) fn generate_line_patch_for_repo(
    repo: &Repository,
    file_path: &Path,
    line_pairs: &[(Option<usize>, Option<usize>)],
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

    // Separate target sets per side. Additions carry their position in `new_lineno`,
    // deletions in `old_lineno`, in both the HEAD→index and index→workdir diffs.
    let new_targets: std::collections::HashSet<usize> =
        line_pairs.iter().filter_map(|(_, new)| *new).collect();
    let old_targets: std::collections::HashSet<usize> =
        line_pairs.iter().filter_map(|(old, _)| *old).collect();

    let mut patch_text = String::new();
    let num_deltas = diff.deltas().len();

    for i in 0..num_deltas {
        let patch = match git2::Patch::from_diff(&diff, i) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(_) => continue,
        };

        let old_path = patch
            .delta()
            .old_file()
            .path()
            .map(PathBuf::from)
            .unwrap_or_default();
        let new_path = patch
            .delta()
            .new_file()
            .path()
            .map(PathBuf::from)
            .unwrap_or_default();

        // Skip patches for other files (pathspec should filter, but be safe).
        if old_path != file_path && new_path != file_path {
            continue;
        }

        let num_hunks = patch.num_hunks();
        for hunk_idx in 0..num_hunks {
            let (hunk, _hunk_start) = patch.hunk(hunk_idx)?;
            let num_lines = patch.num_lines_in_hunk(hunk_idx)?;

            // Collect indices of lines within this hunk that match our targets.
            // Additions match on new_lineno, deletions on old_lineno; context and
            // EOFNL markers are always carried so the partial hunk stays coherent.
            let mut matching_line_indices: Vec<usize> = Vec::new();
            let mut has_change = false;
            for line_idx in 0..num_lines {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                let origin = line.origin();

                let is_target = match origin {
                    '+' => line
                        .new_lineno()
                        .is_some_and(|n| new_targets.contains(&(n as usize))),
                    '-' => line
                        .old_lineno()
                        .is_some_and(|n| old_targets.contains(&(n as usize))),
                    _ => false,
                };

                // Context lines and the "no newline at end of file" markers
                // ('=' context, '>' added, '<' deleted) are carried unconditionally.
                let is_context = matches!(origin, ' ' | '=' | '<' | '>');

                if is_target || is_context {
                    matching_line_indices.push(line_idx);
                    has_change |= matches!(origin, '+' | '-');
                }
            }

            // A hunk made up of only context/marker lines contributes nothing — skip
            // it so we never emit an empty change.
            if !has_change {
                continue;
            }

            // Count emitted pre-/post-image lines and derive the start positions.
            //
            // Counts reflect the *emitted* signs: when unstaging we negate, so an
            // addition becomes a deletion (pre-image side) and vice versa. EOFNL
            // markers annotate the preceding line and do not count toward either side.
            //
            // The output patch is applied to the index in both directions, but the
            // diff's own old/new line numbers describe different sides per direction:
            //   - staging   (index→workdir): pre-image = index   → old_lineno
            //                                 post-image = workdir → new_lineno
            //   - unstaging (HEAD→index):     pre-image = index   → new_lineno
            //                                 post-image = HEAD    → old_lineno
            // so the line number that anchors each side is selected accordingly.
            let mut old_count: u32 = 0;
            let mut new_count: u32 = 0;
            let mut old_start: Option<u32> = None;
            let mut new_start: Option<u32> = None;

            for &line_idx in &matching_line_indices {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                let (preimage, postimage) = if staged {
                    (line.new_lineno(), line.old_lineno())
                } else {
                    (line.old_lineno(), line.new_lineno())
                };
                match emitted_sign(line.origin(), staged) {
                    Some(' ') => {
                        old_count += 1;
                        new_count += 1;
                        old_start = old_start.or(preimage);
                        new_start = new_start.or(postimage);
                    }
                    Some('-') => {
                        old_count += 1;
                        old_start = old_start.or(preimage);
                    }
                    Some('+') => {
                        new_count += 1;
                        new_start = new_start.or(postimage);
                    }
                    _ => {}
                }
            }

            // Fall back to the hunk's own boundaries when one side is empty (e.g. a
            // pure-addition selection has no pre-image line to anchor old_start).
            let (hunk_old_start, hunk_new_start) = if staged {
                (
                    old_start.unwrap_or_else(|| hunk.new_start()),
                    new_start.unwrap_or_else(|| hunk.old_start()),
                )
            } else {
                (
                    old_start.unwrap_or_else(|| hunk.old_start()),
                    new_start.unwrap_or_else(|| hunk.new_start()),
                )
            };

            // Write file header (if first hunk for this file). The `diff --git`
            // line is required for libgit2's `Diff::from_buffer` to recognize the
            // file header; without it the first `@@` is rejected as a hunk header
            // "outside patch".
            if patch_text.is_empty() {
                patch_text.push_str(&format!(
                    "diff --git a/{} b/{}\n",
                    old_path.display(),
                    new_path.display()
                ));
                patch_text.push_str(&format!("--- a/{}\n", old_path.display()));
                patch_text.push_str(&format!("+++ b/{}\n", new_path.display()));
            }

            // Write hunk header with counts derived from the emitted lines.
            patch_text.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                hunk_old_start, old_count, hunk_new_start, new_count
            ));

            // Write the matching lines, applying the sign transform per direction.
            for &line_idx in &matching_line_indices {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                let content = String::from_utf8_lossy(line.content());
                match line.origin() {
                    '+' | '-' | ' ' => {
                        if let Some(sign) = emitted_sign(line.origin(), staged) {
                            patch_text.push(sign);
                        }
                        patch_text.push_str(&content);
                    }
                    // EOFNL markers carry the "\ No newline at end of file" text in
                    // their content; emit it verbatim with no sign prefix.
                    '=' | '>' | '<' => patch_text.push_str(&content),
                    _ => {}
                }
            }
        }
    }

    if patch_text.is_empty() {
        anyhow::bail!("Could not generate patch for selected lines");
    }

    if !patch_text.ends_with('\n') {
        patch_text.push('\n');
    }

    Ok(patch_text)
}

/// Parse a git2::Diff into a CommitDiff using structured Patch iteration.
/// Cross-commit parallelism is handled by the caller; this function processes
/// a single diff sequentially to avoid serialize/re-parse overhead.
pub(crate) fn parse_multi_file_diff(diff: &git2::Diff) -> Result<CommitDiff> {
    let num_patches = diff.deltas().len();
    let mut files = Vec::with_capacity(num_patches);
    for i in 0..num_patches {
        if let Some(mut patch) = git2::Patch::from_diff(diff, i)? {
            files.push(parse_single_patch(&mut patch)?);
        }
    }

    let total_additions = files.iter().map(|f| f.additions).sum();
    let total_deletions = files.iter().map(|f| f.deletions).sum();

    Ok(CommitDiff {
        total_additions,
        total_deletions,
        files,
    })
}

/// Parse a single git2::Patch into a FileDiff.
/// Uses the patch's print callback to extract hunk/line content.
fn parse_single_patch(patch: &mut git2::Patch) -> Result<FileDiff> {
    let path = patch
        .delta()
        .new_file()
        .path()
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let kind = delta_to_file_change_kind(patch.delta().status());
    let (_, additions, deletions) = patch.line_stats().unwrap_or((0, 0, 0));

    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;

    // Use the patch's embedded print callback to extract hunk/line content.
    // patch.print() processes THIS PATCH ONLY (not the full diff), giving us
    // per-file isolation without needing a separate diff per file.
    patch.print(&mut |_delta, hunk_range, line| {
        if let Some(hunk) = hunk_range {
            let header = String::from_utf8_lossy(hunk.header()).to_string();
            let needs_new = current_hunk
                .as_ref()
                .is_none_or(|h| h.new_start != hunk.new_start() || h.header != header);
            if needs_new {
                if let Some(prev) = current_hunk.take() {
                    hunks.push(prev);
                }
                current_hunk = Some(DiffHunk {
                    old_start: hunk.old_start(),
                    old_lines: hunk.old_lines(),
                    new_start: hunk.new_start(),
                    new_lines: hunk.new_lines(),
                    header,
                    lines: Vec::new(),
                });
            }
        }

        if let Some(ref mut hunk) = current_hunk {
            // delta is the same for all lines in a patch-level print
            let content = String::from_utf8_lossy(line.content()).to_string();
            match line.origin() {
                '+' => hunk.lines.push(DiffLine::Addition(content)),
                '-' => hunk.lines.push(DiffLine::Deletion(content)),
                ' ' => hunk.lines.push(DiffLine::Context(content)),
                _ => {}
            }
        }

        true
    })?;

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    Ok(FileDiff {
        path,
        hunks,
        additions,
        deletions,
        kind,
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
    let diff_timer = std::time::Instant::now();
    log::debug!(
        "compute_commit_diff: oid={} repo={}",
        oid,
        repo_path.display()
    );
    let repo = Repository::open(repo_path)?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let result = parse_multi_file_diff(&diff);
    if let Ok(ref d) = result {
        log::debug!(
            "compute_commit_diff complete in {:?}: {} files",
            diff_timer.elapsed(),
            d.files.len()
        );
    }
    result
}

pub fn compute_stash_diff(repo_path: &Path, index: usize) -> Result<CommitDiff> {
    let mut repo = Repository::open(repo_path)?;
    let mut stash_oid: Option<git2::Oid> = None;
    repo.stash_foreach(|idx, _msg, oid| {
        if idx == index {
            stash_oid = Some(*oid);
            false // stop early — found the target stash
        } else {
            true
        }
    })?;
    let stash_oid =
        stash_oid.ok_or_else(|| anyhow::anyhow!("Stash index {} out of range", index))?;
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
        let worktree_path = self.repo_path.clone();
        self.stage_hunk_at(file_path, hunk_index, &worktree_path, cx)
    }

    /// Stage a specific hunk in the given worktree.
    pub fn stage_hunk_at(
        &mut self,
        file_path: &Path,
        hunk_index: usize,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
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
                    let repo = Repository::open(&worktree_path)?;
                    let patch_text =
                        generate_hunk_patch_for_repo(&repo, &task_file_path, hunk_index, false)?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    repo.apply(&diff, git2::ApplyLocation::Index, None)?;
                    gather_refresh_data_lightweight_cached(
                        &refresh_repo_path,
                        commit_limit,
                        &worktree_cache,
                        author_filter.as_deref(),
                    )
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.refresh_ahead_behind(cx);
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
        let worktree_path = self.repo_path.clone();
        self.unstage_hunk_at(file_path, hunk_index, &worktree_path, cx)
    }

    /// Unstage a specific hunk from a staged file diff in the given worktree.
    pub fn unstage_hunk_at(
        &mut self,
        file_path: &Path,
        hunk_index: usize,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
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
                    let repo = Repository::open(&worktree_path)?;
                    let patch_text =
                        generate_hunk_patch_for_repo(&repo, &task_file_path, hunk_index, true)?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    let mut opts = git2::ApplyOptions::new();
                    repo.apply(&diff, git2::ApplyLocation::Index, Some(&mut opts))?;
                    gather_refresh_data_lightweight_cached(
                        &refresh_repo_path,
                        commit_limit,
                        &worktree_cache,
                        author_filter.as_deref(),
                    )
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.refresh_ahead_behind(cx);
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

    /// Stage specific lines within a file's diff.
    /// `line_pairs` is `&[(Option<usize>, Option<usize>)]` from the diff viewer —
    /// (old_lineno, new_lineno) for each selected line.
    pub fn stage_lines(
        &mut self,
        file_path: &Path,
        line_pairs: &[(Option<usize>, Option<usize>)],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let worktree_path = self.repo_path.clone();
        self.stage_lines_at(file_path, line_pairs, &worktree_path, cx)
    }

    /// Stage specific lines within a file's diff in the given worktree.
    pub fn stage_lines_at(
        &mut self,
        file_path: &Path,
        line_pairs: &[(Option<usize>, Option<usize>)],
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
        let branch_name = self.head_branch.clone();
        let line_count = line_pairs.len();
        let line_pairs_owned = line_pairs.to_vec();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            format!(
                "Staging {} line{} in {}...",
                line_count,
                if line_count == 1 { "" } else { "s" },
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
                    let repo = Repository::open(&worktree_path)?;
                    let patch_text = generate_line_patch_for_repo(
                        &repo,
                        &task_file_path,
                        &line_pairs_owned,
                        false, // staging from workdir to index
                    )?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    repo.apply(&diff, git2::ApplyLocation::Index, None)?;
                    gather_refresh_data_lightweight_cached(
                        &refresh_repo_path,
                        commit_limit,
                        &worktree_cache,
                        author_filter.as_deref(),
                    )
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.refresh_ahead_behind(cx);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Stage,
                                format!(
                                    "Staged {} line{} in {}",
                                    line_count,
                                    if line_count == 1 { "" } else { "s" },
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
                                "Stage lines failed",
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

    /// Unstage specific lines from a staged file diff.
    pub fn unstage_lines(
        &mut self,
        file_path: &Path,
        line_pairs: &[(Option<usize>, Option<usize>)],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let worktree_path = self.repo_path.clone();
        self.unstage_lines_at(file_path, line_pairs, &worktree_path, cx)
    }

    /// Unstage specific lines from a staged file diff in the given worktree.
    pub fn unstage_lines_at(
        &mut self,
        file_path: &Path,
        line_pairs: &[(Option<usize>, Option<usize>)],
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
        let branch_name = self.head_branch.clone();
        let line_count = line_pairs.len();
        let line_pairs_owned = line_pairs.to_vec();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            format!(
                "Unstaging {} line{} in {}...",
                line_count,
                if line_count == 1 { "" } else { "s" },
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
                    let repo = Repository::open(&worktree_path)?;
                    // staged=true: diff is HEAD→index; we negate signs to remove from index.
                    let patch_text = generate_line_patch_for_repo(
                        &repo,
                        &task_file_path,
                        &line_pairs_owned,
                        true, // unstaging from staged (HEAD→index)
                    )?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    let mut opts = git2::ApplyOptions::new();
                    repo.apply(&diff, git2::ApplyLocation::Index, Some(&mut opts))?;
                    gather_refresh_data_lightweight_cached(
                        &refresh_repo_path,
                        commit_limit,
                        &worktree_cache,
                        author_filter.as_deref(),
                    )
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(data) => {
                            this.apply_refresh_data(data);
                            this.refresh_ahead_behind(cx);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                format!(
                                    "Unstaged {} line{} in {}",
                                    line_count,
                                    if line_count == 1 { "" } else { "s" },
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
                                "Unstage lines failed",
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

// ── Three-way conflict diff ───────────────────────────────────

/// Compute a 3-way conflict diff for a conflicted file.
///
/// Returns the ancestor (merge-base), ours, and theirs versions along with
/// detected conflict regions.
pub fn compute_three_way_conflict_diff(
    repo_path: &Path,
    file_path: &Path,
) -> Result<ThreeWayFileDiff> {
    let repo = Repository::open(repo_path)?;
    let index = repo.index()?;
    let mut conflicts = index.conflicts()?;

    let path_bytes = file_path.as_os_str().as_encoded_bytes();

    let conflict_entry = loop {
        if let Some(Ok(conflict)) = conflicts.next() {
            let conflict_path_bytes: Option<&[u8]> = conflict
                .our
                .as_ref()
                .map(|e| e.path.as_slice())
                .or_else(|| conflict.their.as_ref().map(|e| e.path.as_slice()))
                .or_else(|| conflict.ancestor.as_ref().map(|e| e.path.as_slice()));

            if conflict_path_bytes.is_some_and(|pb| pb == path_bytes) {
                break conflict;
            }
        } else {
            anyhow::bail!("conflict not found for path '{}'", file_path.display());
        }
    };

    fn read_blob_text(repo: &Repository, entry: &git2::IndexEntry) -> Result<Vec<String>> {
        let blob = repo.find_blob(entry.id)?;
        let content = blob.content();
        let text = String::from_utf8_lossy(content);
        Ok(text.lines().map(|l| l.to_string()).collect())
    }

    let ancestor_lines = if let Some(ref entry) = conflict_entry.ancestor {
        read_blob_text(&repo, entry)?
    } else {
        Vec::new()
    };

    let ours_lines = if let Some(ref entry) = conflict_entry.our {
        read_blob_text(&repo, entry)?
    } else {
        Vec::new()
    };

    let theirs_lines = if let Some(ref entry) = conflict_entry.their {
        read_blob_text(&repo, entry)?
    } else {
        Vec::new()
    };

    // Detect conflict regions by comparing each version to the ancestor.
    // A region is "conflicted" if both ours and theirs differ from ancestor.
    // A region is "ours-only" if ours differs but theirs matches ancestor.
    // A region is "theirs-only" if theirs differs but ours matches ancestor.
    let regions = compute_conflict_regions(&ancestor_lines, &ours_lines, &theirs_lines);

    Ok(ThreeWayFileDiff {
        path: file_path.to_path_buf(),
        ancestor_lines,
        ours_lines,
        theirs_lines,
        regions,
    })
}

/// Compute conflict/non-conflict regions between ancestor/ours/theirs.
fn compute_conflict_regions(
    ancestor: &[String],
    ours: &[String],
    theirs: &[String],
) -> Vec<ConflictRegion> {
    let n = ancestor.len().max(ours.len()).max(theirs.len());
    if n == 0 {
        return Vec::new();
    }

    // For each line index, determine if ours/theirs differ from ancestor
    let ours_diffs: Vec<bool> = (0..n)
        .map(|i| {
            let a = ancestor.get(i);
            let o = ours.get(i);
            // Differ if one is None and the other is Some, or both Some but different
            match (a, o) {
                (None, None) | (Some(_), None) | (None, Some(_)) => true,
                (Some(a), Some(o)) => a != o,
            }
        })
        .collect();

    let theirs_diffs: Vec<bool> = (0..n)
        .map(|i| {
            let a = ancestor.get(i);
            let t = theirs.get(i);
            match (a, t) {
                (None, None) | (Some(_), None) | (None, Some(_)) => true,
                (Some(a), Some(t)) => a != t,
            }
        })
        .collect();

    // Merge consecutive runs into regions
    let mut regions = Vec::new();
    let mut i = 0;
    while i < n {
        let ours_diff = ours_diffs[i];
        let theirs_diff = theirs_diffs[i];
        let start = i;
        while i < n && ours_diffs[i] == ours_diff && theirs_diffs[i] == theirs_diff {
            i += 1;
        }
        let is_conflict = ours_diff && theirs_diff;
        // Only record regions that are non-empty and actually differ in at least one side
        if ours_diff || theirs_diff {
            regions.push(ConflictRegion {
                start,
                end: i,
                is_conflict,
            });
        }
    }
    regions
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

    // ── emitted_sign ──────────────────────────────────────────────

    #[test]
    fn emitted_sign_staging_preserves_signs() {
        // Staging (staged=false) applies an index→workdir patch to the index, so
        // signs are preserved: a workdir addition stays '+', a deletion stays '-'.
        assert_eq!(emitted_sign('+', false), Some('+'));
        assert_eq!(emitted_sign('-', false), Some('-'));
        assert_eq!(emitted_sign(' ', false), Some(' '));
        assert_eq!(emitted_sign('=', false), None);
    }

    #[test]
    fn emitted_sign_unstaging_negates_signs() {
        // Unstaging (staged=true) applies a HEAD→index patch to the index, so signs
        // are negated: an index addition becomes '-' (removed), a deletion '+'.
        assert_eq!(emitted_sign('+', true), Some('-'));
        assert_eq!(emitted_sign('-', true), Some('+'));
        assert_eq!(emitted_sign(' ', true), Some(' '));
        assert_eq!(emitted_sign('>', true), None);
    }

    // ── generate_line_patch_for_repo tests ───────────────────────────────────

    /// End-to-end check of the viewer→git contract for unstaging.
    ///
    /// The diff viewer emits an addition as `(None, Some(new_lineno))`. For the
    /// staged diff that turns "beta" into "beta_modified", the addition
    /// "+beta_modified" has new_lineno=2, so the viewer sends `(None, Some(2))`.
    /// Unstaging must negate that addition into a "-beta_modified" deletion so the
    /// applied patch removes the line from the index.
    ///
    /// This is the contract BUG-14 corrected: the previous code matched additions
    /// against the `old` slot, which is always `None` for viewer-emitted additions,
    /// silently producing a context-only no-op patch.
    #[test]
    fn hunk_patch_stages_and_parses_as_a_complete_patch() {
        let (_dir, path) = make_staged_change_repo();
        let repo = git2::Repository::open(&path).unwrap();

        let head_tree = repo.head().unwrap().peel_to_tree().unwrap();
        let mut index = repo.index().unwrap();
        index.read_tree(&head_tree).unwrap();
        index.write().unwrap();

        let patch_text =
            generate_hunk_patch_for_repo(&repo, Path::new("data.txt"), 0, false).unwrap();
        assert!(patch_text.starts_with("diff --git "));
        let diff = git2::Diff::from_buffer(patch_text.as_bytes()).unwrap();
        repo.apply(&diff, git2::ApplyLocation::Index, None).unwrap();
    }

    #[test]
    fn hunk_patch_unstaging_reverses_the_change() {
        let (_dir, path) = make_staged_change_repo();
        let repo = git2::Repository::open(&path).unwrap();

        let patch_text =
            generate_hunk_patch_for_repo(&repo, Path::new("data.txt"), 0, true).unwrap();
        assert!(patch_text.contains("-beta_modified\n"));
        assert!(patch_text.contains("+beta\n"));
        let diff = git2::Diff::from_buffer(patch_text.as_bytes()).unwrap();
        repo.apply(&diff, git2::ApplyLocation::Index, None).unwrap();

        let head_tree = repo.head().unwrap().peel_to_tree().unwrap();
        let staged = repo
            .diff_tree_to_index(Some(&head_tree), None, None)
            .unwrap();
        assert_eq!(staged.deltas().len(), 0);
    }

    #[test]
    fn line_patch_unstage_addition_negates_to_deletion() {
        let (_dir, path) = make_staged_change_repo();
        let repo = git2::Repository::open(&path).unwrap();

        // Viewer-produced pair for the "+beta_modified" addition (new_lineno=2).
        let line_pairs = vec![(None, Some(2usize))];
        let patch_text =
            generate_line_patch_for_repo(&repo, Path::new("data.txt"), &line_pairs, true).unwrap();

        assert!(
            patch_text.contains("-beta_modified\n"),
            "unstaging should negate the addition into '-beta_modified', got:\n{patch_text}"
        );
        assert!(
            !patch_text.contains("+beta_modified\n"),
            "the addition must not be emitted with its original '+' sign, got:\n{patch_text}"
        );

        // The patch is applied to the index, whose content is the pre-image. Only a
        // correct old_start/new_start swap (BUG-13) yields a patch git will accept;
        // a text-only check cannot catch an off-by-one in those positions.
        let diff = git2::Diff::from_buffer(patch_text.as_bytes()).unwrap();
        repo.apply(&diff, git2::ApplyLocation::Index, None).unwrap();
    }

    /// A deletion selected for unstaging (viewer pair `(Some(old_lineno), None)`)
    /// must be negated into a restoring "+" line.
    #[test]
    fn line_patch_unstage_deletion_negates_to_addition() {
        let (_dir, path) = make_staged_change_repo();
        let repo = git2::Repository::open(&path).unwrap();

        // "-beta" deletion in the staged diff has old_lineno=2.
        let line_pairs = vec![(Some(2usize), None)];
        let patch_text =
            generate_line_patch_for_repo(&repo, Path::new("data.txt"), &line_pairs, true).unwrap();

        assert!(
            patch_text.contains("+beta\n"),
            "unstaging should negate the deletion into '+beta', got:\n{patch_text}"
        );
    }

    /// Staging a workdir addition preserves its '+' sign so applying the patch adds
    /// the line to the index. Build an unstaged-only addition and stage it.
    #[test]
    fn line_patch_stage_addition_preserves_sign() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);
        let sig = git2::Signature::now("T", "t@t.com").unwrap();

        let file = path.join("data.txt");
        std::fs::write(&file, "alpha\ngamma\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("data.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // Insert "beta" in the working tree only (index→workdir addition at new line 2).
        std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

        let line_pairs = vec![(None, Some(2usize))];
        let patch_text =
            generate_line_patch_for_repo(&repo, Path::new("data.txt"), &line_pairs, false).unwrap();

        assert!(
            patch_text.contains("+beta\n"),
            "staging should preserve the '+beta' addition, got:\n{patch_text}"
        );

        // The generated patch must apply cleanly to the index.
        let diff = git2::Diff::from_buffer(patch_text.as_bytes()).unwrap();
        repo.apply(&diff, git2::ApplyLocation::Index, None).unwrap();
    }

    /// Appending a line at the end of a long file makes the last matched line an
    /// addition (old_lineno = None) inside a hunk that starts well past line 1.
    /// The old `last_lineno + 1 - hunk_old_start` math underflowed u32 here
    /// (BUG-13); counting emitted lines instead must produce an applicable patch.
    #[test]
    fn line_patch_stage_trailing_addition_no_underflow() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@t.com").unwrap();
        drop(cfg);
        let sig = git2::Signature::now("T", "t@t.com").unwrap();

        let file = path.join("data.txt");
        let base: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        std::fs::write(&file, &base).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("data.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // Append a line at the end; its hunk starts past line 1 and ends on the
        // addition itself (old_lineno = None for that line).
        std::fs::write(&file, format!("{base}appended\n")).unwrap();

        // The appended line sits at new line 21.
        let line_pairs = vec![(None, Some(21usize))];
        let patch_text =
            generate_line_patch_for_repo(&repo, Path::new("data.txt"), &line_pairs, false).unwrap();

        assert!(
            patch_text.contains("+appended\n"),
            "patch should contain the appended line, got:\n{patch_text}"
        );
        // Underflow would yield "@@ -N,4294967292 ... @@"; assert no such count.
        assert!(
            !patch_text.contains("4294967292"),
            "hunk header must not carry an underflowed count, got:\n{patch_text}"
        );
        let diff = git2::Diff::from_buffer(patch_text.as_bytes()).unwrap();
        repo.apply(&diff, git2::ApplyLocation::Index, None).unwrap();
    }
}
