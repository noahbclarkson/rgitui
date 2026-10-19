use super::messages::error_message;
use anyhow::{Context as _, Result};
use git2::Repository;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use rgitui_i18n::{msg, msgp, Message};
use std::path::Path;

use crate::types::*;

use super::argsafe::{validate_branch_name, validate_remote_name};
use super::auth::run_git_network_command;
use super::refresh::gather_refresh_data;
use super::{
    ensure_clean_worktree, head_branch_name, pull_target, push_target, GitProject, GitProjectEvent,
    RefreshData,
};

/// Open the repository handle for a specific working tree. Network operations
/// resolve their branch and upstream from *this* checkout, which is what makes
/// pushing from an inspected worktree push that worktree's branch rather than
/// the branch the main checkout happens to be on.
fn open_worktree_repo(worktree_path: &Path) -> Result<Repository> {
    Repository::open(worktree_path)
        .with_context(|| format!("Failed to open worktree at {}", worktree_path.display()))
}

impl GitProject {
    pub fn fetch_default_at(
        &mut self,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!("fetch_default: worktree={}", worktree_path.display());
        let remote_name = match self.preferred_remote_name_at(worktree_path) {
            Ok(remote_name) => remote_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Fetch,
                    msg!("git.fetch.not_started"),
                    error,
                    true,
                    worktree_path,
                    cx,
                )
            }
        };
        self.fetch_at(&remote_name, worktree_path, cx)
    }

    pub fn pull_default_at(
        &mut self,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!("pull_default: worktree={}", worktree_path.display());
        let (remote_name, branch_name) =
            match open_worktree_repo(worktree_path).and_then(|repo| pull_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::Pull,
                        msg!("git.pull.not_started"),
                        error,
                        true,
                        worktree_path,
                        cx,
                    )
                }
            };
        self.pull_from_at(&remote_name, &branch_name, worktree_path, cx)
    }

    pub fn push_default_at(
        &mut self,
        force: bool,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "push_default: force={} worktree={}",
            force,
            worktree_path.display()
        );
        let (remote_name, branch_name, set_upstream) =
            match open_worktree_repo(worktree_path).and_then(|repo| push_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::push(force),
                        msg!("git.push.not_started"),
                        error,
                        true,
                        worktree_path,
                        cx,
                    )
                }
            };
        self.push_to_at(
            &remote_name,
            &branch_name,
            force,
            set_upstream,
            worktree_path,
            cx,
        )
    }

    /// Fetch from a remote, running git in the given worktree.
    pub fn fetch_at(
        &mut self,
        remote_name: &str,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "fetch: remote={} worktree={}",
            remote_name,
            worktree_path.display()
        );
        let remote_name = remote_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_worktree_path = worktree_path.to_path_buf();
        let repo_path = self.repo_path.clone();
        let commit_limit = self.commit_limit;
        let operation_id = self.begin_operation(
            GitOperationKind::Fetch,
            msg!("git.fetch.running", remote = &remote_name),
            Some(remote_name.clone()),
            self.head_branch_at(worktree_path),
            cx,
        );
        self.note_operation_worktree(operation_id, worktree_path);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    validate_remote_name(&task_remote_name)?;
                    let details = run_git_network_command(
                        &task_worktree_path,
                        &["fetch", "--prune", &task_remote_name],
                    )?;
                    let data = gather_refresh_data(&repo_path, commit_limit)?;
                    anyhow::Ok((details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((details, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                GitOperationKind::Fetch,
                                msg!("git.fetch.done", remote = &remote_name),
                                (
                                    details
                                        .map(Message::verbatim)
                                        .or(Some(msg!("git.fetch.detail"))),
                                    Some(remote_name.clone()),
                                    this.head_branch.clone(),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            log::error!("Fetch failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Fetch,
                                msg!("git.fetch.failed", remote = &remote_name),
                                error_message(&e),
                                (Some(remote_name.clone()), this.head_branch.clone(), true),
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

    /// Pull into the given worktree, merging into the branch *that* worktree
    /// has checked out.
    pub fn pull_at(
        &mut self,
        remote_name: &str,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "pull: remote={} worktree={}",
            remote_name,
            worktree_path.display()
        );
        let branch_name = match open_worktree_repo(worktree_path)
            .and_then(|repo| pull_target(&repo, Some(remote_name)).map(|(_, branch)| branch))
        {
            Ok(branch_name) => branch_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Pull,
                    msg!("git.pull.not_started"),
                    error,
                    true,
                    worktree_path,
                    cx,
                )
            }
        };
        self.pull_from_at(remote_name, &branch_name, worktree_path, cx)
    }

    pub fn pull_from_at(
        &mut self,
        remote_name: &str,
        branch_name: &str,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "pull_from: remote={} branch={} worktree={}",
            remote_name,
            branch_name,
            worktree_path.display()
        );
        let remote_name = remote_name.to_string();
        let branch_name = branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_branch_name = branch_name.clone();
        let task_worktree_path = worktree_path.to_path_buf();
        let repo_path = self.repo_path.clone();
        let commit_limit = self.commit_limit;
        let operation_id = self.begin_operation(
            GitOperationKind::Pull,
            msg!(
                "git.pull.running",
                branch = &branch_name,
                remote = &remote_name
            ),
            Some(remote_name.clone()),
            Some(branch_name.clone()),
            cx,
        );
        self.note_operation_worktree(operation_id, worktree_path);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Message, bool, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, is_conflict, details) = {
                        let repo = Repository::open(&task_worktree_path)?;
                        ensure_clean_worktree(&repo, GitOperationKind::Pull.name())?;
                        drop(repo);

                        validate_remote_name(&task_remote_name)?;
                        validate_branch_name(&task_branch_name)?;
                        match run_git_network_command(
                            &task_worktree_path,
                            &["pull", &task_remote_name, &task_branch_name],
                        ) {
                            Ok(details) => {
                                let msg = if details
                                    .as_deref()
                                    .map(|text| text.contains("Already up to date."))
                                    .unwrap_or(false)
                                {
                                    msg!(
                                        "git.pull.up_to_date",
                                        branch = &task_branch_name,
                                        remote = &task_remote_name
                                    )
                                } else {
                                    msg!(
                                        "git.pull.done",
                                        branch = &task_branch_name,
                                        remote = &task_remote_name
                                    )
                                };
                                (msg, false, details)
                            }
                            Err(e) => {
                                let error_msg = e.to_string();
                                if error_msg.contains("CONFLICT")
                                    || error_msg.contains("Automatic merge failed")
                                {
                                    let repo = Repository::open(&task_worktree_path)?;
                                    let conflict_count = repo.index()?.conflicts()?.count();
                                    let msg = msgp!(
                                        "git.pull.conflicts_detail",
                                        conflict_count,
                                        remote = &task_remote_name,
                                        branch = &task_branch_name
                                    );
                                    (msg, true, Some(error_msg))
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    }; // repo dropped here

                    let data = gather_refresh_data(&repo_path, commit_limit)?;
                    Ok((msg, is_conflict, details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, is_conflict, details, data)) => {
                            this.apply_refresh_data(data);
                            if is_conflict {
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    msg!("git.pull.conflict", remote = &remote_name),
                                    msg,
                                    (Some(remote_name.clone()), Some(branch_name.clone()), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    msg,
                                    (
                                        details
                                            .map(Message::verbatim)
                                            .or(Some(msg!("git.pull.detail"))),
                                        Some(remote_name.clone()),
                                        Some(branch_name.clone()),
                                    ),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::RepositoryChanged);
                        }
                        Err(e) => {
                            log::error!("Pull failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Pull,
                                msg!("git.pull.failed", remote = &remote_name),
                                error_message(&e),
                                (Some(remote_name.clone()), Some(branch_name.clone()), true),
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

    /// Push the branch checked out in the given worktree.
    pub fn push_at(
        &mut self,
        remote_name: &str,
        force: bool,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "push: remote={} force={} worktree={}",
            remote_name,
            force,
            worktree_path.display()
        );
        let (branch_name, set_upstream) = match open_worktree_repo(worktree_path).and_then(|repo| {
            push_target(&repo, Some(remote_name)).map(|(_, branch, set)| (branch, set))
        }) {
            Ok(target) => target,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::push(force),
                    msg!("git.push.not_started"),
                    error,
                    true,
                    worktree_path,
                    cx,
                )
            }
        };
        self.push_to_at(
            remote_name,
            &branch_name,
            force,
            set_upstream,
            worktree_path,
            cx,
        )
    }

    pub fn push_to_at(
        &mut self,
        remote_name: &str,
        remote_branch_name: &str,
        force: bool,
        set_upstream: bool,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "push_to: remote={} branch={} force={} worktree={}",
            remote_name,
            remote_branch_name,
            force,
            worktree_path.display()
        );
        let remote_name = remote_name.to_string();
        let remote_branch_name = remote_branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_remote_branch_name = remote_branch_name.clone();
        let task_worktree_path = worktree_path.to_path_buf();
        let repo_path = self.repo_path.clone();
        let commit_limit = self.commit_limit;
        let kind = GitOperationKind::push(force);
        let operation_id = self.begin_operation(
            kind,
            if force {
                msg!(
                    "git.force_push.running",
                    remote = &remote_name,
                    branch = &remote_branch_name
                )
            } else {
                msg!(
                    "git.push.running",
                    remote = &remote_name,
                    branch = &remote_branch_name
                )
            },
            Some(remote_name.clone()),
            Some(remote_branch_name.clone()),
            cx,
        );
        self.note_operation_worktree(operation_id, worktree_path);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Message, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, details) = {
                        let repo = Repository::open(&task_worktree_path)?;
                        let branch_name = head_branch_name(&repo)?;
                        drop(repo);

                        validate_remote_name(&task_remote_name)?;
                        validate_branch_name(&task_remote_branch_name)?;

                        let mut args = vec!["push"];
                        if force {
                            args.push("--force");
                        }
                        if set_upstream {
                            args.push("--set-upstream");
                        }
                        args.push(task_remote_name.as_str());
                        let refspec = format!("HEAD:{}", task_remote_branch_name);
                        args.push(refspec.as_str());
                        let details = run_git_network_command(&task_worktree_path, &args)?;
                        let msg = if force {
                            msg!(
                                "git.force_push.done",
                                branch = &branch_name,
                                remote = &task_remote_name,
                                remote_branch = &task_remote_branch_name
                            )
                        } else if set_upstream {
                            msg!(
                                "git.push.done_set_upstream",
                                branch = &branch_name,
                                remote = &task_remote_name,
                                remote_branch = &task_remote_branch_name
                            )
                        } else {
                            msg!(
                                "git.push.done",
                                branch = &branch_name,
                                remote = &task_remote_name,
                                remote_branch = &task_remote_branch_name
                            )
                        };
                        (msg, details)
                    };
                    let data = gather_refresh_data(&repo_path, commit_limit)?;
                    Ok((msg, details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, details, data)) => {
                            this.apply_refresh_data(data);
                            this.complete_op(
                                operation_id,
                                kind,
                                msg,
                                (
                                    details
                                        .map(Message::verbatim)
                                        .or(Some(msg!("git.push.detail"))),
                                    Some(remote_name.clone()),
                                    Some(remote_branch_name.clone()),
                                ),
                                cx,
                            );
                            cx.emit(GitProjectEvent::RefsChanged);
                        }
                        Err(e) => {
                            log::error!("Push failed: {}", e);
                            this.fail_op(
                                operation_id,
                                kind,
                                if force {
                                    msg!("git.force_push.failed", remote = &remote_name)
                                } else {
                                    msg!("git.push.failed", remote = &remote_name)
                                },
                                error_message(&e),
                                (
                                    Some(remote_name.clone()),
                                    Some(remote_branch_name.clone()),
                                    true,
                                ),
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
