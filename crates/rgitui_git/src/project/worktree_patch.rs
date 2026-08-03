//! Applying and reverting diff content in the working tree.
//!
//! The patch comes from a pair of trees that need not include the working tree
//! at all — a past commit against its parent, a stash entry, or two arbitrary
//! branches — and the result is written to files on disk. A dirty working tree is
//! the normal case: you compare against another revision precisely because you
//! are mid-change.
//!
//! ## Constraints on the two obvious appliers
//!
//! Neither off-the-shelf applier tolerates that dirty working tree:
//!
//! * `repo.apply(.., ApplyLocation::WorkDir, ..)` matches each hunk's context
//!   against the target and fails outright when it does not line up, with no
//!   three-way fallback. An uncommitted edit anywhere inside a hunk's context
//!   window (three lines either side) defeats it.
//! * `git apply --3way` merges into the *index*, and refuses with
//!   `error: <path>: does not match index` whenever the working-tree file differs
//!   from its index entry. On failure it also leaves conflict markers and an
//!   unmerged index entry behind.
//!
//! ## What this module does
//!
//! Reconstructs the two sides and lets libgit2 merge them:
//!
//! * `base`   — the file as it is on the side the patch starts from.
//! * `target` — `base` rewritten with exactly the selected hunk or lines
//!   applied (or reverted). Computed from the diff rather than by matching
//!   context, so it is exact by construction.
//! * `ours`   — the file as it is in the working tree right now.
//!
//! A three-way merge of (base, ours, target) then keeps unrelated local edits
//! and conflicts only where an edit overlaps the selected region. The index is
//! never touched and never has to match the working tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use git2::{IndexEntry, IndexTime, Oid, Repository};

/// Which side of a diff the working tree should be moved toward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreePatchDirection {
    /// Bring the diff's new-side content into the working tree — the
    /// hunk-level equivalent of a cherry-pick.
    Apply,
    /// Restore the diff's old-side content in the working tree — the
    /// hunk-level equivalent of a revert.
    Revert,
}

impl WorktreePatchDirection {
    /// Verb used in progress and success messages ("Applying", "Applied").
    pub fn present_participle(self) -> &'static str {
        match self {
            WorktreePatchDirection::Apply => "Applying",
            WorktreePatchDirection::Revert => "Reverting",
        }
    }

    /// Verb used in success messages.
    pub fn past_tense(self) -> &'static str {
        match self {
            WorktreePatchDirection::Apply => "Applied",
            WorktreePatchDirection::Revert => "Reverted",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            WorktreePatchDirection::Apply => "apply",
            WorktreePatchDirection::Revert => "revert",
        }
    }

    /// The direction that undoes this one.
    pub fn inverse(self) -> Self {
        match self {
            WorktreePatchDirection::Apply => WorktreePatchDirection::Revert,
            WorktreePatchDirection::Revert => WorktreePatchDirection::Apply,
        }
    }
}

/// How much of a file's diff a working-tree apply or revert covers.
///
/// The three variants are the three granularities the diff viewer offers: the
/// hunk under the cursor by default, a manual line selection when the user has
/// made one, and the whole file from the file-level menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreePatchScope {
    /// Every hunk in the file.
    File,
    /// One hunk, indexed as the diff viewer displays it.
    Hunk(usize),
    /// Only the given change lines, as `(old_lineno, new_lineno)` pairs from
    /// the diff viewer: an addition is `(None, Some(n))`, a deletion
    /// `(Some(n), None)`.
    Lines(Vec<(Option<usize>, Option<usize>)>),
}

impl WorktreePatchScope {
    /// Human-readable description used in operation summaries and toasts.
    pub fn describe(&self) -> String {
        match self {
            WorktreePatchScope::File => "all changes".to_string(),
            WorktreePatchScope::Hunk(index) => format!("hunk {}", index + 1),
            WorktreePatchScope::Lines(pairs) => format!(
                "{} line{}",
                pairs.len(),
                if pairs.len() == 1 { "" } else { "s" }
            ),
        }
    }
}

/// The pair of revisions whose difference is being applied or reverted.
///
/// Both variants resolve to a `from`/`to` tree pair, so a commit diff and a
/// cross-branch comparison travel identical code from there on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreePatchSource {
    /// The change one commit (or stash entry) introduced: its first parent →
    /// itself. A root commit's `from` side is the empty tree.
    Commit(Oid),
    /// The difference between two arbitrary revisions, `from` → `to`. Both are
    /// resolved with `revparse_single`, so branch names, tags, `HEAD~3`,
    /// `origin/main` and raw OIDs all work.
    Compare { from: String, to: String },
}

impl WorktreePatchSource {
    /// Short label naming this source in user-facing messages.
    pub fn label(&self) -> String {
        match self {
            WorktreePatchSource::Commit(oid) => short_oid(&oid.to_string()),
            WorktreePatchSource::Compare { from, to } => format!("{from}...{to}"),
        }
    }

    /// The (from, to) trees this source compares. Either side may be absent —
    /// a root commit has no parent tree, and an explicitly empty revision
    /// resolves to no tree.
    fn trees<'repo>(
        &self,
        repo: &'repo Repository,
    ) -> Result<(Option<git2::Tree<'repo>>, Option<git2::Tree<'repo>>)> {
        match self {
            WorktreePatchSource::Commit(oid) => {
                let commit = repo.find_commit(*oid).with_context(|| {
                    format!(
                        "Commit {} is not in this repository",
                        short_oid(&oid.to_string())
                    )
                })?;
                let to = commit.tree()?;
                let from = if commit.parent_count() > 0 {
                    Some(commit.parent(0)?.tree()?)
                } else {
                    None
                };
                Ok((from, Some(to)))
            }
            WorktreePatchSource::Compare { from, to } => Ok((
                Some(resolve_tree(repo, from)?),
                Some(resolve_tree(repo, to)?),
            )),
        }
    }
}

fn resolve_tree<'repo>(repo: &'repo Repository, rev: &str) -> Result<git2::Tree<'repo>> {
    let object = repo.revparse_single(rev).with_context(|| {
        format!("Can't resolve '{rev}' — check the branch, tag or commit name and try again.")
    })?;
    object
        .peel_to_tree()
        .with_context(|| format!("'{rev}' does not name a commit or tree."))
}

/// Shorten a hex OID for display, leaving anything already short untouched.
pub(crate) fn short_oid(oid_hex: &str) -> String {
    oid_hex[..7.min(oid_hex.len())].to_string()
}

/// The contents of one working-tree file before an apply or revert rewrote it.
///
/// Undo restores these bytes verbatim rather than deriving a reverse patch, which
/// would not be exact when the forward operation went through a three-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeFileSnapshot {
    /// Path relative to the worktree root.
    pub path: PathBuf,
    /// Contents before the operation; `None` when the file did not exist.
    pub contents: Option<Vec<u8>>,
}

/// Largest total snapshot the undo stack will hold for one operation. Past this
/// the operation still runs, but it is not offered as undoable rather than
/// parking tens of megabytes in a 20-deep history.
pub const MAX_UNDO_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

/// Whether `snapshots` are small enough to keep in the undo stack.
///
/// Pure so the cap is testable without a repository.
pub fn snapshots_fit_undo_stack(snapshots: &[WorktreeFileSnapshot]) -> bool {
    snapshots
        .iter()
        .filter_map(|s| s.contents.as_ref().map(Vec::len))
        .sum::<usize>()
        <= MAX_UNDO_SNAPSHOT_BYTES
}

/// Outcome of a successful working-tree apply or revert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreePatchOutcome {
    /// Pre-operation contents of every file the operation rewrote.
    pub snapshots: Vec<WorktreeFileSnapshot>,
    /// True when the change could not be dropped in verbatim and was merged
    /// around unrelated local edits. Worth telling the user about: their file
    /// now holds both their edit and the applied change.
    pub merged_with_local_changes: bool,
}

