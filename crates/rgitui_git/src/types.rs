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
    pub time: DateTime<Utc>,
    pub parent_oids: Vec<git2::Oid>,
    pub refs: Vec<RefLabel>,
}

/// Information about a branch.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub oid: git2::Oid,
    pub message: Option<String>,
}

/// Information about a remote.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}

/// Information about a stash entry.
#[derive(Debug, Clone)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub oid: git2::Oid,
}

/// Status of a file in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
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

/// Result from a merge operation.
#[derive(Debug)]
pub enum MergeResult {
    AlreadyUpToDate,
    FastForward,
    Merged(git2::Oid),
    Conflict(Vec<PathBuf>),
}

/// Result from a pull operation.
#[derive(Debug)]
pub enum PullResult {
    AlreadyUpToDate,
    FastForward,
    Merged(git2::Oid),
    Conflict(Vec<PathBuf>),
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
