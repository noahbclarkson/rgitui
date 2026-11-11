use super::messages::{error, error_message};
use anyhow::{Context as _, Result};
use git2::Repository;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use rgitui_i18n::{msg, msgp, Message};
use std::path::Path;

use crate::types::*;

use super::argsafe::{validate_branch_name, validate_remote_name};
use super::auth::run_git_network_command;
use super::autostash::{pending_autostash, AutostashRestore};
use super::local_ops::format_path_list;
use super::refresh::gather_refresh_data;
use super::{
    head_branch_name, pull_target, push_target, repo_has_worktree_changes, GitProject,
    GitProjectEvent, RefreshData,
};

/// Open the repository handle for a specific working tree. Network operations
/// resolve their branch and upstream from *this* checkout, which is what makes
/// pushing from an inspected worktree push that worktree's branch rather than
/// the branch the main checkout happens to be on.
fn open_worktree_repo(worktree_path: &Path) -> Result<Repository> {
    Repository::open(worktree_path)
        .with_context(|| format!("Failed to open worktree at {}", worktree_path.display()))
}

/// How a pull ended, when git got far enough to leave something to report.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PullOutcome {
    /// There was nothing to integrate.
    UpToDate,
    /// The remote branch was integrated.
    Updated,
    /// Git stopped part-way with a merge or rebase to resolve.
    Conflicts {
        count: usize,
        /// Git stashed uncommitted changes first and is holding them until the
        /// operation is continued or aborted.
        autostash_pending: bool,
    },
}

#[derive(Debug)]
struct PullReport {
    outcome: PullOutcome,
    /// What git printed.
    output: Option<String>,
    /// The pull itself went through, but reapplying the changes git had
    /// stashed for it conflicted. Git has put them in the stash list.
    autostash_conflicted: bool,
}

/// A pull git refused or could not carry out.
#[derive(Debug)]
struct PullFailure {
    error: anyhow::Error,
    /// See [`PullAttempt::blocked_by_local_changes`].
    blocked_by_local_changes: bool,
}

impl From<anyhow::Error> for PullFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            blocked_by_local_changes: false,
        }
    }
}

impl From<git2::Error> for PullFailure {
    fn from(error: git2::Error) -> Self {
        anyhow::Error::from(error).into()
    }
}

/// What stood in the way when git refused to pull over uncommitted changes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalChangesBlock {
    /// An incoming commit touches these files, which have uncommitted changes.
    Files(Vec<String>),
    /// The pull rebases, and a rebase will not start with any tracked file
    /// changed, whichever files the incoming commits touch.
    Rebase,
}

/// Recognise git refusing a pull because of uncommitted changes to tracked
/// files, from its output (which [`super::git_command`] keeps in English).
///
/// Only that refusal counts. Git words an untracked file standing in the way
/// almost identically, but a stash does not take untracked files, so offering
/// autostash for it would promise a fix that does not work.
fn local_changes_block(output: &str) -> Option<LocalChangesBlock> {
    if output.contains("cannot pull with rebase") {
        return Some(LocalChangesBlock::Rebase);
    }

    let mut lines = output.lines().skip_while(|line| {
        !line.contains("Your local changes to the following files would be overwritten")
    });
    lines.next()?;
    // The files follow, indented, up to git's advice on what to do about them.
    let files = lines
        .take_while(|line| line.starts_with([' ', '\t']))
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    Some(LocalChangesBlock::Files(files))
}

impl LocalChangesBlock {
    fn message(&self) -> Message {
        match self {
            LocalChangesBlock::Files(files) if !files.is_empty() => msg!(
                "git.pull.blocked_files_detail",
                paths = format_path_list(files)
            ),
            LocalChangesBlock::Files(_) => msg!("git.pull.blocked_detail"),
            LocalChangesBlock::Rebase => msg!("git.pull.blocked_rebase_detail"),
        }
    }
}