// ── Pure line rewriting ───────────────────────────────────────────────────────

/// Split `text` into lines, keeping each line's terminator so a file with no
/// trailing newline round-trips unchanged.
pub(crate) fn split_keeping_terminators(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == '\n' {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// One line of a hunk, reduced to what the rewrite needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScopedLine {
    Context {
        old_lineno: usize,
        new_lineno: usize,
    },
    Addition {
        new_lineno: usize,
        text: String,
    },
    Deletion {
        old_lineno: usize,
        text: String,
    },
}

impl ScopedLine {
    /// This line's position on `direction`'s base side, or `None` when it only
    /// exists on the target side.
    fn base_lineno(&self, direction: WorktreePatchDirection) -> Option<usize> {
        match (self, direction) {
            (ScopedLine::Context { old_lineno, .. }, WorktreePatchDirection::Apply) => {
                Some(*old_lineno)
            }
            (ScopedLine::Context { new_lineno, .. }, WorktreePatchDirection::Revert) => {
                Some(*new_lineno)
            }
            (ScopedLine::Deletion { old_lineno, .. }, WorktreePatchDirection::Apply) => {
                Some(*old_lineno)
            }
            (ScopedLine::Addition { new_lineno, .. }, WorktreePatchDirection::Revert) => {
                Some(*new_lineno)
            }
            _ => None,
        }
    }

    fn is_context(&self) -> bool {
        matches!(self, ScopedLine::Context { .. })
    }
}

/// Rewrite `base` — the file as it stands on `direction`'s starting side — into
/// the side the operation moves toward, covering only what the scope selects.
///
/// `hunks` is the file's diff, hunk by hunk, in display order. `selected_hunks`
/// is `None` for "every hunk"; `selected_lines` is `None` for "every change line
/// of the selected hunks" and otherwise the `(old, new)` pairs the viewer
/// emitted.
///
/// Within a run of consecutive `-`/`+` lines the *i*-th removal is treated as
/// paired with the *i*-th insertion, matching what the side-by-side view shows
/// the user. Git's unified output groups all removals ahead of all insertions,
/// so reading the rows in order would put a kept line on the wrong side of an
/// applied one when only part of a run is selected.
///
/// Per pair the rule is uniform in both directions: emit the target-side line
/// when its row is selected, and keep the base-side line when its row is not.
/// Selecting both sides of a substitution therefore replaces the line, selecting
/// neither leaves it alone, selecting only the insertion adds a line beside the
/// original, and selecting only the removal drops it.
///
/// Pure: no repository, no filesystem, no GPUI, and both directions run the same
/// walk with the two sides swapped, so an apply and a revert can never disagree
/// about which lines they touch.
pub(crate) fn rewrite_side(
    base: &[String],
    hunks: &[Vec<ScopedLine>],
    direction: WorktreePatchDirection,
    selected_hunks: Option<&HashSet<usize>>,
    selected_lines: Option<&SelectedLines>,
) -> String {
    /// Emit base lines from `cursor` up to (but not including) `upto`.
    fn copy_base_through<'a>(
        pieces: &mut Vec<&'a str>,
        base: &'a [String],
        cursor: &mut usize,
        upto: usize,
    ) {
        while *cursor < upto && *cursor <= base.len() {
            pieces.push(&base[*cursor - 1]);
            *cursor += 1;
        }
    }

    let mut pieces: Vec<&str> = Vec::new();
    // Next base line number not yet emitted, 1-based.
    let mut cursor = 1usize;

    for (hunk_index, lines) in hunks.iter().enumerate() {
        let hunk_selected = selected_hunks.is_none_or(|set| set.contains(&hunk_index));
        let is_selected = |line: &ScopedLine| -> bool {
            if !hunk_selected {
                return false;
            }
            match (line, selected_lines) {
                (_, None) => true,
                (ScopedLine::Addition { new_lineno, .. }, Some(selected)) => {
                    selected.contains_addition(*new_lineno)
                }
                (ScopedLine::Deletion { old_lineno, .. }, Some(selected)) => {
                    selected.contains_deletion(*old_lineno)
                }
                (ScopedLine::Context { .. }, Some(_)) => false,
            }
        };

        // Copy the untouched region ahead of this hunk.
        if let Some(first) = lines.iter().filter_map(|l| l.base_lineno(direction)).min() {
            copy_base_through(&mut pieces, base, &mut cursor, first);
        }

        let mut index = 0;
        while index < lines.len() {
            if lines[index].is_context() {
                if let Some(lineno) = lines[index].base_lineno(direction) {
                    copy_base_through(&mut pieces, base, &mut cursor, lineno);
                    if lineno <= base.len() {
                        pieces.push(&base[lineno - 1]);
                    }
                    cursor = lineno + 1;
                }
                index += 1;
                continue;
            }

            // A run of consecutive change lines, split into the two sides.
            let run_end = lines[index..]
                .iter()
                .position(ScopedLine::is_context)
                .map(|offset| index + offset)
                .unwrap_or(lines.len());
            let run = &lines[index..run_end];
            let base_side: Vec<&ScopedLine> = run
                .iter()
                .filter(|line| line.base_lineno(direction).is_some())
                .collect();
            let target_side: Vec<&ScopedLine> = run
                .iter()
                .filter(|line| line.base_lineno(direction).is_none())
                .collect();

            for pair_index in 0..base_side.len().max(target_side.len()) {
                let base_line = base_side.get(pair_index);
                let target_line = target_side.get(pair_index);
                let kept_base = base_line
                    .filter(|line| !is_selected(line))
                    .and_then(|line| line.base_lineno(direction))
                    .filter(|lineno| *lineno <= base.len())
                    .map(|lineno| base[lineno - 1].as_str());
                let applied_target =
                    target_line
                        .filter(|line| is_selected(line))
                        .and_then(|line| match line {
                            ScopedLine::Addition { text, .. }
                            | ScopedLine::Deletion { text, .. } => Some(text.as_str()),
                            ScopedLine::Context { .. } => None,
                        });

                // Unified diffs list removals before insertions, so applying
                // emits the kept base line first and reverting emits the
                // restored line first.
                let ordered = match direction {
                    WorktreePatchDirection::Apply => [kept_base, applied_target],
                    WorktreePatchDirection::Revert => [applied_target, kept_base],
                };
                pieces.extend(ordered.into_iter().flatten());
            }

            if let Some(last) = base_side
                .iter()
                .filter_map(|line| line.base_lineno(direction))
                .max()
            {
                cursor = last + 1;
            }
            index = run_end;
        }
    }

    copy_base_through(&mut pieces, base, &mut cursor, base.len() + 1);

    // Only the last piece may lack a newline: any earlier one that arrives
    // without one gains it, so the other side's unterminated final line landing
    // mid-file cannot glue two lines together.
    let count = pieces.len();
    let mut out = String::new();
    for (index, piece) in pieces.into_iter().enumerate() {
        out.push_str(piece);
        if !piece.ends_with('\n') && index + 1 < count {
            out.push('\n');
        }
    }
    out
}

/// The viewer's line selection, split into the two sides it targets.
///
/// Additions are matched on their new-side line number and deletions on their
/// old-side one, kept apart so an addition and a deletion that happen to share
/// a number are never confused.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SelectedLines {
    additions: HashSet<usize>,
    deletions: HashSet<usize>,
}

