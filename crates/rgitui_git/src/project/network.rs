use anyhow::Result;
use git2::Repository;
use gpui::{AsyncApp, Context, Task, WeakEntity};

use crate::types::*;

use super::auth::run_git_network_command;
use super::refresh::gather_refresh_data;
use super::{
    ensure_clean_worktree, head_branch_name, pull_target, push_target, GitProject, GitProjectEvent,
    RefreshData,
};

impl GitProject {
    pub fn fetch_default(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        log::info!("fetch_default");
        let remote_name = match self.preferred_remote_name() {
            Ok(remote_name) => remote_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Fetch,
                    "Fetch could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.fetch(&remote_name, cx)
    }

    pub fn pull_default(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        log::info!("pull_default");
        let (remote_name, branch_name) =
            match self.open_repo().and_then(|repo| pull_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::Pull,
                        "Pull could not start",
                        error,
                        true,
                        cx,
                    )
                }
            };
        self.pull_from(&remote_name, &branch_name, cx)
    }

    pub fn push_default(&mut self, force: bool, cx: &mut Context<Self>) -> Task<Result<()>> {
        log::info!("push_default: force={}", force);
        let (remote_name, branch_name, set_upstream) =
            match self.open_repo().and_then(|repo| push_target(&repo, None)) {
                Ok(target) => target,
                Err(error) => {
                    return self.fail_to_start_task(
                        GitOperationKind::Push,
                        "Push could not start",
                        error,
                        true,
                        cx,
                    )
                }
            };
        self.push_to(&remote_name, &branch_name, force, set_upstream, cx)
    }

    /// Fetch from a remote.
    pub fn fetch(&mut self, remote_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        log::info!("fetch: remote={}", remote_name);
        let remote_name = remote_name.to_string();
        let task_remote_name = remote_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Fetch,
            format!("Fetching from '{}'...", remote_name),
            Some(remote_name.clone()),
            self.head_branch.clone(),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let details = run_git_network_command(
                        &repo_path,
                        &["fetch", "--prune", &task_remote_name],
                    )?;
                    let data = gather_refresh_data(&repo_path)?;
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
                                format!("Fetched from '{}'", remote_name),
                                (
                                    details.or(Some("Remote refs refreshed.".into())),
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
                                format!("Fetch from '{}' failed", remote_name),
                                e.to_string(),
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

    /// Pull from a remote (fetch + merge).
    pub fn pull(&mut self, remote_name: &str, cx: &mut Context<Self>) -> Task<Result<()>> {
        log::info!("pull: remote={}", remote_name);
        let branch_name = match self
            .open_repo()
            .and_then(|repo| pull_target(&repo, Some(remote_name)).map(|(_, branch)| branch))
        {
            Ok(branch_name) => branch_name,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Pull,
                    "Pull could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.pull_from(remote_name, &branch_name, cx)
    }

    pub fn pull_from(
        &mut self,
        remote_name: &str,
        branch_name: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!("pull_from: remote={} branch={}", remote_name, branch_name);
        let remote_name = remote_name.to_string();
        let branch_name = branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_branch_name = branch_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Pull,
            format!("Pulling '{}' from '{}'...", branch_name, remote_name),
            Some(remote_name.clone()),
            Some(branch_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, details) = {
                        let repo = Repository::open(&repo_path)?;
                        ensure_clean_worktree(&repo, "Pull")?;
                        drop(repo);

                        match run_git_network_command(
                            &repo_path,
                            &["pull", &task_remote_name, &task_branch_name],
                        ) {
                            Ok(details) => {
                                let msg = if details
                                    .as_deref()
                                    .map(|text| text.contains("Already up to date."))
                                    .unwrap_or(false)
                                {
                                    format!(
                                        "'{}' is already up to date with '{}'",
                                        task_branch_name, task_remote_name
                                    )
                                } else {
                                    format!(
                                        "Pulled '{}' from '{}'",
                                        task_branch_name, task_remote_name
                                    )
                                };
                                (msg, details)
                            }
                            Err(e) => {
                                let error_msg = e.to_string();
                                if error_msg.contains("CONFLICT")
                                    || error_msg.contains("Automatic merge failed")
                                {
                                    let repo = Repository::open(&repo_path)?;
                                    let conflict_count =
                                        repo.index()?.conflicts()?.count();
                                    let msg = format!(
                                        "CONFLICT:{} conflict(s) during pull from '{}/{}'. Resolve and continue.",
                                        conflict_count, task_remote_name, task_branch_name
                                    );
                                    (msg, Some(error_msg))
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    }; // repo dropped here

                    let data = gather_refresh_data(&repo_path)?;
                    Ok((msg, details, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok((msg, details, data)) => {
                            let is_conflict = msg.starts_with("CONFLICT:");
                            this.apply_refresh_data(data);
                            if is_conflict {
                                let user_msg = msg.trim_start_matches("CONFLICT:").to_string();
                                this.fail_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    format!("Pull from '{}' has conflicts", remote_name),
                                    user_msg,
                                    (Some(remote_name.clone()), Some(branch_name.clone()), false),
                                    cx,
                                );
                            } else {
                                this.complete_op(
                                    operation_id,
                                    GitOperationKind::Pull,
                                    msg,
                                    (details.or(Some(
                                        "Repository state refreshed after pull.".into(),
                                    )), Some(remote_name.clone()), Some(branch_name.clone())),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::HeadChanged);
                            cx.emit(GitProjectEvent::StatusChanged);
                        }
                        Err(e) => {
                            log::error!("Pull failed: {}", e);
                            this.fail_op(
                                operation_id,
                                GitOperationKind::Pull,
                                format!("Pull from '{}' failed", remote_name),
                                e.to_string(),
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

    /// Push to a remote.
    pub fn push(
        &mut self,
        remote_name: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!("push: remote={} force={}", remote_name, force);
        let (branch_name, set_upstream) = match self.open_repo().and_then(|repo| {
            push_target(&repo, Some(remote_name)).map(|(_, branch, set)| (branch, set))
        }) {
            Ok(target) => target,
            Err(error) => {
                return self.fail_to_start_task(
                    GitOperationKind::Push,
                    "Push could not start",
                    error,
                    true,
                    cx,
                )
            }
        };
        self.push_to(remote_name, &branch_name, force, set_upstream, cx)
    }

    pub fn push_to(
        &mut self,
        remote_name: &str,
        remote_branch_name: &str,
        force: bool,
        set_upstream: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!("push_to: remote={} branch={} force={}", remote_name, remote_branch_name, force);
        let remote_name = remote_name.to_string();
        let remote_branch_name = remote_branch_name.to_string();
        let task_remote_name = remote_name.clone();
        let task_remote_branch_name = remote_branch_name.clone();
        let repo_path = self.repo_path.clone();
        let operation_id = self.begin_operation(
            GitOperationKind::Push,
            if force {
                format!(
                    "Force pushing to '{}/{}'...",
                    remote_name, remote_branch_name
                )
            } else {
                format!("Pushing to '{}/{}'...", remote_name, remote_branch_name)
            },
            Some(remote_name.clone()),
            Some(remote_branch_name.clone()),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<(String, Option<String>, RefreshData)> = cx
                .background_executor()
                .spawn(async move {
                    let (msg, details) = {
                        let repo = Repository::open(&repo_path)?;
                        let branch_name = head_branch_name(&repo)?;
                        drop(repo);

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
                        let details = run_git_network_command(&repo_path, &args)?;
                        let msg = if force {
                            format!(
                                "Force pushed '{}' to '{}/{}'",
                                branch_name, task_remote_name, task_remote_branch_name
                            )
                        } else if set_upstream {
                            format!(
                                "Pushed '{}' to '{}/{}' and set upstream",
                                branch_name, task_remote_name, task_remote_branch_name
                            )
                        } else {
                            format!(
                                "Pushed '{}' to '{}/{}'",
                                branch_name, task_remote_name, task_remote_branch_name
                            )
                        };
                        (msg, details)
                    };
                    let data = gather_refresh_data(&repo_path)?;
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
                                GitOperationKind::Push,
                                msg,
                                (
                                    details.or(Some("Remote refs refreshed after push.".into())),
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
                                GitOperationKind::Push,
                                format!("Push to '{}' failed", remote_name),
                                e.to_string(),
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
