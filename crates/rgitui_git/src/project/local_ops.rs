use anyhow::{Context as _, Result};
use git2::Repository;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use std::path::PathBuf;
use std::process::Command;

use rgitui_settings::current_git_auth_runtime;

use crate::types::*;

use super::refresh::gather_refresh_data;
use super::{ensure_clean_worktree, head_branch_name, GitProject, GitProjectEvent, RefreshData};

impl GitProject {
    /// Stage specific files.
    pub fn stage_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let task_paths = paths.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            if paths.len() == 1 {
                format!("Staging {}...", paths[0].display())
            } else {
                format!("Staging {} files...", paths.len())
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut index = repo.index()?;
                    for path in &task_paths {
                        if repo_path.join(path).exists() {
                            index.add_path(path)?;
                        } else {
                            index.remove_path(path)?;
                        }
                    }
                    index.write()?;
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
                                if paths.len() == 1 {
                                    format!("Staged {}", paths[0].display())
                                } else {
                                    format!("Staged {} files", paths.len())
                                },
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Stage failed",
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

    /// Unstage specific files.
    pub fn unstage_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let task_paths = paths.clone();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            if paths.len() == 1 {
                format!("Unstaging {}...", paths[0].display())
            } else {
                format!("Unstaging {} files...", paths.len())
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    if let Ok(head_tree) = repo.head().and_then(|h| h.peel_to_tree()) {
                        repo.reset_default(Some(&head_tree.into_object()), &task_paths)?;
                    } else {
                        let mut index = repo.index()?;
                        for path in &task_paths {
                            if let Err(e) = index.remove_path(path) {
                                log::warn!(
                                    "Failed to remove path from index during unstage: {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                        index.write()?;
                    }
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
                                if paths.len() == 1 {
                                    format!("Unstaged {}", paths[0].display())
                                } else {
                                    format!("Unstaged {} files", paths.len())
                                },
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstage failed",
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

    /// Stage all changes.
    pub fn stage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stage,
            "Staging all changes...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut index = repo.index()?;
                    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                    index.write()?;
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
                                "Staged all changes",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stage,
                                "Stage all failed",
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

    /// Unstage all changes.
    pub fn unstage_all(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Unstage,
            "Unstaging all changes...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    if let Ok(head) = repo.head() {
                        let obj = head.peel(git2::ObjectType::Any)?;
                        repo.reset(&obj, git2::ResetType::Mixed, None)?;
                    }
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
                                "Unstaged all changes",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Unstage,
                                "Unstage all failed",
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

    /// Create a commit with the current staged changes.
    pub fn commit(
        &mut self,
        message: &str,
        amend: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<git2::Oid>> {
        let message = message.to_string();
        let task_message = message.clone();
        let commit_summary = message.lines().next().unwrap_or("").to_string();
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Commit,
            if amend {
                "Amending commit..."
            } else {
                "Creating commit..."
            },
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(git2::Oid, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let sig = repo.signature()?;
                    let mut index = repo.index()?;
                    if index.is_empty() {
                        anyhow::bail!("There are no staged changes to commit.")
                    }
                    let tree_oid = index.write_tree()?;
                    let tree = repo.find_tree(tree_oid)?;

                    let auth = current_git_auth_runtime();
                    let oid = if amend {
                        if auth.sign_commits {
                            let gpg_key = auth.gpg_key_id.as_deref().ok_or_else(|| {
                                anyhow::anyhow!(
                                    "GPG signing enabled but no key ID configured in settings"
                                )
                            })?;
                            let head = repo.head()?.peel_to_commit()?;
                            let parents: Vec<git2::Commit> = head.parents().collect();
                            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                            let buf = repo.commit_create_buffer(
                                &sig,
                                &sig,
                                &task_message,
                                &tree,
                                &parent_refs,
                            )?;
                            let buf_str = std::str::from_utf8(&buf)
                                .context("commit buffer contains invalid UTF-8")?;
                            let signature = sign_with_gpg(buf_str, gpg_key)?;
                            let commit_oid =
                                repo.commit_signed(buf_str, &signature, Some("gpgsig"))?;
                            if let Ok(mut head_ref) = repo.head() {
                                head_ref.set_target(commit_oid, "commit (gpg signed amend)")?;
                            } else {
                                repo.reference(
                                    "HEAD",
                                    commit_oid,
                                    true,
                                    "commit (gpg signed amend)",
                                )?;
                            }
                            commit_oid
                        } else {
                            let head = repo.head()?.peel_to_commit()?;
                            head.amend(
                                Some("HEAD"),
                                Some(&sig),
                                Some(&sig),
                                None,
                                Some(&task_message),
                                Some(&tree),
                            )?
                        }
                    } else {
                        let parents: Vec<git2::Commit> = if let Ok(head) = repo.head() {
                            vec![head.peel_to_commit()?]
                        } else {
                            vec![]
                        };
                        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                        if auth.sign_commits {
                            let gpg_key = auth.gpg_key_id.as_deref().ok_or_else(|| {
                                anyhow::anyhow!(
                                    "GPG signing enabled but no key ID configured in settings"
                                )
                            })?;
                            let buf = repo.commit_create_buffer(
                                &sig,
                                &sig,
                                &task_message,
                                &tree,
                                &parent_refs,
                            )?;
                            let buf_str = std::str::from_utf8(&buf)
                                .context("commit buffer contains invalid UTF-8")?;
                            let signature = sign_with_gpg(buf_str, gpg_key)?;
                            let commit_oid =
                                repo.commit_signed(buf_str, &signature, Some("gpgsig"))?;
                            if let Ok(mut head_ref) = repo.head() {
                                head_ref.set_target(commit_oid, "commit (gpg signed)")?;
                            } else {
                                repo.reference("HEAD", commit_oid, true, "commit (gpg signed)")?;
                            }
                            commit_oid
                        } else {
                            repo.commit(
                                Some("HEAD"),
                                &sig,
                                &sig,
                                &task_message,
                                &tree,
                                &parent_refs,
                            )?
                        }
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((oid, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| match result {
                    Ok((oid, data)) => {
                        this.apply_refresh_data(data);
                        this.complete_op(
                            operation_id,
                            GitOperationKind::Commit,
                            if amend {
                                format!("Amended commit {}", &oid.to_string()[..7])
                            } else {
                                format!("Created commit {}", &oid.to_string()[..7])
                            },
                            (Some(commit_summary.clone()), None, branch_name.clone()),
                            cx,
                        );
                        cx.emit(GitProjectEvent::HeadChanged);
                        cx.emit(GitProjectEvent::StatusChanged);
                        cx.notify();
                        Ok(oid)
                    }
                    Err(e) => {
                        this.fail_op(
                            operation_id,
                            GitOperationKind::Commit,
                            if amend {
                                "Amend failed"
                            } else {
                                "Commit failed"
                            },
                            e.to_string(),
                            (None, branch_name.clone(), false),
                            cx,
                        );
                        Err(e)
                    }
                })
            })?
        })
    }

    /// Checkout a branch by name.
    /// Handles both local branches and remote tracking branches (e.g. `origin/main`).
    /// For remote branches, creates a local tracking branch first.
    pub fn checkout_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Checkout,
            format!("Switching to '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Checkout")?;
                    let current_branch = head_branch_name(&repo).ok();

                    // Determine whether this is a local or remote branch, and the
                    // object + local branch name to use.
                    let (obj, local_branch_name, is_tracking) =
                        match repo.revparse_single(&format!("refs/heads/{}", task_name)) {
                            Ok(o) => (o, task_name.clone(), false),
                            Err(_) => {
                                // Not a local branch — check if it's a remote tracking branch
                                // (e.g. "origin/main" → refs/remotes/origin/main).
                                let remote_ref = format!("refs/remotes/{}", task_name);
                                if let Ok(remote_obj) = repo.revparse_single(&remote_ref) {
                                    let Some((_remote, short)) = task_name.split_once('/') else {
                                        anyhow::bail!(
                                            "Invalid remote branch name '{}'. \
                                            Expected 'remote/branch' format.",
                                            task_name
                                        );
                                    };
                                    let local_branch_name = short;

                                    // Refuse to overwrite an existing local branch.
                                    if repo
                                        .find_branch(local_branch_name, git2::BranchType::Local)
                                        .is_ok()
                                    {
                                        anyhow::bail!(
                                            "A local branch named '{}' already exists. \
                                            Please delete or rename it first.",
                                            local_branch_name
                                        );
                                    }

                                    // Create the local tracking branch at the remote's commit.
                                    let commit = remote_obj.peel_to_commit()?;
                                    repo.branch(local_branch_name, &commit, false)?;

                                    // Set upstream to track the remote branch.
                                    if let Ok(mut branch) =
                                        repo.find_branch(local_branch_name, git2::BranchType::Local)
                                    {
                                        let _ = branch.set_upstream(Some(&task_name));
                                    }

                                    (remote_obj, local_branch_name.to_string(), true)
                                } else {
                                    anyhow::bail!(
                                        "Branch '{}' not found as a local or remote branch. \
                                        Try fetching to update remote refs.",
                                        task_name
                                    );
                                }
                            }
                        };

                    // Bail if already on the target branch (use local name for tracking).
                    if current_branch.as_deref() == Some(local_branch_name.as_str()) {
                        anyhow::bail!("Already on branch '{}'.", local_branch_name);
                    }

                    let head_ref = if is_tracking {
                        format!("refs/heads/{}", local_branch_name)
                    } else {
                        format!("refs/heads/{}", task_name)
                    };

                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.safe();
                    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                    repo.set_head(&head_ref)?;
                    let data = gather_refresh_data(&repo_path)?;
                    let msg = if is_tracking {
                        format!(
                            "Switched to new branch '{}' tracking '{}'",
                            local_branch_name, task_name
                        )
                    } else {
                        format!("Switched to '{}'", task_name)
                    };
                    Ok((msg, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                msg,
                                (
                                    Some("Working tree updated for the selected branch.".into()),
                                    None,
                                    Some(name.clone()),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checkout of '{}' failed", name),
                                e.to_string(),
                                (None, Some(name.clone()), true),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Checkout a specific commit (detached HEAD).
    pub fn checkout_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Checkout,
            format!("Checking out {}...", short_id),
            None,
            Some(short_id.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Checkout")?;
                    let commit = repo.find_commit(oid)?;
                    let obj = commit.into_object();
                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.safe();
                    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                    repo.set_head_detached(oid)?;
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
                                GitOperationKind::Checkout,
                                format!("Checked out {}", short_id),
                                (
                                    Some("HEAD is now detached at the selected commit.".into()),
                                    None,
                                    Some(short_id.clone()),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checkout of {} failed", short_id),
                                e.to_string(),
                                (None, Some(short_id.clone()), true),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Checkout a tag, putting HEAD in detached state.
    pub fn checkout_tag(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Checkout,
            format!("Checking out tag '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Checkout")?;
                    let obj = repo.revparse_single(&format!("refs/tags/{}", task_name))?;
                    let commit = obj.peel_to_commit()?;
                    let oid = commit.id();
                    let obj = commit.into_object();
                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.safe();
                    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
                    repo.set_head_detached(oid)?;
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
                                GitOperationKind::Checkout,
                                format!("Checked out tag '{}'", name),
                                (
                                    Some("HEAD is now detached at the selected tag.".into()),
                                    None,
                                    Some(name.clone()),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Checkout,
                                format!("Checkout of tag '{}' failed", name),
                                e.to_string(),
                                (None, Some(name.clone()), true),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Create a new branch from HEAD.
    pub fn create_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        self.create_branch_at(name, None, cx)
    }

    /// Create a new branch, optionally at a specific commit (SHA or ref).
    /// If `base_ref` is None or empty, creates at HEAD.
    pub fn create_branch_at(
        &mut self,
        name: &str,
        base_ref: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        let base_ref = base_ref.map(|s| s.to_string());
        let task_name = name.clone();
        let task_base_ref = base_ref.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Creating branch '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let target = if let Some(ref r) = task_base_ref {
                        if r.is_empty() {
                            repo.head()?.peel_to_commit()?
                        } else {
                            let obj = repo.revparse_single(r)?;
                            obj.peel_to_commit().map_err(|_| {
                                anyhow::anyhow!("'{}' does not resolve to a commit", r)
                            })?
                        }
                    } else {
                        repo.head()?.peel_to_commit()?
                    };
                    repo.branch(&task_name, &target, false)?;
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
                                GitOperationKind::Branch,
                                format!("Created branch '{}'", name),
                                (
                                    base_ref.as_ref().map(|value| format!("Base: {}", value)),
                                    None,
                                    Some(name.clone()),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Branch '{}' could not be created", name),
                                e.to_string(),
                                (None, Some(name.clone()), false),
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

    /// Delete a local branch.
    pub fn delete_branch(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Deleting branch '{}'...", name),
            None,
            Some(name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut branch = repo.find_branch(&task_name, git2::BranchType::Local)?;
                    branch.delete()?;
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
                                GitOperationKind::Branch,
                                format!("Deleted branch '{}'", name),
                                (None, None, Some(name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Delete branch '{}' failed", name),
                                e.to_string(),
                                (None, Some(name.clone()), false),
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

    /// Rename a local branch.
    pub fn rename_branch(
        &mut self,
        old_name: &str,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let old_name = old_name.to_string();
        let new_name = new_name.to_string();
        let task_old_name = old_name.clone();
        let task_new_name = new_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Branch,
            format!("Renaming branch '{}'...", old_name),
            None,
            Some(old_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let mut branch = repo.find_branch(&task_old_name, git2::BranchType::Local)?;
                    branch.rename(&task_new_name, false)?;
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
                                GitOperationKind::Branch,
                                format!("Renamed '{}' to '{}'", old_name, new_name),
                                (None, None, Some(new_name.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Branch,
                                format!("Rename branch '{}' failed", old_name),
                                e.to_string(),
                                (None, Some(old_name.clone()), false),
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

    /// Create a lightweight tag at the given commit.
    pub fn create_tag(
        &mut self,
        name: &str,
        target_oid: git2::Oid,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Tag,
            format!("Creating tag '{}'...", name),
            None,
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let obj = repo.find_object(target_oid, None)?;
                    repo.tag_lightweight(&task_name, &obj, false)?;
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
                                GitOperationKind::Tag,
                                format!("Created tag '{}'", name),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Tag '{}' could not be created", name),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
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

    /// Delete a tag by name.
    pub fn delete_tag(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let task_name = name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Tag,
            format!("Deleting tag '{}'...", name),
            None,
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    repo.tag_delete(&task_name)?;
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
                                GitOperationKind::Tag,
                                format!("Deleted tag '{}'", name),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Tag,
                                format!("Delete tag '{}' failed", name),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
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

    /// Save the current working tree to a stash.
    pub fn stash_save(
        &mut self,
        message: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let message = message.map(String::from);
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            "Saving stash...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    let sig = repo.signature()?;
                    repo.stash_save(&sig, message.as_deref().unwrap_or("WIP"), None)?;
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
                                GitOperationKind::Stash,
                                "Saved stash",
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                "Save stash failed",
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

    /// Pop the top stash entry.
    pub fn stash_pop(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Popping stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_pop(index, None)?;
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
                                GitOperationKind::Stash,
                                format!("Popped stash #{}", index),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Pop stash #{} failed", index),
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

    /// Apply a stash entry without removing it from the stash list.
    pub fn stash_apply(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Applying stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_apply(index, None)?;
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
                                GitOperationKind::Stash,
                                format!("Applied stash #{}", index),
                                (
                                    Some("The stash entry was kept.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Apply stash #{} failed", index),
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

    /// Drop a stash entry without applying it.
    pub fn stash_drop(&mut self, index: usize, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!("Dropping stash #{}...", index),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;
                    repo.stash_drop(index)?;
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
                                GitOperationKind::Stash,
                                format!("Dropped stash #{}", index),
                                (None, None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Drop stash #{} failed", index),
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

    /// Create a branch from a stash entry and apply the stash to it.
    /// Equivalent to `git stash branch <branchname>`.
    pub fn stash_branch(
        &mut self,
        branch_name: &str,
        stash_index: usize,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name_owned = branch_name.to_string();
        let current_branch = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Stash,
            format!(
                "Creating branch '{}' from stash #{}...",
                branch_name_owned, stash_index
            ),
            None,
            current_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // Clone for cx.update closures (see below)
            let branch_name_for_update = branch_name_owned.clone();
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let mut repo = Repository::open(&repo_path)?;

                    // Collect stash OIDs to find the one at stash_index.
                    let mut stash_oids: Vec<git2::Oid> = Vec::new();
                    repo.stash_foreach(|_idx, _msg, oid| {
                        stash_oids.push(*oid);
                        true
                    })?;

                    let stash_oid = *stash_oids.get(stash_index).ok_or_else(|| {
                        anyhow::anyhow!("Stash index {} out of range", stash_index)
                    })?;

                    // Create a new branch at the stash's commit.
                    // We must drop `commit` before calling `stash_apply` since the
                    // former borrows `repo` immutably and the latter needs a mutable borrow.
                    {
                        let commit = repo.find_commit(stash_oid)?;
                        repo.branch(&branch_name_owned, &commit, false)?;
                    }

                    // Apply the stash to the new branch.
                    repo.stash_apply(stash_index, None)?;

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
                                GitOperationKind::Stash,
                                format!(
                                    "Created branch '{}' from stash #{}",
                                    branch_name_for_update, stash_index
                                ),
                                (None, None, Some(branch_name_for_update.clone())),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Stash,
                                format!("Create branch from stash #{} failed", stash_index),
                                e.to_string(),
                                (None, current_branch, false),
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

    /// Discard changes in specific files (restore to HEAD).
    pub fn discard_changes(
        &mut self,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let paths = paths.to_vec();
        let operation_id = self.begin_operation(
            GitOperationKind::Discard,
            if paths.len() == 1 {
                format!("Discarding changes in {}...", paths[0].display())
            } else {
                format!("Discarding changes in {} files...", paths.len())
            },
            None,
            self.head_branch.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let workdir = repo
                        .workdir()
                        .ok_or_else(|| anyhow::anyhow!("Bare repository has no working directory"))?
                        .to_path_buf();
                    let mut checkout_opts = git2::build::CheckoutBuilder::new();
                    checkout_opts.force();
                    let mut has_tracked = false;
                    for path in &paths {
                        let is_untracked = repo
                            .status_file(path)
                            .map(|s| s.contains(git2::Status::WT_NEW))
                            .unwrap_or(false);
                        if is_untracked {
                            let full = workdir.join(path);
                            if full.is_file() {
                                std::fs::remove_file(&full).with_context(|| {
                                    format!("Failed to delete {}", full.display())
                                })?;
                            } else if full.is_dir() {
                                std::fs::remove_dir_all(&full).with_context(|| {
                                    format!("Failed to delete directory {}", full.display())
                                })?;
                            }
                        } else {
                            checkout_opts.path(path);
                            has_tracked = true;
                        }
                    }
                    if has_tracked {
                        repo.checkout_head(Some(&mut checkout_opts))?;
                    }
                    let data = gather_refresh_data(&repo_path)?;
                    Ok::<_, anyhow::Error>(data)
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
                                "Discarded changes",
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Discard,
                                "Discard changes failed",
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Hard reset to HEAD, discarding all working tree and index changes.
    pub fn reset_hard(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            "Resetting working tree to HEAD...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let head_commit = repo.head()?.peel_to_commit()?;
                    repo.reset(head_commit.as_object(), git2::ResetType::Hard, None)?;
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
                                GitOperationKind::Reset,
                                "Reset working tree to HEAD",
                                (
                                    Some("All staged and unstaged changes were discarded.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                "Reset to HEAD failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Hard-reset the current branch to a specific commit.
    pub fn reset_to_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            format!("Resetting to {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let commit = repo.find_commit(oid)?;
                    repo.reset(commit.as_object(), git2::ResetType::Hard, None)?;
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
                                GitOperationKind::Reset,
                                format!("Reset to {}", short_id),
                                (
                                    Some("Working tree reset to the selected commit.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                format!("Reset to {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Soft-reset the current branch to a specific commit, preserving changes in the index.
    pub fn reset_soft(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            format!("Soft-resetting to {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let commit = repo.find_commit(oid)?;
                    repo.reset(commit.as_object(), git2::ResetType::Soft, None)?;
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
                                GitOperationKind::Reset,
                                format!("Soft reset to {}", short_id),
                                (
                                    Some("Changes preserved in index.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                format!("Soft reset to {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Mixed-reset the current branch to a specific commit, unstaging all changes.
    pub fn reset_mixed(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Reset,
            format!("Mixed-resetting to {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let commit = repo.find_commit(oid)?;
                    repo.reset(commit.as_object(), git2::ResetType::Mixed, None)?;
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
                                GitOperationKind::Reset,
                                format!("Mixed reset to {}", short_id),
                                (
                                    Some("Changes unstaged; index and working tree reset.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Reset,
                                format!("Mixed reset to {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Revert a commit (creates a new commit that undoes the given commit).
    pub fn revert_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Revert,
            format!("Reverting {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData, bool)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Revert")?;
                    let commit = repo.find_commit(oid)?;
                    let summary = commit.summary().unwrap_or("").to_string();
                    let mut opts = git2::RevertOptions::new();
                    repo.revert(&commit, Some(&mut opts))?;
                    let has_conflicts = repo.index()?.has_conflicts();
                    if !has_conflicts {
                        repo.cleanup_state()?;
                    }
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data, has_conflicts))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data, has_conflicts)) => {
                            this.apply_refresh_data(data);
                            cx.emit(GitProjectEvent::StatusChanged);
                            if has_conflicts {
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Revert,
                                    format!("Revert of {} needs conflict resolution", short_id),
                                    "Resolve the conflicts in the working tree, then commit the revert manually.".to_string(),
                                    (None, branch_name.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Revert,
                                    format!("Reverted {}", short_id),
                                    (Some(format!(
                                        "Revert for '{}' has been applied. Review the changes and commit them manually.",
                                        summary
                                    )), None, branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Revert,
                                format!("Revert of {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Cherry-pick a commit onto the current HEAD.
    pub fn cherry_pick(&mut self, oid: git2::Oid, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid.to_string()[..7].to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::CherryPick,
            format!("Cherry-picking {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData, bool)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    ensure_clean_worktree(&repo, "Cherry-pick")?;
                    let commit = repo.find_commit(oid)?;
                    let summary = commit.summary().unwrap_or("").to_string();
                    let mut opts = git2::CherrypickOptions::new();
                    repo.cherrypick(&commit, Some(&mut opts))?;
                    let has_conflicts = repo.index()?.has_conflicts();
                    if !has_conflicts {
                        repo.cleanup_state()?;
                    }
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data, has_conflicts))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data, has_conflicts)) => {
                            this.apply_refresh_data(data);
                            cx.emit(GitProjectEvent::StatusChanged);
                            if has_conflicts {
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::CherryPick,
                                    format!("Cherry-pick of {} needs conflict resolution", short_id),
                                    "Resolve the conflicts in the working tree, then commit the cherry-pick manually.".to_string(),
                                    (None, branch_name.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::CherryPick,
                                    format!("Cherry-picked {}", short_id),
                                    (Some(format!(
                                        "Cherry-pick for '{}' has been applied. Review the changes and commit them manually.",
                                        summary
                                    )), None, branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::CherryPick,
                                format!("Cherry-pick of {} failed", short_id),
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    /// Abort the current in-progress operation (merge, rebase, cherry-pick, revert).
    /// Resets the working tree and index to HEAD and cleans up the repo state.
    pub fn abort_operation(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let state_label = self.repo_state.label().to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge,
            format!("Aborting {}...", state_label.to_lowercase()),
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    let head = repo.head()?.peel_to_commit()?;
                    repo.reset(
                        head.as_object(),
                        git2::ResetType::Hard,
                        Some(git2::build::CheckoutBuilder::new().force()),
                    )?;
                    repo.cleanup_state()?;
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
                                GitOperationKind::Merge,
                                format!("{} aborted", state_label),
                                (
                                    Some("Working tree has been reset to HEAD.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                format!("Failed to abort {}", state_label.to_lowercase()),
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

    /// Continue the current merge by committing with the default merge message.
    /// This stages all files and creates the merge commit.
    pub fn continue_merge(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge,
            "Continuing merge...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;

                    let state = repo.state();
                    if state == git2::RepositoryState::Clean {
                        anyhow::bail!("Repository is not in a merge state");
                    }

                    let mut index = repo.index()?;
                    if index.has_conflicts() {
                        anyhow::bail!(
                            "There are still unresolved conflicts. Resolve all conflicts before continuing."
                        );
                    }

                    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
                    index.write()?;
                    let tree_oid = index.write_tree()?;
                    let tree = repo.find_tree(tree_oid)?;

                    let sig = repo.signature()?;
                    let head_commit = repo.head()?.peel_to_commit()?;

                    let merge_msg_path = repo.path().join("MERGE_MSG");
                    let message = if merge_msg_path.exists() {
                        std::fs::read_to_string(&merge_msg_path)
                            .unwrap_or_else(|_| "Merge commit".to_string())
                    } else {
                        "Merge commit".to_string()
                    };

                    let mut parents = vec![head_commit.clone()];
                    let merge_head_path = repo.path().join("MERGE_HEAD");
                    if merge_head_path.exists() {
                        let contents = std::fs::read_to_string(&merge_head_path)?;
                        for line in contents.lines() {
                            let line = line.trim();
                            if !line.is_empty() {
                                let oid = git2::Oid::from_str(line)?;
                                parents.push(repo.find_commit(oid)?);
                            }
                        }
                    }

                    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parent_refs)?;
                    repo.cleanup_state()?;

                    let summary = message.lines().next().unwrap_or("Merge commit").to_string();
                    let data = gather_refresh_data(&repo_path)?;
                    Ok((summary, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Merge,
                                "Merge completed",
                                (Some(summary), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                "Continue merge failed",
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

    /// Merge a branch into the current HEAD.
    pub fn merge_branch(&mut self, branch_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let branch_name = branch_name.to_string();
        let task_branch_name = branch_name.clone();
        let repo_path = self.repo_path.clone();
        let current_branch = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Merge,
            format!("Merging '{}'...", branch_name),
            None,
            current_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let msg = {
                        let repo = Repository::open(&repo_path)?;
                        ensure_clean_worktree(&repo, "Merge")?;

                        let reference = repo
                            .find_branch(&task_branch_name, git2::BranchType::Local)
                            .or_else(|_| {
                                repo.find_branch(&task_branch_name, git2::BranchType::Remote)
                            })?;
                        let annotated_commit =
                            repo.reference_to_annotated_commit(reference.get())?;

                        let (analysis, _pref) = repo.merge_analysis(&[&annotated_commit])?;

                        if analysis.is_up_to_date() {
                            "Already up to date".to_string()
                        } else if analysis.is_fast_forward() {
                            let head = repo.head()?;
                            let head_branch_name =
                                head.shorthand().unwrap_or("HEAD").to_string();
                            let refname = format!("refs/heads/{}", head_branch_name);
                            let mut reference = repo.find_reference(&refname)?;
                            reference.set_target(
                                annotated_commit.id(),
                                &format!("Fast-forward merge of '{}'", task_branch_name),
                            )?;
                            repo.set_head(&refname)?;
                            repo.checkout_head(Some(
                                git2::build::CheckoutBuilder::new().force(),
                            ))?;
                            format!("Merged '{}' (fast-forward)", task_branch_name)
                        } else if analysis.is_normal() {
                            repo.merge(&[&annotated_commit], None, None)?;

                            let has_conflicts = repo.index()?.has_conflicts();
                            if has_conflicts {
                                let conflict_count = repo
                                    .index()?
                                    .conflicts()?
                                    .count();
                                format!(
                                    "CONFLICT:{} conflict(s) detected merging '{}'. Resolve and continue.",
                                    conflict_count, task_branch_name
                                )
                            } else {
                                let sig = repo.signature()?;
                                let mut index = repo.index()?;
                                let tree_oid = index.write_tree()?;
                                let tree = repo.find_tree(tree_oid)?;
                                let head_commit = repo.head()?.peel_to_commit()?;
                                let merge_commit =
                                    repo.find_commit(annotated_commit.id())?;
                                repo.commit(
                                    Some("HEAD"),
                                    &sig,
                                    &sig,
                                    &format!(
                                        "Merge branch '{}' into {}",
                                        task_branch_name,
                                        repo.head()?
                                            .shorthand()
                                            .unwrap_or("HEAD")
                                    ),
                                    &tree,
                                    &[&head_commit, &merge_commit],
                                )?;
                                repo.cleanup_state()?;
                                format!("Merged '{}' successfully", task_branch_name)
                            }
                        } else {
                            "Merge complete".to_string()
                        }
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((msg, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, data)) => {
                            let is_conflict = msg.starts_with("CONFLICT:");
                            this.apply_refresh_data(data);
                            if is_conflict {
                                let user_msg = msg.trim_start_matches("CONFLICT:").to_string();
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Merge,
                                    format!("Merge conflicts in '{}'", branch_name),
                                    user_msg,
                                    (None, current_branch.clone(), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Merge,
                                    msg,
                                    (Some("Repository state refreshed after merge.".into()), None, current_branch.clone()),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Merge,
                                format!("Merge of '{}' failed", branch_name),
                                e.to_string(),
                                (None, current_branch.clone(), false),
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

    /// Remove a remote by name.
    pub fn remove_remote(&mut self, name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        let name = name.to_string();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::RemoveRemote,
            format!("Removing remote '{}'...", name),
            Some(name.clone()),
            branch_name.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;
                    repo.remote_delete(&name)?;
                    let data = gather_refresh_data(&repo_path)?;
                    Ok::<_, anyhow::Error>((data, name))
                })
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((data, name)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::RemoveRemote,
                                format!("Removed remote '{}'", name),
                                (
                                    Some("Remote list refreshed.".into()),
                                    Some(name.clone()),
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.notify();
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::RemoveRemote,
                                "Removing remote failed",
                                e.to_string(),
                                (None, branch_name.clone(), false),
                                cx,
                            );
                        }
                    }
                    Ok(())
                })
            })?
        })
    }

    // ============================================================================
    // Bisect Operations
    // ============================================================================

    /// Start a bisect session to find a commit that introduced a bug.
    /// After starting, mark commits as good/bad with bisect_good/bisect_bad.
    pub fn bisect_start(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Bisect,
            "Starting bisect...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let output = Command::new("git")
                        .current_dir(&repo_path)
                        .args(["bisect", "start"])
                        .output()
                        .context("Failed to execute git bisect start")?;

                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !output.status.success() {
                        anyhow::bail!("git bisect start failed: {}", stderr.trim());
                    }

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
                                GitOperationKind::Bisect,
                                "Bisect started".to_string(),
                                (
                                    Some("Mark commits as 'good' or 'bad' to narrow down the problematic commit.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                "Failed to start bisect",
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

    /// Mark the specified commit (or current HEAD if None) as "good" during bisect.
    pub fn bisect_good(
        &mut self,
        oid: Option<git2::Oid>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid
            .map(|o| o.to_string()[..7].to_string())
            .unwrap_or_else(|| "HEAD".to_string());
        let operation_id = self.begin_operation(
            GitOperationKind::Bisect,
            format!("Marking {} as good...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        let oid_str = oid.map(|o| o.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let mut cmd = Command::new("git");
                    cmd.current_dir(&repo_path).args(["bisect", "good"]);
                    if let Some(ref oid) = oid_str {
                        cmd.arg(oid);
                    }
                    let output = cmd
                        .output()
                        .context("Failed to execute git bisect good")?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if !output.status.success() {
                        anyhow::bail!("git bisect good failed: {}", stderr.trim());
                    }

                    // Check if bisect found the culprit
                    let found_match = stdout.contains("is the first bad commit");
                    let message = if found_match {
                        Some(stdout.lines().take(10).collect::<Vec<_>>().join("\n"))
                    } else {
                        None
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((message, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((Some(found_msg), data)) => {
                            // Bisect found the bad commit
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                "Bisect complete!".to_string(),
                                (
                                    Some(format!("Found the first bad commit:\n{}", found_msg)),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Ok((None, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Marked {} as good", short_id),
                                (
                                    Some("Bisect continues. Test the current commit and mark as good/bad.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Failed to mark {} as good", short_id),
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

    /// Mark the specified commit (or current HEAD if None) as "bad" during bisect.
    pub fn bisect_bad(
        &mut self,
        oid: Option<git2::Oid>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid
            .map(|o| o.to_string()[..7].to_string())
            .unwrap_or_else(|| "HEAD".to_string());
        let operation_id = self.begin_operation(
            GitOperationKind::Bisect,
            format!("Marking {} as bad...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        let oid_str = oid.map(|o| o.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let mut cmd = Command::new("git");
                    cmd.current_dir(&repo_path).args(["bisect", "bad"]);
                    if let Some(ref oid) = oid_str {
                        cmd.arg(oid);
                    }
                    let output = cmd
                        .output()
                        .context("Failed to execute git bisect bad")?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if !output.status.success() {
                        anyhow::bail!("git bisect bad failed: {}", stderr.trim());
                    }

                    // Check if bisect found the culprit
                    let found_match = stdout.contains("is the first bad commit");
                    let message = if found_match {
                        Some(stdout.lines().take(10).collect::<Vec<_>>().join("\n"))
                    } else {
                        None
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((message, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((Some(found_msg), data)) => {
                            // Bisect found the bad commit
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                "Bisect complete!".to_string(),
                                (
                                    Some(format!("Found the first bad commit:\n{}", found_msg)),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Ok((None, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Marked {} as bad", short_id),
                                (
                                    Some("Bisect continues. Test the current commit and mark as good/bad.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Failed to mark {} as bad", short_id),
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

    /// Mark the current commit (or specified commit) as skipped during bisect.
    /// Skipped commits are excluded from the bisect search.
    pub fn bisect_skip(
        &mut self,
        oid: Option<git2::Oid>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let short_id = oid
            .map(|o| o.to_string()[..7].to_string())
            .unwrap_or_else(|| "HEAD".to_string());
        let operation_id = self.begin_operation(
            GitOperationKind::Bisect,
            format!("Skipping {}...", short_id),
            None,
            branch_name.clone(),
            cx,
        );
        let oid_str = oid.map(|o| o.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let mut cmd = Command::new("git");
                    cmd.current_dir(&repo_path).args(["bisect", "skip"]);
                    if let Some(ref oid) = oid_str {
                        cmd.arg(oid);
                    }
                    let output = cmd.output().context("Failed to execute git bisect skip")?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if !output.status.success() {
                        anyhow::bail!("git bisect skip failed: {}", stderr.trim());
                    }

                    // Check if bisect can no longer continue (only skipped commits remain)
                    let exhausted = stdout.contains("only skipped commits left to test")
                        || stderr.contains("only skipped commits left to test");
                    let message = if exhausted {
                        Some(
                            "Bisect cannot continue: only skipped commits remain.\n\
                             Consider using 'Bisect Reset' and manually narrowing down."
                                .into(),
                        )
                    } else {
                        // git bisect skip outputs lines like:
                        // "Skipping commit <sha>"
                        // "Bisecting: N commits left to test"
                        let lines: Vec<_> = stdout.lines().filter(|l| !l.is_empty()).collect();
                        if lines.is_empty() {
                            None
                        } else {
                            Some(lines.join("\n"))
                        }
                    };

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((message, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((Some(msg), data)) if msg.contains("cannot continue") => {
                            this.apply_refresh_data(data);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                "Bisect exhausted".to_string(),
                                msg,
                                (None, branch_name.clone(), false),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Ok((Some(msg), data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Skipped {}", short_id),
                                (Some(msg), None, branch_name.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Ok((None, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Skipped {}", short_id),
                                (
                                    Some("Bisect continues. Test the current commit.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                format!("Failed to skip {}", short_id),
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

    /// Reset the bisect session and return to the original branch/commit.
    pub fn bisect_reset(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.head_branch.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Bisect,
            "Resetting bisect...",
            None,
            branch_name.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<RefreshData> = cx
                .background_executor()
                .spawn(async move {
                    let output = Command::new("git")
                        .current_dir(&repo_path)
                        .args(["bisect", "reset"])
                        .output()
                        .context("Failed to execute git bisect reset")?;

                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !output.status.success() {
                        anyhow::bail!("git bisect reset failed: {}", stderr.trim());
                    }

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
                                GitOperationKind::Bisect,
                                "Bisect reset".to_string(),
                                (
                                    Some("Returned to original branch/commit.".into()),
                                    None,
                                    branch_name.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Bisect,
                                "Failed to reset bisect",
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

    /// Create a new Git worktree.
    pub fn create_worktree(
        &mut self,
        name: String,
        path: PathBuf,
        branch: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let name_clone = name.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Worktree,
            format!("Creating worktree '{}'...", name),
            None,
            self.head_branch.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let repo = Repository::open(&repo_path)?;

                    // Resolve branch reference before building options (lifetime constraint).
                    let reference = if let Some(ref branch_name) = branch {
                        repo.find_branch(branch_name, git2::BranchType::Local)
                            .ok()
                            .map(|b| b.into_reference())
                    } else {
                        None
                    };

                    let mut opts = git2::WorktreeAddOptions::new();
                    if let Some(ref r) = reference {
                        opts.reference(Some(r));
                    }

                    repo.worktree(&name, &path, Some(&opts))?;
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
                                GitOperationKind::Worktree,
                                format!("Created worktree '{}'", name_clone),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Worktree,
                                format!("Create worktree '{}' failed", name_clone),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
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

    /// Remove a Git worktree.
    pub fn remove_worktree(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Task<Result<()>> {
        let display_path = path.display().to_string();
        let operation_id = self.begin_operation(
            GitOperationKind::Worktree,
            format!("Removing worktree '{}'...", display_path),
            None,
            self.head_branch.clone(),
            cx,
        );
        let repo_path = self.repo_path.clone();
        let display_path_async = display_path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let output = Command::new("git")
                        .current_dir(&repo_path)
                        .args(["worktree", "remove", "--force", &display_path_async])
                        .output()
                        .context("Failed to execute git worktree remove")?;

                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !output.status.success() {
                        anyhow::bail!("git worktree remove failed: {}", stderr.trim());
                    }

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
                                GitOperationKind::Worktree,
                                format!("Removed worktree '{}'", display_path),
                                (None, None, this.head_branch.clone()),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Worktree,
                                format!("Remove worktree '{}' failed", display_path),
                                e.to_string(),
                                (None, this.head_branch.clone(), false),
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

fn sign_with_gpg(content: &str, key_id: &str) -> Result<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("gpg");
    cmd.args(["--status-fd=2", "-bsau", key_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .context("Failed to start gpg. Is GPG installed?")?;

    child.stdin.take().unwrap().write_all(content.as_bytes())?;
    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("GPG signing failed: {}", stderr);
    }

    Ok(String::from_utf8(output.stdout)?)
}
