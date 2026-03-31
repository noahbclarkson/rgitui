use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Information about a Git signature (author or committer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
}

/// A reference label attached to a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefLabel {
    Head,
    LocalBranch(String),
    RemoteBranch(String),
    Tag(String),
}

impl RefLabel {
    pub fn display_name(&self) -> &str {
        match self {
            RefLabel::Head => "HEAD",
            RefLabel::LocalBranch(name) => name,
            RefLabel::RemoteBranch(name) => name,
            RefLabel::Tag(name) => name,
        }
    }
}

/// Information about a single commit.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub oid: git2::Oid,
    pub short_id: String,
    pub summary: String,
    pub message: String,
    pub author: Signature,
    pub committer: Signature,
    pub co_authors: Vec<Signature>,
    pub time: DateTime<Utc>,
    pub parent_oids: Vec<git2::Oid>,
    pub refs: Vec<RefLabel>,
    /// Whether this commit has a GPG signature (gpgsig header present).
    pub is_signed: bool,
}

/// Information about a branch.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchInfo {
    pub name: String,
    pub is_head: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub tip_oid: Option<git2::Oid>,
}

/// Information about a tag.
#[derive(Debug, Clone, PartialEq)]
pub struct TagInfo {
    pub name: String,
    pub oid: git2::Oid,
    pub message: Option<String>,
}

/// Information about a remote.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteInfo {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}

/// Information about a worktree attached to this repository.
#[derive(Debug, Clone, PartialEq)]
pub struct WorktreeInfo {
    /// The worktree name (directory name or custom name).
    pub name: String,
    /// Absolute path to the worktree working directory.
    pub path: PathBuf,
    /// Whether the worktree is locked (e.g. by a running operation).
    pub is_locked: bool,
    /// Whether this is the current worktree (the main repository).
    pub is_current: bool,
    /// The branch currently checked out in this worktree, if any.
    pub branch: Option<String>,
    /// OID of the HEAD commit in this worktree.
    pub head_oid: Option<git2::Oid>,
}

/// Information about a stash entry.
#[derive(Debug, Clone, PartialEq)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub oid: git2::Oid,
}

/// Status of a file in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Untracked,
    Conflicted,
}

impl FileChangeKind {
    pub fn short_code(&self) -> &'static str {
        match self {
            FileChangeKind::Added => "A",
            FileChangeKind::Modified => "M",
            FileChangeKind::Deleted => "D",
            FileChangeKind::Renamed => "R",
            FileChangeKind::Copied => "C",
            FileChangeKind::TypeChange => "T",
            FileChangeKind::Untracked => "?",
            FileChangeKind::Conflicted => "!",
        }
    }
}

/// A file change in the working tree or staging area.
#[derive(Debug, Clone, PartialEq)]
pub struct FileStatus {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    pub old_path: Option<PathBuf>,
    /// Number of lines added in this file change.
    pub additions: usize,
    /// Number of lines deleted in this file change.
    pub deletions: usize,
}

/// Summary of all working tree changes.
#[derive(Debug, Clone, Default)]
pub struct WorkingTreeStatus {
    pub staged: Vec<FileStatus>,
    pub unstaged: Vec<FileStatus>,
}

/// A hunk in a diff.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff hunk.
#[derive(Debug, Clone)]
pub enum DiffLine {
    Context(String),
    Addition(String),
    Deletion(String),
}

/// A complete file diff.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
    pub kind: FileChangeKind,
}

/// A complete commit diff (all files).
#[derive(Debug, Clone)]
pub struct CommitDiff {
    pub files: Vec<FileDiff>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

/// The current state of the repository (normal, mid-merge, mid-rebase, etc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Clean,
    Merge,
    Revert,
    RevertSequence,
    CherryPick,
    CherryPickSequence,
    Bisect,
    Rebase,
    RebaseInteractive,
    RebaseMerge,
    ApplyMailbox,
    ApplyMailboxOrRebase,
}