impl SelectedLines {
    pub(crate) fn from_pairs(pairs: &[(Option<usize>, Option<usize>)]) -> Self {
        let mut selected = Self::default();
        for (old, new) in pairs {
            match (old, new) {
                (None, Some(new)) => {
                    selected.additions.insert(*new);
                }
                (Some(old), None) => {
                    selected.deletions.insert(*old);
                }
                // A context pair carries both numbers and a `(None, None)` row
                // carries neither; neither selects a change line.
                _ => {}
            }
        }
        selected
    }

    fn contains_addition(&self, new_lineno: usize) -> bool {
        self.additions.contains(&new_lineno)
    }

    fn contains_deletion(&self, old_lineno: usize) -> bool {
        self.deletions.contains(&old_lineno)
    }

    fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.deletions.is_empty()
    }
}

// ── Repository-facing work ────────────────────────────────────────────────────

/// One file's diff between the source's two trees, plus the blob on each side.
struct FileDiffSides {
    hunks: Vec<Vec<ScopedLine>>,
    /// Blob on the diff's old side, `None` when the file was added.
    old_blob: Option<Oid>,
    /// Blob on the diff's new side, `None` when the file was deleted.
    new_blob: Option<Oid>,
}

/// Read `file_path`'s diff out of `source` as hunks of [`ScopedLine`]s.
///
/// The diff is computed with default options so hunk indices line up with the
/// `FileDiff` the viewer is displaying, which came from the same tree pair via
/// `parse_multi_file_diff`.
fn scoped_hunks(
    repo: &Repository,
    source: &WorktreePatchSource,
    file_path: &Path,
) -> Result<FileDiffSides> {
    let (from_tree, to_tree) = source.trees(repo)?;
    let diff = repo.diff_tree_to_tree(from_tree.as_ref(), to_tree.as_ref(), None)?;

    for delta_index in 0..diff.deltas().len() {
        let patch = match git2::Patch::from_diff(&diff, delta_index) {
            Ok(Some(patch)) => patch,
            _ => continue,
        };
        let old_path = patch.delta().old_file().path().map(Path::to_path_buf);
        let new_path = patch.delta().new_file().path().map(Path::to_path_buf);
        if old_path.as_deref() != Some(file_path) && new_path.as_deref() != Some(file_path) {
            continue;
        }

        let mut hunks = Vec::with_capacity(patch.num_hunks());
        for hunk_index in 0..patch.num_hunks() {
            let mut lines = Vec::new();
            for line_index in 0..patch.num_lines_in_hunk(hunk_index)? {
                let line = patch.line_in_hunk(hunk_index, line_index)?;
                let text = String::from_utf8_lossy(line.content()).to_string();
                match line.origin() {
                    ' ' => {
                        if let (Some(old), Some(new)) = (line.old_lineno(), line.new_lineno()) {
                            lines.push(ScopedLine::Context {
                                old_lineno: old as usize,
                                new_lineno: new as usize,
                            });
                        }
                    }
                    '+' => {
                        if let Some(new) = line.new_lineno() {
                            lines.push(ScopedLine::Addition {
                                new_lineno: new as usize,
                                text,
                            });
                        }
                    }
                    '-' => {
                        if let Some(old) = line.old_lineno() {
                            lines.push(ScopedLine::Deletion {
                                old_lineno: old as usize,
                                text,
                            });
                        }
                    }
                    // '=', '>' and '<' carry the "\ No newline at end of file"
                    // note, which annotates the preceding line rather than
                    // being a line of its own. The preceding line's content
                    // already lacks its terminator, so drop these.
                    _ => {}
                }
            }
            hunks.push(lines);
        }

        let old_blob = patch.delta().old_file().id();
        let new_blob = patch.delta().new_file().id();
        return Ok(FileDiffSides {
            hunks,
            old_blob: (!old_blob.is_zero()).then_some(old_blob),
            new_blob: (!new_blob.is_zero()).then_some(new_blob),
        });
    }

    anyhow::bail!(
        "{} is unchanged between {}, so there is nothing to apply or revert. Pick a file that \
         differs between them.",
        file_path.display(),
        source.label()
    )
}

fn blob_text(repo: &Repository, blob: Option<Oid>) -> Result<String> {
    match blob {
        None => Ok(String::new()),
        Some(oid) => {
            let blob = repo.find_blob(oid)?;
            Ok(String::from_utf8_lossy(blob.content()).to_string())
        }
    }
}

/// Apply or revert part of `source`'s diff for one file in `worktree_path`.
///
/// Returns the pre-operation snapshot so the caller can offer undo. Every error
/// is a sentence naming the file, the reason and what to do next.
pub fn apply_worktree_patch(
    worktree_path: &Path,
    file_path: &Path,
    source: &WorktreePatchSource,
    scope: &WorktreePatchScope,
    direction: WorktreePatchDirection,
) -> Result<WorktreePatchOutcome> {
    let repo = Repository::open(worktree_path).with_context(|| {
        format!(
            "Failed to open the repository at {}",
            worktree_path.display()
        )
    })?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "This is a bare repository, so there is no working tree to {} into.",
                direction.verb()
            )
        })?
        .to_path_buf();

    let FileDiffSides {
        hunks,
        old_blob,
        new_blob,
    } = scoped_hunks(&repo, source, file_path)?;

    let selected_hunks = match scope {
        WorktreePatchScope::File | WorktreePatchScope::Lines(_) => None,
        WorktreePatchScope::Hunk(index) => {
            if *index >= hunks.len() {
                anyhow::bail!(
                    "Hunk {} is no longer part of {}'s diff — reselect the hunk and try again.",
                    index + 1,
                    file_path.display()
                );
            }
            Some(HashSet::from([*index]))
        }
    };
    let selected_lines = match scope {
        WorktreePatchScope::File | WorktreePatchScope::Hunk(_) => None,
        WorktreePatchScope::Lines(pairs) => {
            let selected = SelectedLines::from_pairs(pairs);
            if selected.is_empty() {
                anyhow::bail!(
                    "No added or removed lines are selected, so there is nothing to {}. Select \
                     the lines you want first.",
                    direction.verb()
                );
            }
            Some(selected)
        }
    };

    let (base_blob, target_side_blob) = match direction {
        WorktreePatchDirection::Apply => (old_blob, new_blob),
        WorktreePatchDirection::Revert => (new_blob, old_blob),
    };

    let base_text = blob_text(&repo, base_blob)?;
    let base_lines = split_keeping_terminators(&base_text);
    let target_text = rewrite_side(
        &base_lines,
        &hunks,
        direction,
        selected_hunks.as_ref(),
        selected_lines.as_ref(),
    );

    if target_text == base_text {
        anyhow::bail!(
            "None of the selected lines are part of {}'s diff against {} — reselect them and \
             try again.",
            file_path.display(),
            source.label()
        );
    }

    let absolute = workdir.join(file_path);

    // The whole file moving to a side that does not have it is a deletion, not
    // an empty file. Only the file-level scope can express that.
    let deletes_file = matches!(scope, WorktreePatchScope::File)
        && target_side_blob.is_none()
        && target_text.is_empty();

    let existing = match std::fs::read(&absolute) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", absolute.display()))
        }
    };
    let existing_text = existing
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).to_string());
    let snapshot = vec![WorktreeFileSnapshot {
        path: file_path.to_path_buf(),
        contents: existing.clone(),
    }];

    if deletes_file {
        let Some(existing_text) = existing_text.as_deref() else {
            anyhow::bail!(
                "{} is already absent from the working tree, so there is nothing to {}.",
                file_path.display(),
                direction.verb()
            );
        };
        if existing_text != base_text {
            anyhow::bail!(
                "Can't {} the deletion of {}: the file has uncommitted edits that would be lost. \
                 Commit or stash them, then try again.",
                direction.verb(),
                file_path.display()
            );
        }
        std::fs::remove_file(&absolute)
            .with_context(|| format!("Failed to delete {}", absolute.display()))?;
        return Ok(WorktreePatchOutcome {
            snapshots: snapshot,
            merged_with_local_changes: false,
        });
    }

    let ours_text = existing_text.clone().unwrap_or_default();

    // Clean case: the working tree still matches the side the patch starts
    // from, so the rewrite is exact and no merge is needed.
    let merged_with_local_changes = ours_text != base_text;
    let merged_text = if merged_with_local_changes {
        match three_way_merge(&repo, file_path, &base_text, &ours_text, &target_text)? {
            Some(merged) => merged,
            None => {
                anyhow::bail!(conflict_message(&repo, file_path, scope, source, direction));
            }
        }
    } else {
        target_text
    };

    // The merge folding back to what is already on disk means the selected
    // change is already present (or already gone). Reporting that beats writing
    // the same bytes and claiming to have done something — and it is the general
    // test, since a scope covering part of a file leaves the rest of `target`
    // disagreeing with the working tree for reasons that are none of its
    // business.
    if merged_text == ours_text {
        anyhow::bail!(already_applied_message(file_path, scope, source, direction));
    }

    if let Some(parent) = absolute.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(&absolute, merged_text.as_bytes())
        .with_context(|| format!("Failed to write {}", absolute.display()))?;

    Ok(WorktreePatchOutcome {
        snapshots: snapshot,
        merged_with_local_changes,
    })
}

