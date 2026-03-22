mod auth;
mod blame;
mod diff;
mod file_history;
mod local_ops;
mod network;
mod rebase;
mod reflog;
mod refresh;
mod submodule;
mod watcher;

use anyhow::{Context as _, Result};
use git2::{Repository, StatusOptions};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use notify::RecommendedWatcher;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::types::*;

pub use blame::{compute_blame, BlameEntry, BlameLine};
pub use diff::{
    compute_commit_diff, compute_file_diff, compute_staged_diff_text, compute_stash_diff,
};
pub use reflog::{compute_reflog, ReflogEntryInfo};
pub use refresh::gather_refresh_data;
pub use submodule::{
    compute_submodules, submodule_init, submodule_init_all, submodule_update, submodule_update_all,
    SubmoduleInfo,
};

const DEFAULT_COMMIT_LIMIT: usize = 1000;

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
    status: Arc<WorkingTreeStatus>,
    recent_commits: Arc<Vec<CommitInfo>>,
    /// Whether the repository has more commits beyond the loaded set.
    has_more_commits: bool,
    next_operation_id: u64,

    // Filesystem watcher (kept alive)
    _watcher: Option<RecommendedWatcher>,
}

impl EventEmitter<GitProjectEvent> for GitProject {}

impl GitProject {
    /// Create a minimal non-functional instance for error recovery paths.
    /// The resulting entity should be dropped immediately; it exists only
    /// to satisfy GPUI's `cx.new()` requirement of returning a value.
    pub fn empty_at(path: PathBuf) -> Self {
        Self {
            repo_path: path,
            head_branch: None,
            head_detached: false,
            repo_state: RepoState::Clean,
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            status: Arc::new(WorkingTreeStatus::default()),
            recent_commits: Arc::new(Vec::new()),
            has_more_commits: false,
            next_operation_id: 1,
            _watcher: None,
        }
    }

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
            status: Arc::new(WorkingTreeStatus::default()),
            recent_commits: Arc::new(Vec::new()),
            has_more_commits: false,
            next_operation_id: 1,
            _watcher: None,
        };

        project.start_watcher(cx);

        Ok(project)
    }

    pub(crate) fn begin_operation(
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

    pub(crate) fn complete_op(
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

    pub(crate) fn fail_op(
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

    pub(crate) fn fail_to_start_task(
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

    pub fn status_arc(&self) -> Arc<WorkingTreeStatus> {
        Arc::clone(&self.status)
    }

    pub fn recent_commits(&self) -> &[CommitInfo] {
        &self.recent_commits
    }

    pub fn recent_commits_arc(&self) -> Arc<Vec<CommitInfo>> {
        Arc::clone(&self.recent_commits)
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

    pub(crate) fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.repo_path)
            .with_context(|| format!("Failed to open repository at {}", self.repo_path.display()))
    }

    /// Apply pre-gathered refresh data to self.
    pub(crate) fn apply_refresh_data(&mut self, data: RefreshData) {
        self.head_branch = data.head_branch;
        self.head_detached = data.head_detached;
        self.repo_state = data.repo_state;
        self.branches = data.branches;
        self.tags = data.tags;
        self.remotes = data.remotes;
        self.stashes = data.stashes;
        self.status = Arc::new(data.status);
        self.recent_commits = Arc::new(data.recent_commits);
        self.has_more_commits = data.has_more_commits;
    }

    /// Whether there are more commits beyond the currently loaded set.
    pub fn has_more_commits(&self) -> bool {
        self.has_more_commits
    }
}