/// Run `git pull` in `worktree_path` and work out what it left behind.
///
/// Whether uncommitted changes stop the pull is `local_changes`' call. Under
/// [`PullLocalChanges::GitDecides`] nothing is checked up front: git refuses on
/// its own, touching nothing, when an incoming commit would overwrite one, and
/// that refusal comes back as a [`PullFailure`] marked as blocked.
fn run_pull(
    worktree_path: &Path,
    remote_name: &str,
    branch_name: &str,
    local_changes: PullLocalChanges,
) -> Result<PullReport, PullFailure> {
    validate_remote_name(remote_name)?;
    validate_branch_name(branch_name)?;

    let state_before = {
        let repo = Repository::open(worktree_path)?;
        if local_changes == PullLocalChanges::RequireClean && repo_has_worktree_changes(&repo)? {
            return Err(PullFailure {
                error: error(msg!(
                    "git.error.clean_worktree_required",
                    operation = GitOperationKind::Pull.name()
                )),
                blocked_by_local_changes: true,
            });
        }
        repo.state()
    };

    let mut args = vec!["pull"];
    if local_changes == PullLocalChanges::Autostash {
        args.push("--autostash");
    }
    args.push(remote_name);
    args.push(branch_name);

    match run_git_network_command(worktree_path, &args) {
        Ok(output) => {
            let up_to_date = output
                .as_deref()
                .is_some_and(|text| text.contains("Already up to date."));
            // Git exits successfully with unmerged entries in exactly one case:
            // the pull went through and reapplying its autostash conflicted.
            let autostash_conflicted = Repository::open(worktree_path)?.index()?.has_conflicts();
            Ok(PullReport {
                outcome: if up_to_date {
                    PullOutcome::UpToDate
                } else {
                    PullOutcome::Updated
                },
                output,
                autostash_conflicted,
            })
        }
        Err(e) => {
            let text = e.to_string();
            let repo = Repository::open(worktree_path)?;
            // A pull that leaves a merge or rebase behind stopped part-way,
            // whatever it printed; one that was already in progress did not
            // come from this pull.
            let stopped = state_before == git2::RepositoryState::Clean
                && repo.state() != git2::RepositoryState::Clean;
            if stopped || text.contains("CONFLICT") || text.contains("Automatic merge failed") {
                Ok(PullReport {
                    outcome: PullOutcome::Conflicts {
                        count: repo.index()?.conflicts()?.count(),
                        autostash_pending: pending_autostash(&repo).is_some(),
                    },
                    output: Some(text),
                    autostash_conflicted: false,
                })
            } else if let Some(block) = local_changes_block(&text) {
                Err(PullFailure {
                    error: error(block.message()),
                    blocked_by_local_changes: true,
                })
            } else {
                Err(e.into())
            }
        }
    }
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

    /// Pull the branch `worktree_path` has checked out from its upstream.
    /// `local_changes` says what to do about uncommitted changes.
    pub fn pull_default_at(
        &mut self,
        local_changes: PullLocalChanges,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "pull_default: local_changes={:?} worktree={}",
            local_changes,
            worktree_path.display()
        );
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
        self.pull_from_at(&remote_name, &branch_name, local_changes, worktree_path, cx)
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
        local_changes: PullLocalChanges,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "pull: remote={} local_changes={:?} worktree={}",
            remote_name,
            local_changes,
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
        self.pull_from_at(remote_name, &branch_name, local_changes, worktree_path, cx)
    }

    pub fn pull_from_at(
        &mut self,
        remote_name: &str,
        branch_name: &str,
        local_changes: PullLocalChanges,
        worktree_path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        log::info!(
            "pull_from: remote={} branch={} local_changes={:?} worktree={}",
            remote_name,
            branch_name,
            local_changes,
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
            if local_changes == PullLocalChanges::Autostash {
                msg!(
                    "git.pull.autostash_running",
                    branch = &branch_name,
                    remote = &remote_name
                )
            } else {
                msg!(
                    "git.pull.running",
                    branch = &branch_name,
                    remote = &remote_name
                )
            },
            Some(remote_name.clone()),
            Some(branch_name.clone()),
            cx,
        );
        self.note_operation_worktree(operation_id, worktree_path);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: Result<(PullReport, RefreshData), PullFailure> = cx
                .background_executor()
                .spawn(async move {
                    let report = run_pull(
                        &task_worktree_path,
                        &task_remote_name,
                        &task_branch_name,
                        local_changes,
                    )?;
                    let data = gather_refresh_data(&repo_path, commit_limit)?;
                    Ok((report, data))
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let finish = |this: &Self,
                                  state: GitOperationState,
                                  summary: Message,
                                  details: Message,
                                  (retryable, blocked_by_local_changes): (bool, bool),
                                  cx: &mut Context<Self>| {
                        cx.emit(GitProjectEvent::OperationUpdated(GitOperationUpdate {
                            id: operation_id,
                            kind: GitOperationKind::Pull,
                            state,
                            summary,
                            details: Some(details),
                            remote_name: Some(remote_name.clone()),
                            branch_name: Some(branch_name.clone()),
                            worktree_path: this.operation_worktree(operation_id),
                            retryable,
                            pull: Some(PullAttempt {
                                local_changes,
                                blocked_by_local_changes,
                            }),
                        }));
                    };

                    match result {
                        Ok((report, data)) => {
                            this.apply_refresh_data(data);
                            let output = report.output.map(Message::verbatim);
                            match report.outcome {
                                PullOutcome::Conflicts {
                                    count,
                                    autostash_pending,
                                } => {
                                    let details = if autostash_pending {
                                        msgp!(
                                            "git.pull.conflicts_autostash_detail",
                                            count,
                                            remote = &remote_name,
                                            branch = &branch_name
                                        )
                                    } else {
                                        msgp!(
                                            "git.pull.conflicts_detail",
                                            count,
                                            remote = &remote_name,
                                            branch = &branch_name
                                        )
                                    };
                                    finish(
                                        this,
                                        GitOperationState::Failed,
                                        msg!("git.pull.conflict", remote = &remote_name),
                                        details,
                                        (false, false),
                                        cx,
                                    );
                                }
                                PullOutcome::UpToDate => finish(
                                    this,
                                    GitOperationState::Succeeded,
                                    msg!(
                                        "git.pull.up_to_date",
                                        branch = &branch_name,
                                        remote = &remote_name
                                    ),
                                    output.unwrap_or(msg!("git.pull.detail")),
                                    (false, false),
                                    cx,
                                ),
                                PullOutcome::Updated => finish(
                                    this,
                                    GitOperationState::Succeeded,
                                    msg!(
                                        "git.pull.done",
                                        branch = &branch_name,
                                        remote = &remote_name
                                    ),
                                    output.unwrap_or(msg!("git.pull.detail")),
                                    (false, false),
                                    cx,
                                ),
                            }
                            if report.autostash_conflicted {
                                this.report_autostash_restore(
                                    AutostashRestore::Conflicted,
                                    Some(branch_name.clone()),
                                    cx,
                                );
                            }
                            cx.emit(GitProjectEvent::RepositoryChanged);
                        }
                        Err(failure) => {
                            log::error!("Pull failed: {}", failure.error);
                            finish(
                                this,
                                GitOperationState::Failed,
                                if failure.blocked_by_local_changes {
                                    msg!("git.pull.blocked", remote = &remote_name)
                                } else {
                                    msg!("git.pull.failed", remote = &remote_name)
                                },
                                error_message(&failure.error),
                                (true, failure.blocked_by_local_changes),
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

#[cfg(test)]
mod tests {
    use super::super::local_ops::{abort_in_progress_operation, continue_in_progress_operation};
    use super::*;
    use rgitui_test_support::TempRepo;

    const BASE: &str = "one\ntwo\nthree\nfour\nfive\n";

    fn git(repo: &TempRepo, args: &[&str]) -> String {
        let output = super::super::git_command()
            .current_dir(repo.path())
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn read(repo: &TempRepo, file: &str) -> String {
        std::fs::read_to_string(repo.path().join(file)).expect("file should exist")
    }

    fn state(repo: &TempRepo) -> git2::RepositoryState {
        Repository::open(repo.path()).unwrap().state()
    }

    /// A repository to pull from, and a checkout of it that is one pull behind
    /// nothing yet: both hold `shared.txt` and `mine.txt` at the same commit.
    ///
    /// The checkout pins every setting a pull consults, so the machine's own
    /// git config — a global `pull.rebase` or `merge.autoStash` — cannot change
    /// what these tests exercise.
    fn remote_and_checkout(rebase: bool) -> (TempRepo, TempRepo) {
        let remote = TempRepo::init();
        remote.commit_file("shared.txt", BASE, "add shared");
        remote.commit_file("mine.txt", "mine\n", "add mine");

        let checkout = TempRepo::init();
        let url = remote.path().to_string_lossy().to_string();
        git(&checkout, &["remote", "add", "origin", &url]);
        for (key, value) in [
            ("pull.rebase", if rebase { "true" } else { "false" }),
            ("pull.ff", "true"),
            ("merge.autoStash", "false"),
            ("rebase.autoStash", "false"),
            ("commit.gpgSign", "false"),
        ] {
            git(&checkout, &["config", key, value]);
        }
        git(&checkout, &["pull", "origin", "main"]);
        (remote, checkout)
    }

    fn pull(
        checkout: &TempRepo,
        local_changes: PullLocalChanges,
    ) -> Result<PullReport, PullFailure> {
        run_pull(checkout.path(), "origin", "main", local_changes)
    }

    /// The remote and the checkout each commit a different first line of
    /// `shared.txt`, so pulling stops on a conflict; `mine.txt` is edited and
    /// left uncommitted.
    fn diverged_with_uncommitted_change(rebase: bool) -> (TempRepo, TempRepo) {
        let (remote, checkout) = remote_and_checkout(rebase);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.commit_file("shared.txt", &BASE.replace("one", "ours"), "our change");
        checkout.write_file("mine.txt", "mine\nuncommitted\n");
        (remote, checkout)
    }

    #[test]
    fn nothing_to_pull_is_reported_as_up_to_date() {
        let (_remote, checkout) = remote_and_checkout(false);
        let report = pull(&checkout, PullLocalChanges::GitDecides).unwrap();
        assert_eq!(report.outcome, PullOutcome::UpToDate);
        assert!(!report.autostash_conflicted);
    }

    /// The case from the issue: an uncommitted edit to one file must not stop
    /// a pull whose commits only touch another.
    #[test]
    fn git_pulls_past_uncommitted_changes_the_incoming_commits_do_not_touch() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("mine.txt", "mine\nuncommitted\n");

        let report = pull(&checkout, PullLocalChanges::GitDecides).unwrap();

        assert_eq!(report.outcome, PullOutcome::Updated);
        assert_eq!(read(&checkout, "shared.txt"), BASE.replace("one", "theirs"));
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
    }

    #[test]
    fn git_refusing_to_overwrite_a_change_is_reported_as_blocked_and_names_the_file() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("shared.txt", &BASE.replace("five", "uncommitted"));
        let head = checkout.head_oid();

        let failure = pull(&checkout, PullLocalChanges::GitDecides).unwrap_err();

        assert!(failure.blocked_by_local_changes);
        let message = error_message(&failure.error).english();
        assert!(message.contains("shared.txt"), "got {message:?}");
        assert!(message.contains("AutoStash"), "got {message:?}");
        // Git refused before touching anything.
        assert_eq!(checkout.head_oid(), head);
        assert_eq!(
            read(&checkout, "shared.txt"),
            BASE.replace("five", "uncommitted")
        );
    }

    #[test]
    fn requiring_a_clean_tree_refuses_before_git_runs() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("mine.txt", "mine\nuncommitted\n");
        let head = checkout.head_oid();

        let failure = pull(&checkout, PullLocalChanges::RequireClean).unwrap_err();

        assert!(failure.blocked_by_local_changes);
        assert_eq!(
            checkout.head_oid(),
            head,
            "a pull git itself would have accepted must still be refused"
        );
    }

    #[test]
    fn requiring_a_clean_tree_ignores_untracked_files() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("scratch.txt", "untracked\n");

        let report = pull(&checkout, PullLocalChanges::RequireClean).unwrap();
        assert_eq!(report.outcome, PullOutcome::Updated);
    }

    #[test]
    fn autostash_pulls_over_a_changed_file_and_puts_the_change_back() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("shared.txt", &BASE.replace("five", "uncommitted"));

        let report = pull(&checkout, PullLocalChanges::Autostash).unwrap();

        assert_eq!(report.outcome, PullOutcome::Updated);
        assert!(!report.autostash_conflicted);
        assert_eq!(
            read(&checkout, "shared.txt"),
            BASE.replace("one", "theirs").replace("five", "uncommitted")
        );
        assert_eq!(git(&checkout, &["stash", "list"]), "");
    }

    #[test]
    fn an_autostash_that_conflicts_on_the_way_back_is_kept_in_the_stash_list() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("shared.txt", &BASE.replace("one", "uncommitted"));

        let report = pull(&checkout, PullLocalChanges::Autostash).unwrap();

        // The pull itself went through; it is the reapplied change that
        // conflicts, and the caller reports that separately.
        assert_eq!(report.outcome, PullOutcome::Updated);
        assert!(report.autostash_conflicted);
        assert_eq!(checkout.head_oid(), remote.head_oid());
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        assert!(
            git(&checkout, &["stash", "list"]).contains("autostash"),
            "the uncommitted change must be recoverable from the stash list"
        );
    }

    #[test]
    fn a_pull_stopped_by_a_merge_conflict_holds_the_autostash_until_it_is_aborted() {
        let (_remote, checkout) = diverged_with_uncommitted_change(false);
        let head = checkout.head_oid();

        let report = pull(&checkout, PullLocalChanges::Autostash).unwrap();
        assert_eq!(
            report.outcome,
            PullOutcome::Conflicts {
                count: 1,
                autostash_pending: true
            }
        );
        // Git took the uncommitted change out of the tree for the merge.
        assert_eq!(read(&checkout, "mine.txt"), "mine\n");
        assert_eq!(git(&checkout, &["stash", "list"]), "");

        let restore = abort_in_progress_operation(checkout.path()).unwrap();

        assert_eq!(restore, Some(AutostashRestore::Applied));
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        assert_eq!(checkout.head_oid(), head);
        assert_eq!(read(&checkout, "shared.txt"), BASE.replace("one", "ours"));
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
        assert!(pending_autostash(checkout.repo()).is_none());
    }

    #[test]
    fn concluding_a_conflicted_merge_pull_restores_the_autostash() {
        let (remote, checkout) = diverged_with_uncommitted_change(false);
        let ours = checkout.head_oid();
        pull(&checkout, PullLocalChanges::Autostash).unwrap();

        checkout.write_file("shared.txt", &BASE.replace("one", "resolved"));
        git(&checkout, &["add", "shared.txt"]);
        let (_summary, restore) = continue_in_progress_operation(checkout.path()).unwrap();

        assert_eq!(restore, Some(AutostashRestore::Applied));
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        let merge = checkout.repo().find_commit(checkout.head_oid()).unwrap();
        assert_eq!(
            merge.parent_ids().collect::<Vec<_>>(),
            vec![ours, remote.head_oid()]
        );
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
        assert!(pending_autostash(checkout.repo()).is_none());
        assert_eq!(git(&checkout, &["stash", "list"]), "");
    }

    #[test]
    fn aborting_a_conflicted_rebase_pull_restores_the_autostash() {
        let (_remote, checkout) = diverged_with_uncommitted_change(true);
        let head = checkout.head_oid();

        let report = pull(&checkout, PullLocalChanges::Autostash).unwrap();
        assert_eq!(
            report.outcome,
            PullOutcome::Conflicts {
                count: 1,
                autostash_pending: true
            }
        );

        let restore = abort_in_progress_operation(checkout.path()).unwrap();

        assert_eq!(restore, Some(AutostashRestore::Applied));
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        // Back on the branch, at the commit it had — not left detached on the
        // commit the rebase had reached.
        assert_eq!(git(&checkout, &["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(checkout.head_oid(), head);
        assert_eq!(read(&checkout, "shared.txt"), BASE.replace("one", "ours"));
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
        assert_eq!(git(&checkout, &["stash", "list"]), "");
    }

    #[test]
    fn aborting_a_rebase_without_an_autostash_returns_to_the_branch() {
        let (_remote, checkout) = diverged_with_uncommitted_change(true);
        git(&checkout, &["checkout", "--", "mine.txt"]);
        let head = checkout.head_oid();
        let report = pull(&checkout, PullLocalChanges::GitDecides).unwrap();
        assert_eq!(
            report.outcome,
            PullOutcome::Conflicts {
                count: 1,
                autostash_pending: false
            }
        );

        let restore = abort_in_progress_operation(checkout.path()).unwrap();

        assert_eq!(restore, None);
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        assert_eq!(git(&checkout, &["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(checkout.head_oid(), head);
    }

    #[test]
    fn continuing_a_conflicted_rebase_pull_leaves_the_autostash_to_git() {
        let (_remote, checkout) = diverged_with_uncommitted_change(true);
        pull(&checkout, PullLocalChanges::Autostash).unwrap();

        checkout.write_file("shared.txt", &BASE.replace("one", "resolved"));
        git(&checkout, &["add", "shared.txt"]);
        let (_summary, restore) = continue_in_progress_operation(checkout.path()).unwrap();

        // Git reapplied it cleanly, so there is nothing further to report.
        assert_eq!(restore, None);
        assert_eq!(state(&checkout), git2::RepositoryState::Clean);
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
        assert_eq!(git(&checkout, &["stash", "list"]), "");
    }

    #[test]
    fn a_rebase_pull_refused_over_uncommitted_changes_is_reported_as_blocked() {
        let (remote, checkout) = remote_and_checkout(true);
        remote.commit_file("shared.txt", &BASE.replace("one", "theirs"), "their change");
        checkout.write_file("mine.txt", "mine\nuncommitted\n");

        let failure = pull(&checkout, PullLocalChanges::GitDecides).unwrap_err();
        assert!(failure.blocked_by_local_changes);

        // The same pull with autostash is what gets past it.
        let report = pull(&checkout, PullLocalChanges::Autostash).unwrap();
        assert_eq!(report.outcome, PullOutcome::Updated);
        assert_eq!(read(&checkout, "mine.txt"), "mine\nuncommitted\n");
    }

    /// A stash does not take untracked files, so autostash cannot help here and
    /// must not be offered.
    #[test]
    fn an_untracked_file_in_the_way_is_not_reported_as_blocked_by_local_changes() {
        let (remote, checkout) = remote_and_checkout(false);
        remote.commit_file("new.txt", "theirs\n", "add new");
        checkout.write_file("new.txt", "untracked\n");

        let failure = pull(&checkout, PullLocalChanges::GitDecides).unwrap_err();

        assert!(!failure.blocked_by_local_changes);
        assert_eq!(read(&checkout, "new.txt"), "untracked\n");
    }

    #[test]
    fn a_pull_without_conflicts_is_not_mistaken_for_one_when_a_merge_is_already_in_progress() {
        let (_remote, checkout) = diverged_with_uncommitted_change(false);
        git(&checkout, &["checkout", "--", "mine.txt"]);
        pull(&checkout, PullLocalChanges::GitDecides).unwrap();
        assert_eq!(state(&checkout), git2::RepositoryState::Merge);

        // Git refuses a second pull outright; that is a failure, not a new
        // conflict to resolve.
        let failure = pull(&checkout, PullLocalChanges::GitDecides).unwrap_err();
        assert!(!failure.blocked_by_local_changes);
    }

    #[test]
    fn the_blocking_files_are_read_from_gits_refusal() {
        let output = "From /tmp/remote\n * branch            main       -> FETCH_HEAD\n\
                      error: Your local changes to the following files would be overwritten by merge:\n\
                      \tsrc/a.rs\n\tdocs/b.md\n\
                      Please commit your changes or stash them before you merge.\nAborting\n\
                      Updating b7b4571..8f9bbdd";
        assert_eq!(
            local_changes_block(output),
            Some(LocalChangesBlock::Files(vec![
                "src/a.rs".to_string(),
                "docs/b.md".to_string()
            ]))
        );
    }

    #[test]
    fn a_refusal_from_the_merge_strategy_is_recognised_too() {
        // A true merge with a staged change words the same refusal differently.
        let output =
            "error: Your local changes to the following files would be overwritten by merge:\n  \
                      shared.txt\nMerge with strategy ort failed.";
        assert_eq!(
            local_changes_block(output),
            Some(LocalChangesBlock::Files(vec!["shared.txt".to_string()]))
        );
    }

    #[test]
    fn a_rebase_refusal_names_no_files() {
        let output = "error: cannot pull with rebase: You have unstaged changes.\n\
                      error: Please commit or stash them.";
        assert_eq!(local_changes_block(output), Some(LocalChangesBlock::Rebase));
    }

    #[test]
    fn other_failures_are_not_blamed_on_local_changes() {
        for output in [
            "error: The following untracked working tree files would be overwritten by merge:\n\
             \tnew.txt\nPlease move or remove them before you merge.\nAborting",
            "fatal: unable to access 'https://example.com/repo.git/': Could not resolve host",
            "fatal: Need to specify how to reconcile divergent branches.",
        ] {
            assert_eq!(local_changes_block(output), None, "for {output:?}");
        }
    }
}