impl RepoState {
    /// Convert from git2::RepositoryState.
    pub fn from_git2(state: git2::RepositoryState) -> Self {
        match state {
            git2::RepositoryState::Clean => RepoState::Clean,
            git2::RepositoryState::Merge => RepoState::Merge,
            git2::RepositoryState::Revert => RepoState::Revert,
            git2::RepositoryState::RevertSequence => RepoState::RevertSequence,
            git2::RepositoryState::CherryPick => RepoState::CherryPick,
            git2::RepositoryState::CherryPickSequence => RepoState::CherryPickSequence,
            git2::RepositoryState::Bisect => RepoState::Bisect,
            git2::RepositoryState::Rebase => RepoState::Rebase,
            git2::RepositoryState::RebaseInteractive => RepoState::RebaseInteractive,
            git2::RepositoryState::RebaseMerge => RepoState::RebaseMerge,
            git2::RepositoryState::ApplyMailbox => RepoState::ApplyMailbox,
            git2::RepositoryState::ApplyMailboxOrRebase => RepoState::ApplyMailboxOrRebase,
        }
    }

    pub fn is_clean(&self) -> bool {
        matches!(self, RepoState::Clean)
    }

    /// Human-readable label for the repo state.
    pub fn label(&self) -> &'static str {
        match self {
            RepoState::Clean => "Clean",
            RepoState::Merge => "Merging",
            RepoState::Revert => "Reverting",
            RepoState::RevertSequence => "Reverting",
            RepoState::CherryPick => "Cherry-picking",
            RepoState::CherryPickSequence => "Cherry-picking",
            RepoState::Bisect => "Bisecting",
            RepoState::Rebase => "Rebasing",
            RepoState::RebaseInteractive => "Rebasing (interactive)",
            RepoState::RebaseMerge => "Rebasing",
            RepoState::ApplyMailbox => "Applying patches",
            RepoState::ApplyMailboxOrRebase => "Applying patches",
        }
    }
}

/// The action to perform on a commit during interactive rebase.
#[derive(Debug, Clone)]
pub enum RebaseEntryAction {
    Pick,
    Reword(String),
    Squash,
    Fixup,
    Drop,
}

