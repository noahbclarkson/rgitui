use anyhow::{Context as _, Result};
use chrono::{TimeZone, Utc};
use git2::{DiffOptions, Repository, StatusOptions};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::types::*;

/// Events emitted by GitProject.
#[derive(Debug, Clone)]
pub enum GitProjectEvent {
    StatusChanged,
    HeadChanged,
    RefsChanged,
    OperationStarted(String),
    OperationCompleted(String),
    OperationFailed(String, String),
}

/// The core Git project state holder.
/// Wraps a git2::Repository and provides async operations.
pub struct GitProject {
    repo_path: PathBuf,

    // Cached state
    head_branch: Option<String>,
    branches: Vec<BranchInfo>,
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    stashes: Vec<StashEntry>,
    status: WorkingTreeStatus,
    recent_commits: Vec<CommitInfo>,

    // Filesystem watcher (kept alive)
    _watcher: Option<RecommendedWatcher>,
}

impl EventEmitter<GitProjectEvent> for GitProject {}

impl GitProject {
    /// Open a repository at the given path.
    pub fn open(path: PathBuf, cx: &mut Context<Self>) -> Result<Self> {
        let repo = Repository::open(&path)
            .with_context(|| format!("Failed to open repository at {}", path.display()))?;

        let repo_path = repo
            .workdir()
            .unwrap_or_else(|| repo.path())
            .to_path_buf();

        let mut project = Self {
            repo_path,
            head_branch: None,
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            status: WorkingTreeStatus::default(),
            recent_commits: Vec::new(),
            _watcher: None,
        };

        project.refresh_sync()?;

        // Set up filesystem watching
        project.start_watcher(cx);

        Ok(project)
    }

    /// Start watching the repository for filesystem changes.
    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let dirty = Arc::new(AtomicBool::new(false));
        let dirty_flag = dirty.clone();

        let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Ignore .git directory internal changes to avoid feedback loops
                let dominated_by_git = event.paths.iter().all(|p| {
                    p.components().any(|c| c.as_os_str() == ".git")
                });
                if dominated_by_git {
                    return;
                }
                match event.kind {
                    notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_) => {
                        dirty_flag.store(true, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        });

        if let Ok(mut watcher) = watcher {
            let _ = watcher.watch(&repo_path, RecursiveMode::Recursive);
            self._watcher = Some(watcher);

            // Poll the dirty flag from async executor — never blocks the UI thread
            cx.spawn(async move |weak, cx: &mut AsyncApp| {
                loop {
                    // Async sleep — yields to executor, keeps UI responsive
                    smol::Timer::after(Duration::from_millis(300)).await;

                    if dirty.swap(false, Ordering::Relaxed) {
                        // Debounce: wait a bit more for quiet
                        smol::Timer::after(Duration::from_millis(200)).await;
                        dirty.store(false, Ordering::Relaxed);

                        let result = cx.update(|cx| {
                            weak.update(cx, |this, cx| {
                                let _ = this.refresh_sync();
                                cx.emit(GitProjectEvent::StatusChanged);
                                cx.notify();
                            })
                        });

                        if result.is_err() {
                            break;
                        }
                    }
                }
            })
            .detach();
        }
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

    pub fn branches(&self) -> &[BranchInfo] {
        &self.branches
    }

    pub fn tags(&self) -> &[TagInfo] {
        &self.tags
    }

    pub fn remotes(&self) -> &[RemoteInfo] {
        &self.remotes
    }

    pub fn stashes(&self) -> &[StashEntry] {
        &self.stashes
    }

    pub fn status(&self) -> &WorkingTreeStatus {
        &self.status
    }

    pub fn recent_commits(&self) -> &[CommitInfo] {
        &self.recent_commits
    }