/// Restore files to the contents captured before an apply or revert.
pub fn restore_worktree_files(
    worktree_path: &Path,
    snapshots: &[WorktreeFileSnapshot],
) -> Result<()> {
    let repo = Repository::open(worktree_path).with_context(|| {
        format!(
            "Failed to open the repository at {}",
            worktree_path.display()
        )
    })?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("This is a bare repository, so it has no working tree."))?
        .to_path_buf();

    for snapshot in snapshots {
        let absolute = workdir.join(&snapshot.path);
        match &snapshot.contents {
            Some(bytes) => {
                if let Some(parent) = absolute.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create {}", parent.display()))?;
                }
                std::fs::write(&absolute, bytes)
                    .with_context(|| format!("Failed to restore {}", absolute.display()))?;
            }
            None => match std::fs::remove_file(&absolute) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to remove {}", absolute.display()))
                }
            },
        }
    }

    Ok(())
}

/// Three-way merge three versions of one file's text.
///
/// Returns `None` when the merge would need conflict markers. libgit2's file
/// merge works on blobs, so the three versions are written to the object
/// database first; they are unreferenced and collected by the next `git gc`.
fn three_way_merge(
    repo: &Repository,
    file_path: &Path,
    ancestor: &str,
    ours: &str,
    theirs: &str,
) -> Result<Option<String>> {
    let path_bytes = file_path.to_string_lossy().replace('\\', "/").into_bytes();
    let entry = |text: &str| -> Result<IndexEntry> {
        Ok(IndexEntry {
            ctime: IndexTime::new(0, 0),
            mtime: IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: text.len() as u32,
            id: repo.blob(text.as_bytes())?,
            flags: 0,
            flags_extended: 0,
            path: path_bytes.clone(),
        })
    };

    // Default (non-diff3) conflict style: we never keep a conflicted result, so
    // the markers only ever feed `is_automergeable()`.
    let mut options = git2::MergeFileOptions::new();
    options.style_standard(true);
    let result = repo.merge_file_from_index(
        &entry(ancestor)?,
        &entry(ours)?,
        &entry(theirs)?,
        Some(&mut options),
    )?;

    if !result.is_automergeable() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(result.content()).to_string()))
}

// ── Error messages ────────────────────────────────────────────────────────────

fn already_applied_message(
    file_path: &Path,
    scope: &WorktreePatchScope,
    source: &WorktreePatchSource,
    direction: WorktreePatchDirection,
) -> String {
    match direction {
        WorktreePatchDirection::Apply => format!(
            "{} already matches {} for {} — there is nothing to apply. Press r to revert it \
             instead.",
            file_path.display(),
            source.label(),
            scope.describe()
        ),
        WorktreePatchDirection::Revert => format!(
            "{}'s {} from {} is already absent from the working tree — there is nothing to \
             revert. Press a to apply it instead.",
            file_path.display(),
            scope.describe(),
            source.label()
        ),
    }
}

fn conflict_message(
    repo: &Repository,
    file_path: &Path,
    scope: &WorktreePatchScope,
    source: &WorktreePatchSource,
    direction: WorktreePatchDirection,
) -> String {
    let dirty = repo
        .status_file(file_path)
        .map(|status| {
            status.intersects(
                git2::Status::WT_MODIFIED
                    | git2::Status::WT_NEW
                    | git2::Status::WT_TYPECHANGE
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_NEW,
            )
        })
        .unwrap_or(false);

    if dirty {
        // A conflicting local edit: the user has uncommitted work in the same
        // place. Naming the file tells them exactly what to park.
        format!(
            "Can't {} {} of {}: your uncommitted edits to that file overlap it. Commit or stash \
             {}, then try again.",
            direction.verb(),
            scope.describe(),
            file_path.display(),
            file_path.display()
        )
    } else {
        // A context mismatch: the file is clean, so the patch does not fit
        // because the file itself has moved on since that revision.
        format!(
            "Can't {} {} of {}: the surrounding lines have changed since {}, so the patch no \
             longer fits. {} the whole file from the file menu to take that revision's version \
             wholesale.",
            direction.verb(),
            scope.describe(),
            file_path.display(),
            source.label(),
            match direction {
                WorktreePatchDirection::Apply => "Apply",
                WorktreePatchDirection::Revert => "Revert",
            }
        )
    }
}

// ── GitProject operations ─────────────────────────────────────────────────────

use gpui::{AsyncApp, Context, Task, WeakEntity};

use super::refresh::gather_refresh_data_lightweight_cached;
use super::{GitProject, GitProjectEvent, RefreshData};
use crate::types::GitOperationKind;

