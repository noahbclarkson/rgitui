use anyhow::Result;
use git2::Repository;
use gpui::{AsyncApp, Context, Task, WeakEntity};

use crate::types::*;

use super::auth::{make_fetch_options, make_push_options, remote_uses_ssh, run_git_network_command};
use super::refresh::gather_refresh_data;
use super::{
    ensure_clean_worktree, head_branch_name, pull_target, push_target,
    GitProject, GitProjectEvent, RefreshData,
};

impl GitProject {
    pub fn fetch_default(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
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
                    let details = {
                        let repo = Repository::open(&repo_path)?;
                        if remote_uses_ssh(&repo, &task_remote_name)? {
                            drop(repo);
                            run_git_network_command(
                                &repo_path,
                                &["fetch", "--prune", &task_remote_name],
                            )?
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let mut fetch_opts = make_fetch_options();
                            remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
                            None
                        }
                    };
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
                                (details.or(Some("Remote refs refreshed.".into())), Some(remote_name.clone()), this.head_branch.clone()),
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

                        if remote_uses_ssh(&repo, &task_remote_name)? {
                            drop(repo);
                            let details = run_git_network_command(
                                &repo_path,
                                &["pull", &task_remote_name, &task_branch_name],
                            )?;
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
                                    "Pulled '{}' from '{}' using system Git",
                                    task_branch_name, task_remote_name
                                )
                            };
                            (msg, details)
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let mut fetch_opts = make_fetch_options();
                            remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
                            drop(remote);

                            let fetch_head = repo.find_reference("FETCH_HEAD")?;
                            let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
                            drop(fetch_head);

                            let (analysis, _pref) = repo.merge_analysis(&[&fetch_commit])?;

                            let msg = if analysis.is_up_to_date() {
                                format!(
                                    "'{}' is already up to date with '{}'",
                                    task_branch_name, task_remote_name
                                )
                            } else if analysis.is_fast_forward() {
                                let refname = format!("refs/heads/{}", task_branch_name);
                                let mut reference = repo.find_reference(&refname)?;
                                reference.set_target(fetch_commit.id(), "Fast-forward pull")?;
                                repo.set_head(&refname)?;
                                repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
                                format!(
                                    "Pulled '{}' from '{}' (fast-forward)",
                                    task_branch_name, task_remote_name
                                )
                            } else if analysis.is_normal() {
                                repo.merge(&[&fetch_commit], None, None)?;
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
                                        &format!(
                                            "Merge remote-tracking branch '{}/{}'",
                                            task_remote_name, task_branch_name
                                        ),
                                        &tree,
                                        &[&head_commit, &fetch_commit_obj],
                                    )?;
                                    repo.cleanup_state()?;
                                    format!(
                                        "Pulled '{}' from '{}' with a merge commit",
                                        task_branch_name, task_remote_name
                                    )
                                } else {
                                    let conflict_count = repo
                                        .index()?
                                        .conflicts()?
                                        .count();
                                    format!(
                                        "CONFLICT:{} conflict(s) during pull from '{}/{}'. Resolve and continue.",
                                        conflict_count, task_remote_name, task_branch_name
                                    )
                                }
                            } else {
                                format!("Pulled '{}' from '{}'", task_branch_name, task_remote_name)
                            };
                            (msg, None)
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

                        if remote_uses_ssh(&repo, &task_remote_name)? {
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
                                    "Force pushed '{}' to '{}/{}' using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else if set_upstream {
                                format!(
                                    "Pushed '{}' to '{}/{}' and set upstream using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            } else {
                                format!(
                                    "Pushed '{}' to '{}/{}' using system Git",
                                    branch_name, task_remote_name, task_remote_branch_name
                                )
                            };
                            (msg, details)
                        } else {
                            let mut remote = repo.find_remote(&task_remote_name)?;
                            let refspec = if force {
                                format!(
                                    "+refs/heads/{}:refs/heads/{}",
                                    branch_name, task_remote_branch_name
                                )
                            } else {
                                format!(
                                    "refs/heads/{}:refs/heads/{}",
                                    branch_name, task_remote_branch_name
                                )
                            };
                            let mut push_opts = make_push_options();
                            remote.push(&[&refspec], Some(&mut push_opts))?;
                            drop(remote);

                            if let Ok(local_branch) = repo.find_branch(&branch_name, git2::BranchType::Local) {
                                if let Some(tip) = local_branch.get().target() {
                                    let remote_ref_name = format!(
                                        "refs/remotes/{}/{}",
                                        task_remote_name, task_remote_branch_name
                                    );
                                    if let Err(e) = repo.reference(
                                        &remote_ref_name,
                                        tip,
                                        true,
                                        &format!("push: update {} after push", remote_ref_name),
                                    ) {
                                        log::warn!("Failed to update remote tracking ref '{}' after push: {}", remote_ref_name, e);
                                    }
                                }
                            }

                            if set_upstream {
                                let mut branch =
                                    repo.find_branch(&branch_name, git2::BranchType::Local)?;
                                branch.set_upstream(Some(&format!(
                                    "{}/{}",
                                    task_remote_name, task_remote_branch_name
                                )))?;
                            }

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
                            (msg, None)
                        }
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
                                (details.or(Some("Remote refs refreshed after push.".into())), Some(remote_name.clone()), Some(remote_branch_name.clone())),
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
                                (Some(remote_name.clone()), Some(remote_branch_name.clone()), true),
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