    pub fn has_changes(&self) -> bool {
        !self.status.staged.is_empty() || !self.status.unstaged.is_empty()
    }

    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.repo_path)
            .with_context(|| format!("Failed to open repository at {}", self.repo_path.display()))
    }

    /// Refresh all cached state synchronously.
    fn refresh_sync(&mut self) -> Result<()> {
        let repo = self.open_repo()?;
        self.refresh_head(&repo)?;
        self.refresh_branches(&repo)?;
        self.refresh_tags(&repo)?;
        self.refresh_remotes(&repo)?;
        self.refresh_stashes(&repo)?;
        self.refresh_status(&repo)?;
        self.refresh_recent_commits(&repo, 200)?;
        Ok(())
    }

    /// Refresh all state asynchronously.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        cx.spawn(async |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let result = this.refresh_sync();
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    result
                })
            })?
        })
    }

    fn refresh_head(&mut self, repo: &Repository) -> Result<()> {
        self.head_branch = repo
            .head()
            .ok()
            .and_then(|r| r.shorthand().map(String::from));
        Ok(())
    }

    fn refresh_branches(&mut self, repo: &Repository) -> Result<()> {
        self.branches.clear();
        let branches = repo.branches(None)?;
        for branch_result in branches {
            let (branch, branch_type) = branch_result?;
            let name = branch.name()?.unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }

            let is_head = branch.is_head();
            let is_remote = branch_type == git2::BranchType::Remote;
            let tip_oid = branch.get().target();

            let upstream = branch.upstream().ok().and_then(|u| {
                u.name().ok().flatten().map(String::from)
            });

            let (ahead, behind) = if let (Some(local_oid), Ok(upstream_ref)) =
                (tip_oid, branch.upstream())
            {
                if let Some(remote_oid) = upstream_ref.get().target() {
                    repo.graph_ahead_behind(local_oid, remote_oid)
                        .unwrap_or((0, 0))
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

            self.branches.push(BranchInfo {
                name,
                is_head,
                is_remote,
                upstream,
                ahead,
                behind,
                tip_oid,
            });
        }

        // Sort: HEAD first, then local, then remote
        self.branches.sort_by(|a, b| {
            b.is_head
                .cmp(&a.is_head)
                .then(a.is_remote.cmp(&b.is_remote))
                .then(a.name.cmp(&b.name))
        });

        Ok(())
    }

    fn refresh_tags(&mut self, repo: &Repository) -> Result<()> {
        self.tags.clear();
        // tag_foreach can fail on empty repos — ignore errors
        let _ = repo.tag_foreach(|oid, name_bytes| {
            if let Ok(name) = std::str::from_utf8(name_bytes) {
                let name = name.strip_prefix("refs/tags/").unwrap_or(name).to_string();
                self.tags.push(TagInfo {
                    name,
                    oid,
                    message: None,
                });
            }
            true
        });
        self.tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    fn refresh_remotes(&mut self, repo: &Repository) -> Result<()> {
        self.remotes.clear();
        let remote_names = repo.remotes()?;
        for name in remote_names.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name) {
                self.remotes.push(RemoteInfo {
                    name: name.to_string(),
                    url: remote.url().map(String::from),
                    push_url: remote.pushurl().map(String::from),
                });
            }
        }
        Ok(())
    }

    fn refresh_stashes(&mut self, _repo: &Repository) -> Result<()> {
        self.stashes.clear();
        // stash_foreach requires &mut, so open a fresh handle
        let mut repo_mut = Repository::open(&self.repo_path)?;
        repo_mut.stash_foreach(|stash_index, message, oid| {
            self.stashes.push(StashEntry {
                index: stash_index,
                message: message.to_string(),
                oid: *oid,
            });
            true
        })?;
        Ok(())
    }

    fn refresh_status(&mut self, repo: &Repository) -> Result<()> {
        self.status = WorkingTreeStatus::default();

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_unmodified(false);

        let statuses = repo.statuses(Some(&mut opts))?;
        for entry in statuses.iter() {
            let path = PathBuf::from(entry.path().unwrap_or(""));
            let status = entry.status();

            // Index (staged) changes
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                let kind = if status.contains(git2::Status::INDEX_NEW) {
                    FileChangeKind::Added
                } else if status.contains(git2::Status::INDEX_MODIFIED) {
                    FileChangeKind::Modified
                } else if status.contains(git2::Status::INDEX_DELETED) {
                    FileChangeKind::Deleted
                } else if status.contains(git2::Status::INDEX_RENAMED) {
                    FileChangeKind::Renamed
                } else {
                    FileChangeKind::TypeChange
                };
                self.status.staged.push(FileStatus {
                    path: path.clone(),
                    kind,
                    old_path: None,
                });
            }

            // Working tree (unstaged) changes
            if status.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            ) {
                let kind = if status.contains(git2::Status::WT_NEW) {
                    FileChangeKind::Untracked
                } else if status.contains(git2::Status::WT_MODIFIED) {
                    FileChangeKind::Modified
                } else if status.contains(git2::Status::WT_DELETED) {
                    FileChangeKind::Deleted
                } else if status.contains(git2::Status::WT_RENAMED) {
                    FileChangeKind::Renamed
                } else {
                    FileChangeKind::TypeChange
                };
                self.status.unstaged.push(FileStatus {
                    path: path.clone(),
                    kind,
                    old_path: None,
                });
            }

            // Conflicts
            if status.contains(git2::Status::CONFLICTED) {
                self.status.unstaged.push(FileStatus {
                    path,
                    kind: FileChangeKind::Conflicted,
                    old_path: None,
                });
            }
        }

        Ok(())
    }

    fn refresh_recent_commits(&mut self, repo: &Repository, limit: usize) -> Result<()> {
        self.recent_commits.clear();

        // Build ref map: oid -> list of ref labels
        let mut ref_map = std::collections::HashMap::<git2::Oid, Vec<RefLabel>>::new();

        if let Ok(head) = repo.head() {
            if let Some(oid) = head.target() {
                ref_map.entry(oid).or_default().push(RefLabel::Head);
            }
        }

        for branch in &self.branches {
            if let Some(oid) = branch.tip_oid {
                let label = if branch.is_remote {
                    RefLabel::RemoteBranch(branch.name.clone())
                } else {
                    RefLabel::LocalBranch(branch.name.clone())
                };
                ref_map.entry(oid).or_default().push(label);
            }
        }

        for tag in &self.tags {
            ref_map
                .entry(tag.oid)
                .or_default()
                .push(RefLabel::Tag(tag.name.clone()));
        }

        // Walk commits from all refs so all branches appear in the graph.
        let mut revwalk = repo.revwalk()?;
        let has_head = revwalk.push_head().is_ok();
        // Push all local and remote branch tips so branches ahead of HEAD are visible.
        for branch in &self.branches {
            if let Some(oid) = branch.tip_oid {
                revwalk.push(oid).ok();
            }
        }
        if !has_head && self.branches.is_empty() {
            // Completely empty repo
            return Ok(());
        }
        revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;

        let mut count = 0;
        for oid_result in revwalk {
            if count >= limit {
                break;
            }
            let oid = oid_result?;
            let commit = repo.find_commit(oid)?;

            let author = commit.author();
            let committer = commit.committer();
            let time = Utc.timestamp_opt(commit.time().seconds(), 0).single();

            let refs = ref_map.remove(&oid).unwrap_or_default();

            self.recent_commits.push(CommitInfo {
                oid,
                short_id: format!("{:.7}", oid),
                summary: commit.summary().unwrap_or("").to_string(),
                message: commit.message().unwrap_or("").to_string(),
                author: Signature {
                    name: author.name().unwrap_or("").to_string(),
                    email: author.email().unwrap_or("").to_string(),
                },
                committer: Signature {
                    name: committer.name().unwrap_or("").to_string(),
                    email: committer.email().unwrap_or("").to_string(),
                },
                time: time.unwrap_or_else(|| Utc::now()),
                parent_oids: commit.parent_ids().collect(),
                refs,
            });

            count += 1;
        }

        Ok(())
    }

    // -- Write operations --

    /// Stage specific files.
    pub fn stage_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Task<Result<()>> {
        let paths = paths.to_vec();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    let mut index = repo.index()?;
                    for path in &paths {
                        if path.exists() || this.repo_path.join(path).exists() {
                            index.add_path(path)?;
                        } else {
                            index.remove_path(path)?;
                        }
                    }
                    index.write()?;
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Unstage specific files.
    pub fn unstage_files(
        &mut self,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let paths = paths.to_vec();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    // On empty repos, head() fails — unstage by removing from index
                    if let Ok(head_tree) = repo.head().and_then(|h| h.peel_to_tree()) {
                        repo.reset_default(Some(&head_tree.into_object()), &paths)?;
                    } else {
                        let mut index = repo.index()?;
                        for path in &paths {
                            let _ = index.remove_path(path);
                        }
                        index.write()?;
                    }
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Stage all changes.
    pub fn stage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    let mut index = repo.index()?;
                    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                    index.write()?;
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Unstage all changes.
    pub fn unstage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    if let Ok(head) = repo.head() {
                        let obj = head.peel(git2::ObjectType::Any)?;
                        repo.reset(&obj, git2::ResetType::Mixed, None)?;
                    }
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Create a commit with the current staged changes.
    pub fn commit(
        &mut self,
        message: &str,
        amend: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<git2::Oid>> {
        let message = message.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    let sig = repo.signature()?;
                    let mut index = repo.index()?;
                    let tree_oid = index.write_tree()?;
                    let tree = repo.find_tree(tree_oid)?;

                    let oid = if amend {
                        let head = repo.head()?.peel_to_commit()?;
                        head.amend(
                            Some("HEAD"),
                            Some(&sig),
                            Some(&sig),
                            None,
                            Some(&message),
                            Some(&tree),
                        )?
                    } else {
                        let parents: Vec<git2::Commit> = if let Ok(head) = repo.head() {
                            vec![head.peel_to_commit()?]
                        } else {
                            vec![]
                        };
                        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                        repo.commit(
                            Some("HEAD"),
                            &sig,
                            &sig,
                            &message,
                            &tree,
                            &parent_refs,
                        )?
                    };

                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::HeadChanged);
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(oid)
                })
            })?
        })
    }

    /// Get diff for a specific file (staged or unstaged).
    pub fn diff_file(&self, path: &Path, staged: bool) -> Result<FileDiff> {
        let repo = self.open_repo()?;
        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(path);

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
        let stats = diff.stats()?;

        let mut files: Vec<FileDiff> = Vec::new();
        let mut current_hunks: Vec<DiffHunk> = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_additions: usize = 0;
        let mut current_deletions: usize = 0;

        // Use print which handles the callback ordering correctly
        diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
            let delta_path = delta
                .new_file()
                .path()
                .unwrap_or(Path::new(""))
                .to_path_buf();

            // Check if we've moved to a new file
            if current_path.as_ref() != Some(&delta_path) {
                // Save previous file
                if let Some(prev_path) = current_path.take() {
                    files.push(FileDiff {
                        path: prev_path,
                        hunks: std::mem::take(&mut current_hunks),
                        additions: current_additions,
                        deletions: current_deletions,
                    });
                }
                current_path = Some(delta_path);
                current_additions = 0;
                current_deletions = 0;
            }

            if let Some(hunk) = hunk {
                // Ensure we have a hunk entry
                let header = String::from_utf8_lossy(hunk.header()).to_string();
                let expected_start = hunk.new_start();
                // Check if this is a new hunk
                let needs_new = current_hunks.last().map_or(true, |h| h.new_start != expected_start || h.header != header);
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

        // Don't forget the last file
        if let Some(path) = current_path {
            files.push(FileDiff {
                path,
                hunks: current_hunks,
                additions: current_additions,
                deletions: current_deletions,
            });
        }

        Ok(CommitDiff {
            total_additions: stats.insertions(),
            total_deletions: stats.deletions(),
            files,
        })
    }

    /// Checkout a branch by name.
    pub fn checkout_branch(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    cx.emit(GitProjectEvent::OperationStarted(format!("Checking out {}...", name)));
                    let result: Result<()> = (|| {
                        let repo = this.open_repo()?;
                        let obj = repo.revparse_single(&format!("refs/heads/{}", name))?;
                        let mut checkout_opts = git2::build::CheckoutBuilder::new();
                        checkout_opts.safe();
                        repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                        repo.set_head(&format!("refs/heads/{}", name))?;
                        this.refresh_sync()?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            cx.emit(GitProjectEvent::OperationCompleted(format!("Switched to {}", name)));
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            cx.emit(GitProjectEvent::OperationFailed(
                                "Checkout".to_string(),
                                e.to_string(),
                            ));
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Create a new branch from HEAD.
    pub fn create_branch(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    let head = repo.head()?.peel_to_commit()?;
                    repo.branch(&name, &head, false)?;
                    this.refresh_branches(&repo)?;
                    cx.emit(GitProjectEvent::RefsChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Delete a local branch.
    pub fn delete_branch(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let repo = this.open_repo()?;
                    let mut branch = repo.find_branch(&name, git2::BranchType::Local)?;
                    branch.delete()?;
                    this.refresh_branches(&repo)?;
                    cx.emit(GitProjectEvent::RefsChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Discard changes in specific files (restore to HEAD).
    pub fn discard_changes(
        &mut self,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let paths = paths.to_vec();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let result: Result<()> = (|| {
                        let repo = this.open_repo()?;
                        let workdir = repo
                            .workdir()
                            .ok_or_else(|| anyhow::anyhow!("Bare repository has no working directory"))?
                            .to_path_buf();
                        let mut checkout_opts = git2::build::CheckoutBuilder::new();
                        checkout_opts.force();
                        let mut has_tracked = false;
                        for path in &paths {
                            // Untracked (new) files must be deleted; checkout_head can't restore them
                            let is_untracked = repo
                                .status_file(path)
                                .map(|s| s.contains(git2::Status::WT_NEW))
                                .unwrap_or(false);
                            if is_untracked {
                                let full = workdir.join(path);
                                if full.is_file() {
                                    std::fs::remove_file(&full)
                                        .with_context(|| format!("Failed to delete {}", full.display()))?;
                                }
                            } else {
                                checkout_opts.path(path);
                                has_tracked = true;
                            }
                        }
                        if has_tracked {
                            repo.checkout_head(Some(&mut checkout_opts))?;
                        }
                        this.refresh_status(&repo)?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            cx.emit(GitProjectEvent::OperationFailed(
                                "Discard".to_string(),
                                e.to_string(),
                            ));
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Hard reset to HEAD, discarding all working tree and index changes.
    pub fn reset_hard(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let result: Result<()> = (|| {
                        let repo = this.open_repo()?;
                        let head_commit = repo.head()?.peel_to_commit()?;
                        repo.reset(head_commit.as_object(), git2::ResetType::Hard, None)?;
                        this.refresh_sync()?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            cx.emit(GitProjectEvent::OperationFailed(
                                "Reset".to_string(),
                                e.to_string(),
                            ));
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Revert a commit (creates a new commit that undoes the given commit).
    pub fn revert_commit(
        &mut self,
        oid: git2::Oid,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let result: Result<()> = (|| {
                        let repo = this.open_repo()?;
                        let commit = repo.find_commit(oid)?;
                        let mut opts = git2::RevertOptions::new();
                        repo.revert(&commit, Some(&mut opts))?;
                        // Auto-commit the revert
                        let sig = repo.signature()?;
                        let msg = format!("Revert \"{}\"", commit.summary().unwrap_or(""));
                        let index = repo.index()?;
                        let tree_id = {
                            let mut idx = repo.index()?;
                            idx.write_tree()?
                        };
                        let tree = repo.find_tree(tree_id)?;
                        let head_commit = repo.head()?.peel_to_commit()?;
                        repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head_commit])?;
                        repo.cleanup_state()?;
                        this.refresh_sync()?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            cx.emit(GitProjectEvent::OperationFailed(
                                "Revert".to_string(),
                                e.to_string(),
                            ));
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Fetch from a remote.
    pub fn fetch(
        &mut self,
        remote_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    cx.emit(GitProjectEvent::OperationStarted("Fetching...".into()));
                    let repo = this.open_repo()?;
                    let mut remote = repo.find_remote(&remote_name)?;
                    remote.fetch(&[] as &[&str], None, None)?;
                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::OperationCompleted("Fetch complete".into()));
                    cx.emit(GitProjectEvent::RefsChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Pull from a remote (fetch + merge).
    pub fn pull(
        &mut self,
        remote_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    cx.emit(GitProjectEvent::OperationStarted("Pulling...".into()));
                    let repo = this.open_repo()?;

                    // Fetch first
                    let mut remote = repo.find_remote(&remote_name)?;
                    remote.fetch(&[] as &[&str], None, None)?;

                    // Find upstream branch to merge
                    let head = repo.head()?;
                    let branch_name = head.shorthand().unwrap_or("main").to_string();
                    let fetch_head = repo.find_reference("FETCH_HEAD")?;
                    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;

                    // Perform merge analysis
                    let (analysis, _pref) = repo.merge_analysis(&[&fetch_commit])?;

                    if analysis.is_up_to_date() {
                        cx.emit(GitProjectEvent::OperationCompleted("Already up to date".into()));
                    } else if analysis.is_fast_forward() {
                        // Fast-forward
                        let refname = format!("refs/heads/{}", branch_name);
                        let mut reference = repo.find_reference(&refname)?;
                        reference.set_target(fetch_commit.id(), "Fast-forward pull")?;
                        repo.set_head(&refname)?;
                        repo.checkout_head(Some(
                            git2::build::CheckoutBuilder::new().force(),
                        ))?;
                        cx.emit(GitProjectEvent::OperationCompleted("Pull complete (fast-forward)".into()));
                    } else if analysis.is_normal() {
                        // Normal merge
                        repo.merge(&[&fetch_commit], None, None)?;
                        // Auto-commit if no conflicts
                        let has_conflicts = repo.index()?.has_conflicts();
                        if !has_conflicts {
                            let sig = repo.signature()?;
                            let mut index = repo.index()?;
                            let tree_oid = index.write_tree()?;
                            let tree = repo.find_tree(tree_oid)?;
                            let head_commit = repo.head()?.peel_to_commit()?;
                            let fetch_commit_obj = repo.find_commit(fetch_commit.id())?;
                            repo.commit(
                                Some("HEAD"),
                                &sig,
                                &sig,
                                &format!("Merge remote-tracking branch '{}/{}'", remote_name, branch_name),
                                &tree,
                                &[&head_commit, &fetch_commit_obj],
                            )?;
                            repo.cleanup_state()?;
                            cx.emit(GitProjectEvent::OperationCompleted("Pull complete (merge)".into()));
                        } else {
                            cx.emit(GitProjectEvent::OperationCompleted("Pull complete (conflicts to resolve)".into()));
                        }
                    }

                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::HeadChanged);
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Push to a remote.
    pub fn push(
        &mut self,
        remote_name: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let remote_name = remote_name.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    cx.emit(GitProjectEvent::OperationStarted("Pushing...".into()));
                    let repo = this.open_repo()?;
                    let head = repo.head()?;
                    let branch_name = head.shorthand().unwrap_or("main").to_string();

                    let mut remote = repo.find_remote(&remote_name)?;
                    let refspec = if force {
                        format!("+refs/heads/{}:refs/heads/{}", branch_name, branch_name)
                    } else {
                        format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name)
                    };
                    remote.push(&[&refspec], None)?;

                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::OperationCompleted("Push complete".into()));
                    cx.emit(GitProjectEvent::RefsChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Save the current working tree to a stash.
    pub fn stash_save(
        &mut self,
        message: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let message = message.map(String::from);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let mut repo = Repository::open(&this.repo_path)?;
                    let sig = repo.signature()?;
                    repo.stash_save(
                        &sig,
                        message.as_deref().unwrap_or("WIP"),
                        None,
                    )?;
                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Pop the top stash entry.
    pub fn stash_pop(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let mut repo = Repository::open(&this.repo_path)?;
                    repo.stash_pop(index, None)?;
                    this.refresh_sync()?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
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
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let patch_text = this.generate_hunk_patch(&file_path, hunk_index, false)?;
                    let repo = this.open_repo()?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    repo.apply(&diff, git2::ApplyLocation::Index, None)?;
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
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
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let patch_text = this.generate_hunk_patch(&file_path, hunk_index, true)?;
                    let repo = this.open_repo()?;
                    let diff = git2::Diff::from_buffer(patch_text.as_bytes())?;
                    let mut opts = git2::ApplyOptions::new();
                    repo.apply(&diff, git2::ApplyLocation::Index, Some(&mut opts))?;
                    this.refresh_status(&repo)?;
                    cx.emit(GitProjectEvent::StatusChanged);
                    cx.notify();
                    Ok(())
                })
            })?
        })
    }

    /// Generate a patch for a single hunk from a file's diff.
    fn generate_hunk_patch(&self, file_path: &Path, hunk_index: usize, staged: bool) -> Result<String> {
        let repo = self.open_repo()?;
        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(file_path);

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
            // Write file header lines (those without a hunk)
            if hunk.is_none() {
                if !file_header_written {
                    let content = String::from_utf8_lossy(line.content());
                    match line.origin() {
                        'F' => {
                            // File header line - write as-is
                            patch_text.push_str(&content);
                        }
                        _ => {
                            let prefix = match line.origin() {
                                '+' | '-' | ' ' | '>' | '<' => {
                                    String::from(line.origin())
                                }
                                _ => String::new(),
                            };
                            patch_text.push_str(&prefix);
                            patch_text.push_str(&content);
                        }
                    }
                }
                return true;
            }

            let hunk = hunk.unwrap();
            let header = String::from_utf8_lossy(hunk.header()).to_string();

            // Track hunk index
            let is_new_hunk = if current_hunk_idx < 0 {
                true
            } else {
                // Check if header changed
                let _prev_start = hunk.new_start();
                current_hunk_idx >= 0 && patch_text.contains(&header) == false
            };

            if is_new_hunk || current_hunk_idx < 0 {
                current_hunk_idx += 1;
            }

            // Only include lines from the target hunk
            if current_hunk_idx as usize == hunk_index {
                if !file_header_written {
                    // Generate a minimal file header
                    let old_path = delta.old_file().path().unwrap_or(Path::new(""));
                    let new_path = delta.new_file().path().unwrap_or(Path::new(""));
                    patch_text.clear();
                    patch_text.push_str(&format!("--- a/{}\n", old_path.display()));
                    patch_text.push_str(&format!("+++ b/{}\n", new_path.display()));
                    file_header_written = true;
                }

                let content = String::from_utf8_lossy(line.content());
                match line.origin() {
                    'H' => {
                        patch_text.push_str(&content);
                    }
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

        // Ensure trailing newline
        if !patch_text.ends_with('\n') {
            patch_text.push('\n');
        }

        Ok(patch_text)
    }

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
            parts.push(format!("{} {}", file.kind.short_code(), file.path.display()));
        }
        parts.join("\n")
    }
}

/// Parse a git2::Diff into a FileDiff using the print API to avoid borrow issues.
fn parse_file_diff(path: &Path, diff: &git2::Diff) -> Result<FileDiff> {
    let mut file_diff = FileDiff {
        path: path.to_path_buf(),
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
    };

    diff.print(git2::DiffFormat::Patch, |_delta, hunk, line| {
        if let Some(hunk) = hunk {
            let header = String::from_utf8_lossy(hunk.header()).to_string();
            let expected_start = hunk.new_start();
            let needs_new = file_diff.hunks.last().map_or(true, |h| h.new_start != expected_start || h.header != header);
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