/// A single entry in an interactive rebase plan.
#[derive(Debug, Clone)]
pub struct RebasePlanEntry {
    pub oid: String,
    pub message: String,
    pub action: RebaseEntryAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperationKind {
    Fetch,
    Pull,
    Push,
    Checkout,
    Merge,
    CherryPick,
    Revert,
    Reset,
    RemoveRemote,
    Commit,
    Stage,
    Unstage,
    Stash,
    Branch,
    Tag,
    Discard,
    Rebase,
    Bisect,
    Worktree,
    ResolveConflict,
}

impl GitOperationKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            GitOperationKind::Fetch => "Fetch",
            GitOperationKind::Pull => "Pull",
            GitOperationKind::Push => "Push",
            GitOperationKind::Checkout => "Checkout",
            GitOperationKind::Merge => "Merge",
            GitOperationKind::CherryPick => "Cherry-pick",
            GitOperationKind::Revert => "Revert",
            GitOperationKind::Reset => "Reset",
            GitOperationKind::RemoveRemote => "Remove remote",
            GitOperationKind::Commit => "Commit",
            GitOperationKind::Stage => "Stage",
            GitOperationKind::Unstage => "Unstage",
            GitOperationKind::Stash => "Stash",
            GitOperationKind::Branch => "Branch",
            GitOperationKind::Tag => "Tag",
            GitOperationKind::Discard => "Discard",
            GitOperationKind::Rebase => "Rebase",
            GitOperationKind::Bisect => "Bisect",
            GitOperationKind::Worktree => "Worktree",
            GitOperationKind::ResolveConflict => "Resolve conflict",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperationState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct GitOperationUpdate {
    pub id: u64,
    pub kind: GitOperationKind,
    pub state: GitOperationState,
    pub summary: String,
    pub details: Option<String>,
    pub remote_name: Option<String>,
    pub branch_name: Option<String>,
    pub retryable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_label_display_name() {
        assert_eq!(RefLabel::Head.display_name(), "HEAD");
        assert_eq!(RefLabel::LocalBranch("main".into()).display_name(), "main");
        assert_eq!(
            RefLabel::RemoteBranch("origin/main".into()).display_name(),
            "origin/main"
        );
        assert_eq!(RefLabel::Tag("v1.0.0".into()).display_name(), "v1.0.0");
    }

    #[test]
    fn file_change_kind_short_code() {
        assert_eq!(FileChangeKind::Added.short_code(), "A");
        assert_eq!(FileChangeKind::Modified.short_code(), "M");
        assert_eq!(FileChangeKind::Deleted.short_code(), "D");
        assert_eq!(FileChangeKind::Renamed.short_code(), "R");
        assert_eq!(FileChangeKind::Copied.short_code(), "C");
        assert_eq!(FileChangeKind::TypeChange.short_code(), "T");
        assert_eq!(FileChangeKind::Untracked.short_code(), "?");
        assert_eq!(FileChangeKind::Conflicted.short_code(), "!");
    }

    #[test]
    fn repo_state_from_git2() {
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::Clean),
            RepoState::Clean
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::Merge),
            RepoState::Merge
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::Revert),
            RepoState::Revert
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::RevertSequence),
            RepoState::RevertSequence
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::CherryPick),
            RepoState::CherryPick
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::CherryPickSequence),
            RepoState::CherryPickSequence
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::Bisect),
            RepoState::Bisect
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::Rebase),
            RepoState::Rebase
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::RebaseInteractive),
            RepoState::RebaseInteractive
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::RebaseMerge),
            RepoState::RebaseMerge
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::ApplyMailbox),
            RepoState::ApplyMailbox
        );
        assert_eq!(
            RepoState::from_git2(git2::RepositoryState::ApplyMailboxOrRebase),
            RepoState::ApplyMailboxOrRebase
        );
    }

    #[test]
    fn repo_state_is_clean() {
        assert!(RepoState::Clean.is_clean());
        assert!(!RepoState::Merge.is_clean());
        assert!(!RepoState::Rebase.is_clean());
        assert!(!RepoState::RebaseInteractive.is_clean());
        assert!(!RepoState::CherryPick.is_clean());
        assert!(!RepoState::Bisect.is_clean());
        assert!(!RepoState::ApplyMailbox.is_clean());
    }

    #[test]
    fn repo_state_label() {
        assert_eq!(RepoState::Clean.label(), "Clean");
        assert_eq!(RepoState::Merge.label(), "Merging");
        assert_eq!(RepoState::Revert.label(), "Reverting");
        assert_eq!(RepoState::RevertSequence.label(), "Reverting");
        assert_eq!(RepoState::CherryPick.label(), "Cherry-picking");
        assert_eq!(RepoState::CherryPickSequence.label(), "Cherry-picking");
        assert_eq!(RepoState::Bisect.label(), "Bisecting");
        assert_eq!(RepoState::Rebase.label(), "Rebasing");
        assert_eq!(
            RepoState::RebaseInteractive.label(),
            "Rebasing (interactive)"
        );
        assert_eq!(RepoState::RebaseMerge.label(), "Rebasing");
        assert_eq!(RepoState::ApplyMailbox.label(), "Applying patches");
        assert_eq!(RepoState::ApplyMailboxOrRebase.label(), "Applying patches");
    }

    #[test]
    fn git_operation_kind_display_name() {
        assert_eq!(GitOperationKind::Fetch.display_name(), "Fetch");
        assert_eq!(GitOperationKind::Pull.display_name(), "Pull");
        assert_eq!(GitOperationKind::Push.display_name(), "Push");
        assert_eq!(GitOperationKind::Checkout.display_name(), "Checkout");
        assert_eq!(GitOperationKind::Merge.display_name(), "Merge");
        assert_eq!(GitOperationKind::CherryPick.display_name(), "Cherry-pick");
        assert_eq!(GitOperationKind::Revert.display_name(), "Revert");
        assert_eq!(GitOperationKind::Reset.display_name(), "Reset");
        assert_eq!(
            GitOperationKind::RemoveRemote.display_name(),
            "Remove remote"
        );
        assert_eq!(GitOperationKind::Commit.display_name(), "Commit");
        assert_eq!(GitOperationKind::Stage.display_name(), "Stage");
        assert_eq!(GitOperationKind::Unstage.display_name(), "Unstage");
        assert_eq!(GitOperationKind::Stash.display_name(), "Stash");
        assert_eq!(GitOperationKind::Branch.display_name(), "Branch");
        assert_eq!(GitOperationKind::Tag.display_name(), "Tag");
        assert_eq!(GitOperationKind::Discard.display_name(), "Discard");
        assert_eq!(GitOperationKind::Rebase.display_name(), "Rebase");
        assert_eq!(GitOperationKind::Bisect.display_name(), "Bisect");
    }
}