impl GitProject {
    /// Apply or revert part of another revision's diff in `worktree_path`.
    ///
    /// The heavy work — resolving trees, rewriting lines, merging and writing —
    /// happens on the background executor; only the refresh snapshot and the
    /// events come back to the UI thread.
    #[allow(clippy::too_many_arguments)]
    pub fn patch_worktree_at(
        &mut self,
        file_path: &Path,
        source: WorktreePatchSource,
        scope: WorktreePatchScope,
        direction: WorktreePatchDirection,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let file_path = file_path.to_path_buf();
        let task_file_path = file_path.clone();
        let task_worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
        let branch_name = self.head_branch.clone();
        let kind = match direction {
            WorktreePatchDirection::Apply => GitOperationKind::ApplyToWorktree,
            WorktreePatchDirection::Revert => GitOperationKind::RevertInWorktree,
        };
        let scope_text = scope.describe();
        let source_label = source.label();
        let operation_id = self.begin_operation(
            kind,
            format!(
                "{} {} of {} from {}...",
                direction.present_participle(),
                scope_text,
                file_path.display(),
                source_label
            ),
            None,
            branch_name.clone(),
            cx,
        );
        let task_source = source.clone();
        let task_scope = scope.clone();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(WorktreePatchOutcome, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let outcome = apply_worktree_patch(
                        &task_worktree_path,
                        &task_file_path,
                        &task_source,
                        &task_scope,
                        direction,
                    )?;
                    let data = gather_refresh_data_lightweight_cached(
                        &refresh_repo_path,
                        commit_limit,
                        &worktree_cache,
                        author_filter.as_deref(),
                    )?;
                    Ok((outcome, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((outcome, data)) => {
                            this.apply_refresh_data(data);
                            let mut summary = format!(
                                "{} {} of {} from {}",
                                direction.past_tense(),
                                scope_text,
                                file_path.display(),
                                source_label
                            );
                            if outcome.merged_with_local_changes {
                                summary.push_str(
                                    " — merged around your uncommitted edits to that file",
                                );
                            }
                            this.complete_op(
                                operation_id,
                                kind,
                                summary.clone(),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            if snapshots_fit_undo_stack(&outcome.snapshots) {
                                cx.emit(GitProjectEvent::WorktreePatchApplied {
                                    label: summary,
                                    snapshots: outcome.snapshots,
                                });
                            }
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                kind,
                                format!("{} failed", kind.display_name()),
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

    /// Restore working-tree files to a snapshot taken before an apply or revert.
    pub fn restore_worktree_files_at(
        &mut self,
        snapshots: Vec<WorktreeFileSnapshot>,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let task_worktree_path = worktree_path.to_path_buf();
        let refresh_repo_path = self.repo_path.clone();
        let worktree_cache = self.worktree_status_cache.clone();
        let author_filter = self.commit_author_filter.clone();
        let commit_limit = self.commit_limit;
        let branch_name = self.head_branch.clone();
        let file_count = snapshots.len();
        let operation_id = self.begin_operation(
            GitOperationKind::Discard,
            format!(
                "Restoring {} file{}...",
                file_count,
                if file_count == 1 { "" } else { "s" }
            ),
            None,
            branch_name.clone(),
            cx,
        );

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    restore_worktree_files(&task_worktree_path, &snapshots)?;
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
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Discard,
                                format!(
                                    "Restored {} file{}",
                                    file_count,
                                    if file_count == 1 { "" } else { "s" }
                                ),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Discard,
                                "Restore failed",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(old: usize, new: usize) -> ScopedLine {
        ScopedLine::Context {
            old_lineno: old,
            new_lineno: new,
        }
    }

    fn addition(new: usize, text: &str) -> ScopedLine {
        ScopedLine::Addition {
            new_lineno: new,
            text: text.to_string(),
        }
    }

    fn deletion(old: usize, text: &str) -> ScopedLine {
        ScopedLine::Deletion {
            old_lineno: old,
            text: text.to_string(),
        }
    }

    // ── split_keeping_terminators ─────────────────────────────────

    #[test]
    fn split_keeps_line_terminators() {
        assert_eq!(split_keeping_terminators("a\nb\n"), vec!["a\n", "b\n"]);
    }

    #[test]
    fn split_keeps_a_final_line_without_a_newline() {
        assert_eq!(split_keeping_terminators("a\nb"), vec!["a\n", "b"]);
    }

    #[test]
    fn split_of_empty_text_has_no_lines() {
        assert!(split_keeping_terminators("").is_empty());
    }

    // ── rewrite_side ──────────────────────────────────────────────

    fn rewrite(
        base: &str,
        hunks: &[Vec<ScopedLine>],
        direction: WorktreePatchDirection,
        selected_hunks: Option<&HashSet<usize>>,
        selected_lines: Option<&SelectedLines>,
    ) -> String {
        rewrite_side(
            &split_keeping_terminators(base),
            hunks,
            direction,
            selected_hunks,
            selected_lines,
        )
    }

    /// `b` → `B2` as one hunk of a three-line file.
    fn substitution_hunk() -> Vec<Vec<ScopedLine>> {
        vec![vec![
            context(1, 1),
            deletion(2, "b\n"),
            addition(2, "B2\n"),
            context(3, 3),
        ]]
    }

    #[test]
    fn applying_a_substitution_replaces_the_base_line() {
        let hunks = substitution_hunk();
        let out = rewrite(
            "a\nb\nc\n",
            &hunks,
            WorktreePatchDirection::Apply,
            None,
            None,
        );
        assert_eq!(out, "a\nB2\nc\n");
    }

    #[test]
    fn reverting_a_substitution_restores_the_old_line() {
        let hunks = substitution_hunk();
        let out = rewrite(
            "a\nB2\nc\n",
            &hunks,
            WorktreePatchDirection::Revert,
            None,
            None,
        );
        assert_eq!(out, "a\nb\nc\n");
    }

    #[test]
    fn apply_then_revert_round_trips() {
        let hunks = substitution_hunk();
        let original = "a\nb\nc\n";
        let applied = rewrite(original, &hunks, WorktreePatchDirection::Apply, None, None);
        let reverted = rewrite(&applied, &hunks, WorktreePatchDirection::Revert, None, None);
        assert_eq!(reverted, original);
    }

    #[test]
    fn a_hunk_scope_leaves_other_hunks_alone() {
        // Two independent substitutions, far enough apart to be separate hunks.
        let hunks = vec![
            vec![
                context(1, 1),
                deletion(2, "b\n"),
                addition(2, "B2\n"),
                context(3, 3),
            ],
            vec![
                context(5, 5),
                deletion(6, "f\n"),
                addition(6, "F2\n"),
                context(7, 7),
            ],
        ];
        let only_second = HashSet::from([1usize]);
        let out = rewrite(
            "a\nb\nc\nd\ne\nf\ng\n",
            &hunks,
            WorktreePatchDirection::Apply,
            Some(&only_second),
            None,
        );
        assert_eq!(out, "a\nb\nc\nd\ne\nF2\ng\n");
    }

    /// Two adjacent substitutions with no context between them, which is how
    /// git emits them: both removals, then both insertions.
    fn adjacent_substitutions() -> Vec<Vec<ScopedLine>> {
        vec![vec![
            context(1, 1),
            deletion(2, "b\n"),
            deletion(3, "c\n"),
            addition(2, "B2\n"),
            addition(3, "C2\n"),
            context(4, 4),
        ]]
    }

    #[test]
    fn a_line_scope_rewrites_only_the_selected_lines() {
        let hunks = adjacent_substitutions();
        // The second substitution: old line 3 removed, new line 3 added.
        let selected = SelectedLines::from_pairs(&[(Some(3), None), (None, Some(3))]);
        let out = rewrite(
            "a\nb\nc\nd\n",
            &hunks,
            WorktreePatchDirection::Apply,
            None,
            Some(&selected),
        );
        assert_eq!(out, "a\nb\nC2\nd\n");
    }

    #[test]
    fn a_line_scope_keeps_an_unselected_line_in_its_own_place() {
        // Selecting the *first* substitution of a run is where reading the
        // unified rows in order goes wrong: it would emit the kept `c` before
        // the applied `B2`.
        let hunks = adjacent_substitutions();
        let selected = SelectedLines::from_pairs(&[(Some(2), None), (None, Some(2))]);
        let out = rewrite(
            "a\nb\nc\nd\n",
            &hunks,
            WorktreePatchDirection::Apply,
            None,
            Some(&selected),
        );
        assert_eq!(out, "a\nB2\nc\nd\n");
    }

    #[test]
    fn reverting_the_first_of_two_adjacent_substitutions_keeps_the_order() {
        let hunks = adjacent_substitutions();
        let selected = SelectedLines::from_pairs(&[(Some(2), None), (None, Some(2))]);
        let out = rewrite(
            "a\nB2\nC2\nd\n",
            &hunks,
            WorktreePatchDirection::Revert,
            None,
            Some(&selected),
        );
        assert_eq!(out, "a\nb\nC2\nd\n");
    }

    #[test]
    fn selecting_only_an_addition_leaves_the_deletion_in_place() {
        let hunks = substitution_hunk();
        let selected = SelectedLines::from_pairs(&[(None, Some(2))]);
        let out = rewrite(
            "a\nb\nc\n",
            &hunks,
            WorktreePatchDirection::Apply,
            None,
            Some(&selected),
        );
        assert_eq!(out, "a\nb\nB2\nc\n");
    }

    #[test]
    fn selecting_only_a_deletion_drops_the_line() {
        let hunks = substitution_hunk();
        let selected = SelectedLines::from_pairs(&[(Some(2), None)]);
        let out = rewrite(
            "a\nb\nc\n",
            &hunks,
            WorktreePatchDirection::Apply,
            None,
            Some(&selected),
        );
        assert_eq!(out, "a\nc\n");
    }

    #[test]
    fn selection_pairs_ignore_context_and_empty_rows() {
        let selected = SelectedLines::from_pairs(&[(Some(1), Some(1)), (None, None)]);
        assert!(selected.is_empty());
    }

    #[test]
    fn pure_insertion_at_the_top_of_a_file_lands_first() {
        let hunks = vec![vec![addition(1, "new\n"), context(1, 2)]];
        let out = rewrite("a\n", &hunks, WorktreePatchDirection::Apply, None, None);
        assert_eq!(out, "new\na\n");
    }

    #[test]
    fn insertion_without_a_trailing_newline_gains_one_when_lines_follow() {
        // The other side's last line becoming a middle line must not glue two
        // lines together.
        let hunks = vec![vec![context(1, 1), addition(2, "tail"), context(2, 3)]];
        let out = rewrite("a\nb\n", &hunks, WorktreePatchDirection::Apply, None, None);
        assert_eq!(out, "a\ntail\nb\n");
    }

    #[test]
    fn a_final_line_keeps_its_missing_newline() {
        let hunks = vec![vec![context(1, 1), deletion(2, "b"), addition(2, "B2")]];
        let out = rewrite("a\nb", &hunks, WorktreePatchDirection::Apply, None, None);
        assert_eq!(out, "a\nB2");
    }

    #[test]
    fn deleting_every_line_yields_empty_text() {
        let hunks = vec![vec![deletion(1, "a\n"), deletion(2, "b\n")]];
        let out = rewrite("a\nb\n", &hunks, WorktreePatchDirection::Apply, None, None);
        assert_eq!(out, "");
    }

    #[test]
    fn an_unselected_hunk_contributes_its_base_lines_verbatim() {
        let hunks = substitution_hunk();
        let nothing = HashSet::new();
        let out = rewrite(
            "a\nb\nc\n",
            &hunks,
            WorktreePatchDirection::Apply,
            Some(&nothing),
            None,
        );
        assert_eq!(out, "a\nb\nc\n");
    }

    // ── snapshot cap ──────────────────────────────────────────────

    #[test]
    fn small_snapshots_fit_the_undo_stack() {
        let snapshots = vec![WorktreeFileSnapshot {
            path: PathBuf::from("a.txt"),
            contents: Some(vec![0; 1024]),
        }];
        assert!(snapshots_fit_undo_stack(&snapshots));
    }

    #[test]
    fn oversized_snapshots_do_not_fit_the_undo_stack() {
        let snapshots = vec![WorktreeFileSnapshot {
            path: PathBuf::from("a.txt"),
            contents: Some(vec![0; MAX_UNDO_SNAPSHOT_BYTES + 1]),
        }];
        assert!(!snapshots_fit_undo_stack(&snapshots));
    }

    #[test]
    fn a_deleted_file_snapshot_costs_nothing() {
        let snapshots = vec![WorktreeFileSnapshot {
            path: PathBuf::from("a.txt"),
            contents: None,
        }];
        assert!(snapshots_fit_undo_stack(&snapshots));
    }

    // ── labels ────────────────────────────────────────────────────

    #[test]
    fn scope_descriptions_read_as_prose() {
        assert_eq!(WorktreePatchScope::File.describe(), "all changes");
        assert_eq!(WorktreePatchScope::Hunk(0).describe(), "hunk 1");
        assert_eq!(
            WorktreePatchScope::Lines(vec![(None, Some(1))]).describe(),
            "1 line"
        );
        assert_eq!(
            WorktreePatchScope::Lines(vec![(None, Some(1)), (Some(2), None)]).describe(),
            "2 lines"
        );
    }

    #[test]
    fn a_commit_source_is_labelled_by_its_short_oid() {
        let oid = Oid::from_str("1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(WorktreePatchSource::Commit(oid).label(), "1234567");
    }

    #[test]
    fn a_compare_source_is_labelled_by_both_endpoints() {
        let source = WorktreePatchSource::Compare {
            from: "main".to_string(),
            to: "feature".to_string(),
        };
        assert_eq!(source.label(), "main...feature");
    }

    #[test]
    fn direction_inverts() {
        assert_eq!(
            WorktreePatchDirection::Apply.inverse(),
            WorktreePatchDirection::Revert
        );
        assert_eq!(
            WorktreePatchDirection::Revert.inverse(),
            WorktreePatchDirection::Apply
        );
    }
}

// ── Integration tests against real repositories ───────────────────────────────
//
// Every assertion here reads the file back off disk. A test that only checked
// `is_ok()` would pass for a patch applied in the wrong direction, which is the
// one mistake this feature cannot afford.
#[cfg(test)]
mod worktree_patch_integration_tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        path: PathBuf,
        repo: Repository,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let path = dir.path().to_path_buf();
            let repo = Repository::init(&path).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test").unwrap();
            config.set_str("user.email", "t@t.com").unwrap();
            // Keep line endings byte-exact so content assertions hold on
            // Windows, where autocrlf would rewrite what we read back.
            config.set_bool("core.autocrlf", false).unwrap();
            drop(config);
            Self {
                _dir: dir,
                path,
                repo,
            }
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.path.join(name), contents).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.path.join(name)).unwrap()
        }

        fn commit(&self, message: &str, files: &[&str]) -> Oid {
            let signature = git2::Signature::now("Test", "t@t.com").unwrap();
            let mut index = self.repo.index().unwrap();
            for file in files {
                index.add_path(Path::new(file)).unwrap();
            }
            index.write().unwrap();
            let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
            let parents = match self.repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
                Some(parent) => vec![parent],
                None => Vec::new(),
            };
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            self.repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &parent_refs,
                )
                .unwrap()
        }

        fn branch(&self, name: &str) {
            let head = self.repo.head().unwrap().peel_to_commit().unwrap();
            self.repo.branch(name, &head, true).unwrap();
        }

        fn checkout(&self, name: &str) {
            let reference = format!("refs/heads/{name}");
            let object = self.repo.revparse_single(&reference).unwrap();
            self.repo.checkout_tree(&object, None).unwrap();
            self.repo.set_head(&reference).unwrap();
        }

        fn head_branch_name(&self) -> String {
            self.repo.head().unwrap().shorthand().unwrap().to_string()
        }
    }

    /// Numbered lines `l1..l20`, with `substitutions` replacing the given
    /// 1-based line numbers.
    fn numbered_lines(substitutions: &[(usize, &str)]) -> String {
        (1..=20)
            .map(
                |lineno| match substitutions.iter().find(|(at, _)| *at == lineno) {
                    Some((_, text)) => format!("{text}\n"),
                    None => format!("l{lineno}\n"),
                },
            )
            .collect()
    }

    /// A repo whose HEAD commit changes lines 2 and 15 of a 20-line `f.txt`.
    /// The two edits are more than twice the default context apart, so the diff
    /// against the parent really has two hunks rather than one merged one.
    fn two_hunk_commit() -> (Fixture, Oid) {
        let fixture = Fixture::new();
        fixture.write("f.txt", &numbered_lines(&[]));
        fixture.commit("base", &["f.txt"]);
        fixture.write("f.txt", &numbered_lines(&[(2, "L2X"), (15, "L15X")]));
        let oid = fixture.commit("two edits", &["f.txt"]);
        // Leave the working tree matching the commit's parent so an apply has
        // somewhere to land.
        fixture.write("f.txt", &numbered_lines(&[]));
        (fixture, oid)
    }

    fn apply(
        fixture: &Fixture,
        file: &str,
        source: &WorktreePatchSource,
        scope: WorktreePatchScope,
        direction: WorktreePatchDirection,
    ) -> Result<WorktreePatchOutcome> {
        apply_worktree_patch(&fixture.path, Path::new(file), source, &scope, direction)
    }

    // ── hunk apply / revert on a clean tree ───────────────────────

    #[test]
    fn applying_a_commits_hunk_writes_only_that_hunk_to_disk() {
        let (fixture, oid) = two_hunk_commit();
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(2, "L2X")]),
            "hunk 0 changes line 2 and must leave line 15 alone"
        );
    }

    #[test]
    fn applying_the_second_hunk_targets_the_second_change() {
        let (fixture, oid) = two_hunk_commit();
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(1),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), numbered_lines(&[(15, "L15X")]));
    }

    #[test]
    fn reverting_a_hunk_restores_the_parents_line() {
        let (fixture, oid) = two_hunk_commit();
        // Start from the committed content so there is something to revert.
        fixture.write("f.txt", &numbered_lines(&[(2, "L2X"), (15, "L15X")]));
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(15, "L15X")]),
            "only the first hunk should be rolled back"
        );
    }

    #[test]
    fn applying_then_reverting_the_same_hunk_returns_the_original_bytes() {
        let (fixture, oid) = two_hunk_commit();
        let original = fixture.read("f.txt");
        let source = WorktreePatchSource::Commit(oid);
        apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), original);
    }

    // ── whole file and line subsets ───────────────────────────────

    #[test]
    fn applying_the_whole_file_takes_every_hunk() {
        let (fixture, oid) = two_hunk_commit();
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(2, "L2X"), (15, "L15X")])
        );
    }

    #[test]
    fn applying_a_line_subset_rewrites_only_the_selected_lines() {
        let fixture = Fixture::new();
        fixture.write("f.txt", "a\nb\nc\nd\n");
        fixture.commit("base", &["f.txt"]);
        fixture.write("f.txt", "a\nB2\nC2\nd\n");
        let oid = fixture.commit("two lines", &["f.txt"]);
        fixture.write("f.txt", "a\nb\nc\nd\n");

        // Select only the `c` → `C2` substitution: old line 3 and new line 3.
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Lines(vec![(Some(3), None), (None, Some(3))]),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), "a\nb\nC2\nd\n");
    }

    #[test]
    fn reverting_a_line_subset_restores_only_the_selected_lines() {
        let fixture = Fixture::new();
        fixture.write("f.txt", "a\nb\nc\nd\n");
        fixture.commit("base", &["f.txt"]);
        fixture.write("f.txt", "a\nB2\nC2\nd\n");
        let oid = fixture.commit("two lines", &["f.txt"]);

        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Lines(vec![(Some(3), None), (None, Some(3))]),
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), "a\nB2\nc\nd\n");
    }

    // ── local edits ───────────────────────────────────────────────

    #[test]
    fn applying_over_an_unrelated_local_edit_keeps_both_changes() {
        // The hunk changes line 2, so its trailing context runs to line 5. A
        // local edit on line 5 is inside that context window, which is what
        // makes a plain context-matching apply (libgit2's, and `git apply`'s)
        // fail here. The three-way merge keeps both.
        let (fixture, oid) = two_hunk_commit();
        fixture.write("f.txt", &numbered_lines(&[(5, "LOCAL5")]));

        let outcome = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();

        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(2, "L2X"), (5, "LOCAL5")]),
            "the applied hunk and the local edit must both survive"
        );
        assert!(
            outcome.merged_with_local_changes,
            "the caller needs to know a merge happened so it can say so"
        );
    }

    #[test]
    fn libgit2s_own_apply_refuses_the_case_the_merge_handles() {
        // Pins the libgit2 constraint the module docs state: the same hunk, as a
        // patch, against the same working tree that
        // `applying_over_an_unrelated_local_edit_keeps_both_changes` handles.
        let (fixture, _) = two_hunk_commit();
        fixture.write("f.txt", &numbered_lines(&[(5, "LOCAL5")]));
        let patch = "diff --git a/f.txt b/f.txt\n\
                     --- a/f.txt\n\
                     +++ b/f.txt\n\
                     @@ -1,5 +1,5 @@\n\
                     \x20l1\n\
                     -l2\n\
                     +L2X\n\
                     \x20l3\n\
                     \x20l4\n\
                     \x20l5\n";
        let diff = git2::Diff::from_buffer(patch.as_bytes()).unwrap();
        fixture
            .repo
            .apply(&diff, git2::ApplyLocation::WorkDir, None)
            .expect_err("libgit2 matches context literally, and line 5 no longer matches");
        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(5, "LOCAL5")]),
            "and it leaves the file alone, so there is nothing to fall back from"
        );
    }

    #[test]
    fn applying_over_an_overlapping_local_edit_is_refused_and_writes_nothing() {
        let (fixture, oid) = two_hunk_commit();
        fixture.write("f.txt", &numbered_lines(&[(2, "MINE")]));

        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .expect_err("an overlapping edit must not be silently merged");

        let message = error.to_string();
        assert!(
            message.contains("uncommitted edits"),
            "expected the local-edit message, got: {message}"
        );
        assert!(
            message.contains("Commit or stash"),
            "the message must say what to do, got: {message}"
        );
        assert_eq!(
            fixture.read("f.txt"),
            numbered_lines(&[(2, "MINE")]),
            "a refused apply must leave the file untouched"
        );
    }

    #[test]
    fn a_context_mismatch_on_a_clean_file_reports_the_revision_moving_on() {
        // Commit a change, then commit again so the hunk's context no longer
        // exists anywhere in the working tree. The file itself is clean.
        let fixture = Fixture::new();
        fixture.write("f.txt", "a\nb\nc\n");
        fixture.commit("base", &["f.txt"]);
        fixture.write("f.txt", "a\nB2\nc\n");
        let target = fixture.commit("change b", &["f.txt"]);
        fixture.write("f.txt", "totally\ndifferent\nfile\n");
        fixture.commit("rewrite", &["f.txt"]);

        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(target),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .expect_err("the patch cannot fit");

        let message = error.to_string();
        assert!(
            message.contains("surrounding lines have changed"),
            "expected the context-mismatch message, got: {message}"
        );
        assert_eq!(
            fixture.read("f.txt"),
            "totally\ndifferent\nfile\n",
            "a refused apply must leave the file untouched"
        );
    }

    // ── already applied ───────────────────────────────────────────

    #[test]
    fn applying_an_already_applied_hunk_is_a_clear_error_not_a_silent_mess() {
        let (fixture, oid) = two_hunk_commit();
        let source = WorktreePatchSource::Commit(oid);
        apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        let after_first = fixture.read("f.txt");

        let error = apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .expect_err("a second apply has nothing to do");

        let message = error.to_string();
        assert!(
            message.contains("nothing to apply"),
            "expected the already-applied message, got: {message}"
        );
        assert!(
            message.contains("press r to revert") || message.contains("Press r to revert"),
            "the message must point at the way out, got: {message}"
        );
        assert_eq!(
            fixture.read("f.txt"),
            after_first,
            "the second attempt must not double-apply"
        );
    }

    #[test]
    fn reverting_something_already_absent_is_a_clear_error() {
        let (fixture, oid) = two_hunk_commit();
        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Revert,
        )
        .expect_err("the change was never applied");

        let message = error.to_string();
        assert!(
            message.contains("nothing to \nrevert") || message.contains("nothing to revert"),
            "expected the already-reverted message, got: {message}"
        );
    }

    // ── cross-branch compare ──────────────────────────────────────

    /// Two branches that differ in `f.txt`, with `main` checked out.
    fn two_branch_repo() -> Fixture {
        let fixture = Fixture::new();
        fixture.write("f.txt", "shared\nours\nshared2\n");
        fixture.commit("base", &["f.txt"]);
        let base_branch = fixture.head_branch_name();
        fixture.branch("feature");
        fixture.checkout("feature");
        fixture.write("f.txt", "shared\ntheirs\nshared2\n");
        fixture.commit("feature edit", &["f.txt"]);
        fixture.checkout(&base_branch);
        fixture.write("f.txt", "shared\nours\nshared2\n");
        fixture
    }

    #[test]
    fn applying_a_cross_branch_hunk_brings_the_other_branchs_content_in() {
        let fixture = two_branch_repo();
        let base_branch = fixture.head_branch_name();
        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Compare {
                from: base_branch,
                to: "feature".to_string(),
            },
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(
            fixture.read("f.txt"),
            "shared\ntheirs\nshared2\n",
            "applying main→feature must pull feature's line into the working tree"
        );
    }

    #[test]
    fn the_compare_direction_decides_which_branch_wins() {
        // Same two branches, endpoints swapped. Applying feature→main while on
        // main is a no-op, which proves the patch really is generated from the
        // endpoints and not from index→workdir.
        let fixture = two_branch_repo();
        let base_branch = fixture.head_branch_name();
        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Compare {
                from: "feature".to_string(),
                to: base_branch,
            },
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .expect_err("the working tree already holds main's content");
        assert!(
            error.to_string().contains("nothing to apply"),
            "got: {error}"
        );
        assert_eq!(fixture.read("f.txt"), "shared\nours\nshared2\n");
    }

    #[test]
    fn reverting_a_cross_branch_hunk_restores_this_branchs_content() {
        let fixture = two_branch_repo();
        let base_branch = fixture.head_branch_name();
        let source = WorktreePatchSource::Compare {
            from: base_branch,
            to: "feature".to_string(),
        };
        apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        apply(
            &fixture,
            "f.txt",
            &source,
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), "shared\nours\nshared2\n");
    }

    #[test]
    fn a_compare_endpoint_that_does_not_resolve_names_itself() {
        let fixture = two_branch_repo();
        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Compare {
                from: "no-such-branch".to_string(),
                to: "feature".to_string(),
            },
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .expect_err("the endpoint does not exist");
        assert!(error.to_string().contains("no-such-branch"), "got: {error}");
    }

    // ── file creation and deletion ────────────────────────────────

    #[test]
    fn applying_a_commit_that_added_a_file_creates_it() {
        let fixture = Fixture::new();
        fixture.write("keep.txt", "x\n");
        fixture.commit("base", &["keep.txt"]);
        fixture.write("added.txt", "one\ntwo\n");
        let oid = fixture.commit("add a file", &["added.txt"]);
        std::fs::remove_file(fixture.path.join("added.txt")).unwrap();

        apply(
            &fixture,
            "added.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(fixture.read("added.txt"), "one\ntwo\n");
    }

    #[test]
    fn reverting_a_commit_that_added_a_file_deletes_it() {
        let fixture = Fixture::new();
        fixture.write("keep.txt", "x\n");
        fixture.commit("base", &["keep.txt"]);
        fixture.write("added.txt", "one\ntwo\n");
        let oid = fixture.commit("add a file", &["added.txt"]);

        apply(
            &fixture,
            "added.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert!(
            !fixture.path.join("added.txt").exists(),
            "reverting the addition of a file should remove it"
        );
    }

    #[test]
    fn a_file_unchanged_by_the_source_is_refused_by_name() {
        let (fixture, oid) = two_hunk_commit();
        fixture.write("other.txt", "untouched\n");
        let error = apply(
            &fixture,
            "other.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .expect_err("that file is not part of the commit");
        assert!(error.to_string().contains("other.txt"), "got: {error}");
    }

    #[test]
    fn a_hunk_index_past_the_end_is_refused() {
        let (fixture, oid) = two_hunk_commit();
        let error = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::Hunk(9),
            WorktreePatchDirection::Apply,
        )
        .expect_err("there is no tenth hunk");
        assert!(
            error.to_string().contains("reselect the hunk"),
            "got: {error}"
        );
    }

    // ── undo snapshots ────────────────────────────────────────────

    #[test]
    fn the_snapshot_captures_the_bytes_the_apply_overwrote() {
        let (fixture, oid) = two_hunk_commit();
        let before = fixture.read("f.txt");
        let outcome = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .unwrap();

        assert_eq!(outcome.snapshots.len(), 1);
        assert_eq!(outcome.snapshots[0].path, PathBuf::from("f.txt"));
        assert_eq!(
            outcome.snapshots[0].contents.as_deref(),
            Some(before.as_bytes())
        );
    }

    #[test]
    fn restoring_a_snapshot_puts_the_original_bytes_back() {
        let (fixture, oid) = two_hunk_commit();
        let before = fixture.read("f.txt");
        let outcome = apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_ne!(fixture.read("f.txt"), before);

        restore_worktree_files(&fixture.path, &outcome.snapshots).unwrap();
        assert_eq!(fixture.read("f.txt"), before);
    }

    #[test]
    fn restoring_an_absent_snapshot_deletes_the_created_file() {
        let fixture = Fixture::new();
        fixture.write("keep.txt", "x\n");
        fixture.commit("base", &["keep.txt"]);
        fixture.write("added.txt", "one\n");
        let oid = fixture.commit("add", &["added.txt"]);
        std::fs::remove_file(fixture.path.join("added.txt")).unwrap();

        let outcome = apply(
            &fixture,
            "added.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert!(fixture.path.join("added.txt").exists());
        assert_eq!(outcome.snapshots[0].contents, None);

        restore_worktree_files(&fixture.path, &outcome.snapshots).unwrap();
        assert!(!fixture.path.join("added.txt").exists());
    }

    #[test]
    fn restoring_a_snapshot_of_a_deleted_file_recreates_it() {
        let fixture = Fixture::new();
        fixture.write("keep.txt", "x\n");
        fixture.commit("base", &["keep.txt"]);
        fixture.write("added.txt", "one\ntwo\n");
        let oid = fixture.commit("add", &["added.txt"]);

        let outcome = apply(
            &fixture,
            "added.txt",
            &WorktreePatchSource::Commit(oid),
            WorktreePatchScope::File,
            WorktreePatchDirection::Revert,
        )
        .unwrap();
        assert!(!fixture.path.join("added.txt").exists());

        restore_worktree_files(&fixture.path, &outcome.snapshots).unwrap();
        assert_eq!(fixture.read("added.txt"), "one\ntwo\n");
    }

    // ── stash entries travel the commit path ──────────────────────

    #[test]
    fn a_stash_entrys_hunk_applies_like_any_other_commit() {
        let fixture = Fixture::new();
        fixture.write("f.txt", "a\nb\nc\n");
        fixture.commit("base", &["f.txt"]);
        fixture.write("f.txt", "a\nSTASHED\nc\n");
        let mut repo = Repository::open(&fixture.path).unwrap();
        let signature = git2::Signature::now("Test", "t@t.com").unwrap();
        let stash_oid = repo
            .stash_save(&signature, "wip", Some(git2::StashFlags::DEFAULT))
            .unwrap();
        assert_eq!(fixture.read("f.txt"), "a\nb\nc\n", "stash reset the file");

        apply(
            &fixture,
            "f.txt",
            &WorktreePatchSource::Commit(stash_oid),
            WorktreePatchScope::Hunk(0),
            WorktreePatchDirection::Apply,
        )
        .unwrap();
        assert_eq!(fixture.read("f.txt"), "a\nSTASHED\nc\n");
    }
}
