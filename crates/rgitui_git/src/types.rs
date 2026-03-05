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
