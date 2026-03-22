use std::time::Instant;

use gpui::{Context, Entity, SharedString};
use rgitui_ai::{AiEvent, AiGenerator};
use rgitui_diff::{DiffViewer, DiffViewerEvent};
use rgitui_git::{
    GitOperationKind, GitOperationState, GitProject, GitProjectEvent, RebaseEntryAction,
    RebasePlanEntry,
};
use rgitui_graph::{GraphView, GraphViewEvent};

use crate::{
    BlameView, BlameViewEvent, BranchDialog, BranchDialogEvent, CommandPalette,
    CommandPaletteEvent, CommitPanel, CommitPanelEvent, ConfirmAction, ConfirmDialog,
    ConfirmDialogEvent, DetailPanel, DetailPanelEvent, FileHistoryView, FileHistoryViewEvent,
    InteractiveRebase, InteractiveRebaseEvent, ReflogView, ReflogViewEvent, RenameDialog,
    RenameDialogEvent, RepoOpener, RepoOpenerEvent, SettingsModal, SettingsModalEvent,
    ShortcutsHelp, ShortcutsHelpEvent, Sidebar, SidebarEvent, SubmoduleView, SubmoduleViewEvent,
    TagDialog, TagDialogEvent, ToastKind, Toolbar, ToolbarEvent,
};

use super::{ActiveOperation, BottomPanelMode, OperationOutput, UndoAction, UndoEntry, Workspace};

pub(super) fn subscribe_settings_modal(
    cx: &mut Context<Workspace>,
    settings_modal: &Entity<SettingsModal>,
) {
    cx.subscribe(
        settings_modal,
        |_this, _sm, event: &SettingsModalEvent, cx| match event {
            SettingsModalEvent::Dismissed => {
                cx.notify();
            }
            SettingsModalEvent::ThemeChanged(_name) => {
                cx.notify();
            }
            SettingsModalEvent::SettingsChanged => {
                cx.notify();
            }
        },
    )
    .detach();
}

pub(super) fn subscribe_interactive_rebase(
    cx: &mut Context<Workspace>,
    interactive_rebase: &Entity<InteractiveRebase>,
) {
    cx.subscribe(
        interactive_rebase,
        |this, _ir, event: &InteractiveRebaseEvent, cx| match event {
            InteractiveRebaseEvent::Execute(entries) => {
                use crate::interactive_rebase::RebaseAction;

                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let plan: Vec<RebasePlanEntry> = entries
                        .iter()
                        .map(|e| RebasePlanEntry {
                            oid: e.oid.clone(),
                            message: e.original_message.clone(),
                            action: match &e.action {
                                RebaseAction::Pick => RebaseEntryAction::Pick,
                                RebaseAction::Reword(msg) => {
                                    let m = if msg.is_empty() {
                                        e.original_message.clone()
                                    } else {
                                        msg.clone()
                                    };
                                    RebaseEntryAction::Reword(m)
                                }
                                RebaseAction::Squash => RebaseEntryAction::Squash,
                                RebaseAction::Fixup => RebaseEntryAction::Fixup,
                                RebaseAction::Drop => RebaseEntryAction::Drop,
                            },
                        })
                        .collect();

                    let project = tab.project.clone();
                    project.update(cx, |proj, cx| {
                        proj.rebase_interactive(plan, cx).detach();
                    });

                    let count = entries.len();
                    let msg = format!("Interactive rebase started with {} commits.", count);
                    this.status_message = Some(msg.clone());
                    this.show_toast(msg, ToastKind::Info, cx);
                }
            }
            InteractiveRebaseEvent::Cancel => {
                cx.notify();
            }
        },
    )
    .detach();
}

pub(super) fn subscribe_ai(cx: &mut Context<Workspace>, ai: &Entity<AiGenerator>) {
    cx.subscribe(ai, |this, _ai, event: &AiEvent, cx| match event {
        AiEvent::GenerationCompleted(message) => {
            if let Some(tab) = this.tabs.get(this.active_tab) {
                let msg = message.clone();
                tab.commit_panel.update(cx, |cp, cx| {
                    cp.set_message(msg, cx);
                    cp.set_ai_generating(false, cx);
                });
            }
        }
        AiEvent::GenerationFailed(err) => {
            log::error!("AI generation failed: {}", err);
            let msg = format!("AI error: {}", err);
            this.status_message = Some(msg.clone());
            this.show_toast(msg, ToastKind::Error, cx);
            if let Some(tab) = this.tabs.get(this.active_tab) {
                tab.commit_panel.update(cx, |cp, cx| {
                    cp.set_ai_generating(false, cx);
                });
            }
        }
        AiEvent::GenerationStarted => {
            this.status_message = Some("Generating AI commit message...".into());
            this.show_toast("Generating AI commit message...", ToastKind::Info, cx);
        }
    })
    .detach();
}

pub(super) fn subscribe_command_palette(
    cx: &mut Context<Workspace>,
    command_palette: &Entity<CommandPalette>,
) {
    cx.subscribe(
        command_palette,
        |this, _cp, event: &CommandPaletteEvent, cx| match event {
            CommandPaletteEvent::CommandSelected(cmd_id) => {
                this.execute_command(*cmd_id, cx);
            }
            CommandPaletteEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_branch_dialog(
    cx: &mut Context<Workspace>,
    branch_dialog: &Entity<BranchDialog>,
) {
    cx.subscribe(
        branch_dialog,
        |this, _bd, event: &BranchDialogEvent, cx| match event {
            BranchDialogEvent::CreateBranch { name, base_ref } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let base = if base_ref.is_empty() {
                        None
                    } else {
                        Some(base_ref.as_str())
                    };
                    project.update(cx, |proj, cx| {
                        proj.create_branch_at(&name, base, cx).detach();
                    });
                }
                this.show_toast(format!("Branch '{}' created", name), ToastKind::Success, cx);
            }
            BranchDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_tag_dialog(cx: &mut Context<Workspace>, tag_dialog: &Entity<TagDialog>) {
    cx.subscribe(
        tag_dialog,
        |this, _td, event: &TagDialogEvent, cx| match event {
            TagDialogEvent::CreateTag { name, target_oid } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let oid = *target_oid;
                    project.update(cx, |proj, cx| {
                        proj.create_tag(&name, oid, cx).detach();
                    });
                }
            }
            TagDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_rename_dialog(
    cx: &mut Context<Workspace>,
    rename_dialog: &Entity<RenameDialog>,
) {
    cx.subscribe(
        rename_dialog,
        |this, _rd, event: &RenameDialogEvent, cx| match event {
            RenameDialogEvent::Rename { old_name, new_name } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let old = old_name.clone();
                    let new = new_name.clone();
                    project.update(cx, |proj, cx| {
                        proj.rename_branch(&old, &new, cx).detach();
                    });
                }
                this.show_toast(
                    format!("Branch renamed: {} → {}", old_name, new_name),
                    ToastKind::Success,
                    cx,
                );
            }
            RenameDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_confirm_dialog(
    cx: &mut Context<Workspace>,
    confirm_dialog: &Entity<ConfirmDialog>,
) {
    cx.subscribe(
        confirm_dialog,
        |this, _cd, event: &ConfirmDialogEvent, cx| match event {
            ConfirmDialogEvent::Confirmed(action) => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    match action {
                        ConfirmAction::DiscardFile(path) => {
                            let path_buf = std::path::PathBuf::from(path);
                            project.update(cx, |proj, cx| {
                                proj.discard_changes(&[path_buf], cx).detach();
                            });
                        }
                        ConfirmAction::ForcePush => {
                            project.update(cx, |proj, cx| {
                                proj.push_default(true, cx).detach();
                            });
                        }
                        ConfirmAction::BranchDelete(name) => {
                            let tip_oid = project
                                .read(cx)
                                .branches()
                                .iter()
                                .find(|b| b.name == *name)
                                .and_then(|b| b.tip_oid)
                                .map(|oid| oid.to_string());
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.delete_branch(&name, cx).detach();
                            });
                            if let Some(oid_hex) = tip_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!("Deleted branch '{}'", name),
                                        action: UndoAction::RecreateBranch { name, oid_hex },
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Branch deleted. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::StashDrop(index) => {
                            let index = *index;
                            project.update(cx, |proj, cx| {
                                proj.stash_drop(index, cx).detach();
                            });
                        }
                        ConfirmAction::DiscardAll => {
                            let has_changes = project.read(cx).has_changes();
                            if has_changes {
                                project.update(cx, |proj, cx| {
                                    proj.stash_save(Some("rgitui-undo-discard"), cx).detach();
                                });
                                this.push_undo(
                                    UndoEntry {
                                        label: "Discarded all changes".into(),
                                        action: UndoAction::PopStash(0),
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Changes discarded. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::TagDelete(name) => {
                            let tag_oid = project
                                .read(cx)
                                .tags()
                                .iter()
                                .find(|t| t.name == *name)
                                .map(|t| t.oid.to_string());
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.delete_tag(&name, cx).detach();
                            });
                            if let Some(oid_hex) = tag_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!("Deleted tag '{}'", name),
                                        action: UndoAction::RecreateTag { name, oid_hex },
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Tag deleted. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::ResetHard(target) => {
                            let previous_head_oid = project
                                .read(cx)
                                .recent_commits()
                                .first()
                                .map(|c| c.oid.to_string());
                            let target = target.clone();
                            project.update(cx, |proj, cx| {
                                if let Ok(oid) = git2::Oid::from_str(&target) {
                                    proj.reset_to_commit(oid, cx).detach();
                                } else {
                                    proj.reset_hard(cx).detach();
                                }
                            });
                            if let Some(oid_hex) = previous_head_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!(
                                            "Reset to {}",
                                            &target[..7.min(target.len())]
                                        ),
                                        action: UndoAction::ResetTo(oid_hex),
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Reset complete. Use command palette 'Undo' to revert.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::RemoveRemote(name) => {
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.remove_remote(&name, cx).detach();
                            });
                        }
                        ConfirmAction::AbortMerge => {
                            project.update(cx, |proj, cx| {
                                proj.abort_operation(cx).detach();
                            });
                        }
                    }
                }
            }
            ConfirmDialogEvent::Cancelled => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_repo_opener(cx: &mut Context<Workspace>, repo_opener: &Entity<RepoOpener>) {
    cx.subscribe(
        repo_opener,
        |this, _ro, event: &RepoOpenerEvent, cx| match event {
            RepoOpenerEvent::OpenRepo(path) => {
                if let Err(e) = this.open_repo(path.clone(), cx) {
                    this.show_toast(format!("Failed to open: {}", e), ToastKind::Error, cx);
                }
            }
            RepoOpenerEvent::Dismissed => {
                this.focus.pending_focus_restore = true;
                cx.notify();
            }
        },
    )
    .detach();
}

pub(super) fn subscribe_shortcuts_help(
    cx: &mut Context<Workspace>,
    shortcuts_help: &Entity<ShortcutsHelp>,
) {
    cx.subscribe(
        shortcuts_help,
        |_this, _sh, _event: &ShortcutsHelpEvent, _cx| {},
    )
    .detach();
}

// ---- Per-tab subscriptions (called from open_repo) ----

pub(super) fn subscribe_project(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    graph: &Entity<GraphView>,
    sidebar: &Entity<Sidebar>,
    commit_panel: &Entity<CommitPanel>,
    toolbar: &Entity<Toolbar>,
) {
    let graph = graph.clone();
    let sidebar = sidebar.clone();
    let commit_panel = commit_panel.clone();
    let toolbar = toolbar.clone();

    cx.subscribe(project, {
        move |this, project, event: &GitProjectEvent, cx| match event {
            GitProjectEvent::StatusChanged
            | GitProjectEvent::HeadChanged
            | GitProjectEvent::RefsChanged => {
                let proj = project.read(cx);
                let commits = proj.recent_commits_arc();
                let has_more = proj.has_more_commits();
                let wt_status = proj.status_arc();
                let branches = proj.branches().to_vec();
                let tags = proj.tags().to_vec();
                let remotes = proj.remotes().to_vec();
                let stashes = proj.stashes().to_vec();
                let has_stashes = !stashes.is_empty();
                let has_changes = proj.has_changes();
                let staged_count = wt_status.staged.len();
                let (ahead, behind) = branches
                    .iter()
                    .find(|b| b.is_head)
                    .map(|b| (b.ahead, b.behind))
                    .unwrap_or((0, 0));
                let mut seen = std::collections::HashSet::new();
                let authors: Vec<(String, String)> = commits
                    .iter()
                    .filter(|c| seen.insert(c.author.email.clone()))
                    .map(|c| (c.author.name.clone(), c.author.email.clone()))
                    .collect();
                crate::avatar_resolver::resolve_avatars(authors, cx);

                let wt_staged = wt_status.staged.len();
                let wt_unstaged = wt_status.unstaged.len();
                let wt_staged_bd = rgitui_graph::compute_breakdown(&wt_status.staged);
                let wt_unstaged_bd = rgitui_graph::compute_breakdown(&wt_status.unstaged);
                graph.update(cx, |g, cx| {
                    g.set_commits(commits, cx);
                    g.set_all_loaded(!has_more);
                    g.set_working_tree_status(
                        wt_staged,
                        wt_unstaged,
                        wt_staged_bd,
                        wt_unstaged_bd,
                        cx,
                    );
                });

                sidebar.update(cx, |s, cx| {
                    s.update_branches(branches, cx);
                    s.update_tags(tags, cx);
                    s.update_remotes(remotes, cx);
                    s.update_stashes(stashes, cx);
                    s.update_status(wt_status.staged.clone(), wt_status.unstaged.clone(), cx);
                });

                commit_panel.update(cx, |cp, cx| cp.set_staged_count(staged_count, cx));

                toolbar.update(cx, |tb, cx| {
                    tb.set_state(true, true, has_stashes, has_changes, cx);
                    tb.set_ahead_behind(ahead, behind, cx);
                });
            }
            GitProjectEvent::OperationUpdated(update) => {
                let is_running = update.state == GitOperationState::Running;
                let operation_id = update.id;
                let failure_message = if let Some(details) = &update.details {
                    format!("{}: {}", update.summary, details)
                } else {
                    update.summary.clone()
                };

                match update.state {
                    GitOperationState::Running => {
                        this.operations.is_loading = true;
                        this.operations.loading_message = Some(update.summary.clone());
                        this.status_message = Some(update.summary.clone());
                        this.operations.active_git_operation = Some(update.clone());
                        this.operations.active_operations.push(ActiveOperation {
                            id: operation_id,
                            label: update.summary.clone().into(),
                            started_at: Instant::now(),
                        });
                        this.show_toast(update.summary.clone(), ToastKind::Info, cx);
                    }
                    GitOperationState::Succeeded => {
                        this.operations
                            .active_operations
                            .retain(|op| op.id != operation_id);
                        this.operations.is_loading = !this.operations.active_operations.is_empty();
                        this.operations.loading_message = this
                            .operations
                            .active_operations
                            .last()
                            .map(|op| op.label.to_string());
                        this.status_message = Some(update.summary.clone());
                        if this
                            .operations
                            .active_git_operation
                            .as_ref()
                            .is_some_and(|op| op.id == operation_id)
                        {
                            this.operations.active_git_operation = None;
                        }
                        if this
                            .operations
                            .last_failed_git_operation
                            .as_ref()
                            .is_some_and(|op| op.kind == update.kind)
                        {
                            this.operations.last_failed_git_operation = None;
                        }
                        let output_text = update.details.clone().unwrap_or_default();
                        if !output_text.is_empty() {
                            let now = Instant::now();
                            this.operations.last_operation_output = Some(OperationOutput {
                                operation: SharedString::from(
                                    update.kind.display_name().to_string(),
                                ),
                                output: output_text,
                                is_error: false,
                                timestamp: now,
                                expanded: false,
                            });
                            this.schedule_operation_output_auto_hide(now, cx);
                        }
                        this.show_toast(update.summary.clone(), ToastKind::Success, cx);
                    }
                    GitOperationState::Failed => {
                        this.operations
                            .active_operations
                            .retain(|op| op.id != operation_id);
                        this.operations.is_loading = !this.operations.active_operations.is_empty();
                        this.operations.loading_message = this
                            .operations
                            .active_operations
                            .last()
                            .map(|op| op.label.to_string());
                        if this
                            .operations
                            .active_git_operation
                            .as_ref()
                            .is_some_and(|op| op.id == operation_id)
                        {
                            this.operations.active_git_operation = None;
                        }
                        this.operations.last_failed_git_operation = Some(update.clone());
                        this.status_message = Some(failure_message.clone());
                        let error_output = update
                            .details
                            .clone()
                            .unwrap_or_else(|| failure_message.clone());
                        this.operations.last_operation_output = Some(OperationOutput {
                            operation: SharedString::from(update.kind.display_name().to_string()),
                            output: error_output,
                            is_error: true,
                            timestamp: Instant::now(),
                            expanded: true,
                        });
                        this.show_toast(failure_message, ToastKind::Error, cx);
                    }
                }

                toolbar.update(cx, |tb, cx| {
                    tb.set_fetching(is_running && update.kind == GitOperationKind::Fetch, cx);
                    tb.set_pulling(is_running && update.kind == GitOperationKind::Pull, cx);
                    tb.set_pushing(is_running && update.kind == GitOperationKind::Push, cx);
                });
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_sidebar(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    sidebar: &Entity<Sidebar>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
) {
    let project = project.clone();
    let diff_viewer = diff_viewer.clone();
    let detail_panel_ref = detail_panel.clone();

    cx.subscribe(sidebar, {
        move |this, _sidebar, event: &SidebarEvent, cx| match event {
            SidebarEvent::FileSelected { path, staged } => {
                let path_buf = std::path::PathBuf::from(path);
                let p = path.clone();
                let is_staged = *staged;
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let dv = diff_viewer.clone();
                let dp = detail_panel_ref.clone();
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_file_diff(&repo_path, &path_buf, is_staged)
                        })
                        .await;
                    cx.update(|cx| match result {
                        Ok(diff) => {
                            dv.update(cx, |dv, cx| dv.set_diff(diff, p, is_staged, cx));
                            dp.update(cx, |dp, cx| dp.clear(cx));
                        }
                        Err(e) => log::error!("Failed to get diff: {}", e),
                    });
                })
                .detach();
            }
            SidebarEvent::StageFile(path) => {
                let path_buf = std::path::PathBuf::from(path);
                project.update(cx, |proj, cx| {
                    proj.stage_files(&[path_buf], cx).detach();
                });
            }
            SidebarEvent::UnstageFile(path) => {
                let path_buf = std::path::PathBuf::from(path);
                project.update(cx, |proj, cx| {
                    proj.unstage_files(&[path_buf], cx).detach();
                });
            }
            SidebarEvent::StageAll => {
                project.update(cx, |proj, cx| {
                    proj.stage_all(cx).detach();
                });
            }
            SidebarEvent::UnstageAll => {
                project.update(cx, |proj, cx| {
                    proj.unstage_all(cx).detach();
                });
            }
            SidebarEvent::BranchCheckout(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.checkout_branch(&name, cx).detach();
                });
            }
            SidebarEvent::RemoteFetch(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.fetch(&name, cx).detach();
                });
            }
            SidebarEvent::RemotePull(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.pull(&name, cx).detach();
                });
            }
            SidebarEvent::RemotePush(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.push(&name, false, cx).detach();
                });
            }
            SidebarEvent::RemoteRemove(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Remove Remote",
                        format!(
                            "Remove remote '{}' and its configured URLs from this repository?",
                            name
                        ),
                        ConfirmAction::RemoveRemote(name),
                        cx,
                    );
                });
            }
            SidebarEvent::DiscardFile(path) => {
                let path = path.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Discard Changes",
                        format!("Are you sure you want to discard changes to {}?", path),
                        ConfirmAction::DiscardFile(path),
                        cx,
                    );
                });
            }
            SidebarEvent::StashSelected(index) => {
                let idx = *index;
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let dv = diff_viewer.clone();
                let dp = detail_panel_ref.clone();
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { rgitui_git::compute_stash_diff(&repo_path, idx) })
                        .await;
                    cx.update(|cx| match result {
                        Ok(commit_diff) => {
                            if let Some(first_file) = commit_diff.files.first() {
                                let path = first_file.path.display().to_string();
                                dv.update(cx, |dv, cx| {
                                    dv.set_diff(first_file.clone(), path, false, cx)
                                });
                            }
                            dp.update(cx, |dp, cx| dp.clear(cx));
                        }
                        Err(e) => log::error!("Failed to get stash diff: {}", e),
                    });
                })
                .detach();
            }
            SidebarEvent::TagSelected(name) => {
                let proj = project.read(cx);
                if let Ok(oid) = proj.resolve_tag_to_oid(name) {
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        tab.graph.update(cx, |g, cx| {
                            g.scroll_to_commit(oid, cx);
                        });
                    }
                } else {
                    log::warn!("Could not resolve tag '{}' to a commit", name);
                }
            }
            SidebarEvent::BranchSelected(name) => {
                let proj = project.read(cx);
                if let Ok(oid) = proj.resolve_branch_to_oid(name) {
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        tab.graph.update(cx, |g, cx| {
                            g.scroll_to_commit(oid, cx);
                        });
                    }
                } else {
                    log::warn!("Could not resolve branch '{}' to a commit", name);
                }
            }
            SidebarEvent::MergeBranch(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.merge_branch(&name, cx).detach();
                });
            }
            SidebarEvent::BranchCreate => {
                this.dialogs.branch_dialog.update(cx, |bd, cx| {
                    bd.show_visible(None, cx);
                });
            }
            SidebarEvent::BranchDelete(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Delete Branch",
                        format!("Are you sure you want to delete branch '{}'?", name),
                        ConfirmAction::BranchDelete(name),
                        cx,
                    );
                });
            }
            SidebarEvent::OpenRepo => {
                this.overlays.repo_opener.update(cx, |ro, cx| {
                    ro.toggle_visible(cx);
                });
            }
            SidebarEvent::TagDelete(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Delete Tag",
                        format!(
                            "Are you sure you want to delete tag '{}'? This cannot be undone.",
                            name
                        ),
                        ConfirmAction::TagDelete(name),
                        cx,
                    );
                });
            }
            SidebarEvent::StashApply(index) => {
                let index = *index;
                project.update(cx, |proj, cx| {
                    proj.stash_apply(index, cx).detach();
                });
            }
            SidebarEvent::BranchRename(name) => {
                let name = name.clone();
                this.dialogs.rename_dialog.update(cx, |rd, cx| {
                    rd.show_visible(name, cx);
                });
            }
            SidebarEvent::StashDrop(index) => {
                let index = *index;
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Drop Stash",
                        format!(
                            "Are you sure you want to drop stash@{{{}}}? This cannot be undone.",
                            index
                        ),
                        ConfirmAction::StashDrop(index),
                        cx,
                    );
                });
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_graph(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    graph: &Entity<GraphView>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
) {
    let project = project.clone();
    let diff_viewer = diff_viewer.clone();
    let detail_panel_ref = detail_panel.clone();

    cx.subscribe(graph, {
        move |this, _graph, event: &GraphViewEvent, cx| {
            match event {
                GraphViewEvent::CommitSelected(oid) => {
                    let commit_oid = *oid;
                    let proj = project.read(cx);
                    let commit_info = proj
                        .recent_commits()
                        .iter()
                        .find(|c| c.oid == commit_oid)
                        .cloned();
                    let repo_path = proj.repo_path().to_path_buf();
                    let dv = diff_viewer.clone();
                    let dp = detail_panel_ref.clone();
                    cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                rgitui_git::compute_commit_diff(&repo_path, commit_oid)
                            })
                            .await;
                        cx.update(|cx| match result {
                            Ok(commit_diff) => {
                                if let Some(info) = commit_info {
                                    dp.update(cx, |dp, cx| {
                                        dp.set_commit(info, commit_diff.clone(), cx)
                                    });
                                }
                                if let Some(first_file) = commit_diff.files.first() {
                                    let path = first_file.path.display().to_string();
                                    dv.update(cx, |dv, cx| {
                                        dv.set_diff(first_file.clone(), path, false, cx)
                                    });
                                }
                            }
                            Err(e) => log::error!("Failed to get commit diff: {}", e),
                        });
                    })
                    .detach();
                }
                GraphViewEvent::CherryPick(oid) => {
                    let oid = *oid;
                    project.update(cx, |proj, cx| {
                        proj.cherry_pick(oid, cx).detach();
                    });
                }
                GraphViewEvent::RevertCommit(oid) => {
                    let oid = *oid;
                    project.update(cx, |proj, cx| {
                        proj.revert_commit(oid, cx).detach();
                    });
                }
                GraphViewEvent::CreateBranchAtCommit(oid) => {
                    let sha = oid.to_string();
                    this.dialogs.branch_dialog.update(cx, |bd, cx| {
                        bd.show_visible(Some(sha), cx);
                    });
                }
                GraphViewEvent::CheckoutCommit(oid) => {
                    let oid = *oid;
                    project.update(cx, |proj, cx| {
                        proj.checkout_commit(oid, cx).detach();
                    });
                }
                GraphViewEvent::CopyCommitSha(sha) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sha.clone()));
                    let short = &sha[..7.min(sha.len())];
                    this.show_toast(
                        format!("Copied SHA: {}", short),
                        ToastKind::Success,
                        cx,
                    );
                }
                GraphViewEvent::CreateTagAtCommit(oid) => {
                    let oid = *oid;
                    this.dialogs.tag_dialog.update(cx, |td, cx| {
                        td.show_visible(oid, cx);
                    });
                }
                GraphViewEvent::ResetToCommit(oid, sha) => {
                    let oid = *oid;
                    let sha = sha.clone();
                    let short = &sha[..7.min(sha.len())];
                    this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                        cd.show_visible(
                            "Reset to Commit",
                            format!(
                                "Hard reset the current branch to {}? All uncommitted changes and commits after this point will be lost.",
                                short
                            ),
                            ConfirmAction::ResetHard(oid.to_string()),
                            cx,
                        );
                    });
                }
                GraphViewEvent::BisectGood(oid) => {
                    let oid = *oid;
                    let state = project.read(cx).repo_state();
                    if !matches!(state, rgitui_git::RepoState::Bisect) {
                        this.show_toast("No bisect in progress. Use 'Bisect Start' first.", ToastKind::Warning, cx);
                    } else {
                        project.update(cx, |proj, cx| {
                            proj.bisect_good(Some(oid), cx).detach();
                        });
                    }
                }
                GraphViewEvent::BisectBad(oid) => {
                    let oid = *oid;
                    let state = project.read(cx).repo_state();
                    if !matches!(state, rgitui_git::RepoState::Bisect) {
                        this.show_toast("No bisect in progress. Use 'Bisect Start' first.", ToastKind::Warning, cx);
                    } else {
                        project.update(cx, |proj, cx| {
                            proj.bisect_bad(Some(oid), cx).detach();
                        });
                    }
                }
                GraphViewEvent::LoadMoreCommits => {
                    this.show_toast(
                        "Showing maximum 1,000 commits. Use search (Ctrl+F) to find older commits."
                            .to_string(),
                        ToastKind::Info,
                        cx,
                    );
                }
                GraphViewEvent::WorkingTreeSelected => {
                    let dp = detail_panel_ref.clone();
                    let dv = diff_viewer.clone();
                    dp.update(cx, |dp, cx| dp.clear(cx));
                    dv.update(cx, |dv, cx| dv.clear(cx));
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_detail_panel(
    cx: &mut Context<Workspace>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
) {
    let diff_viewer = diff_viewer.clone();

    cx.subscribe(detail_panel, {
        move |this, _dp, event: &DetailPanelEvent, cx| match event {
            DetailPanelEvent::FileSelected(file_diff, path) => {
                let p = path.clone();
                let fd = file_diff.clone();
                diff_viewer.update(cx, |dv, cx| {
                    dv.set_diff(fd, p, false, cx);
                });
            }
            DetailPanelEvent::CopySha(sha) => {
                let short = &sha[..7.min(sha.len())];
                this.show_toast(format!("Copied SHA: {}", short), ToastKind::Success, cx);
            }
            DetailPanelEvent::CherryPick(sha) => {
                if let Some(project) = this.active_project().cloned() {
                    if let Ok(oid) = git2::Oid::from_str(sha) {
                        project.update(cx, |proj, cx| {
                            proj.cherry_pick(oid, cx).detach();
                        });
                    }
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_diff_viewer(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    diff_viewer: &Entity<DiffViewer>,
) {
    let project = project.clone();
    let diff_viewer_ref = diff_viewer.clone();

    cx.subscribe(diff_viewer, {
        move |_this, _dv, event: &DiffViewerEvent, cx| {
            let file_path = diff_viewer_ref
                .read(cx)
                .file_path()
                .map(std::path::PathBuf::from);

            if let Some(path) = file_path {
                match event {
                    DiffViewerEvent::HunkStageRequested(hunk_idx) => {
                        let idx = *hunk_idx;
                        project.update(cx, |proj, cx| {
                            proj.stage_hunk(&path, idx, cx).detach();
                        });
                    }
                    DiffViewerEvent::HunkUnstageRequested(hunk_idx) => {
                        let idx = *hunk_idx;
                        project.update(cx, |proj, cx| {
                            proj.unstage_hunk(&path, idx, cx).detach();
                        });
                    }
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_commit_panel(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    ai: &Entity<AiGenerator>,
    commit_panel: &Entity<CommitPanel>,
) {
    let project = project.clone();
    let ai = ai.clone();
    let commit_panel_ref = commit_panel.clone();

    cx.subscribe(commit_panel, {
        move |this, _cp, event: &CommitPanelEvent, cx| match event {
            CommitPanelEvent::CommitRequested { message, amend } => {
                let previous_head_oid = project
                    .read(cx)
                    .recent_commits()
                    .first()
                    .map(|c| c.oid.to_string());
                let msg = message.clone();
                let is_amend = *amend;
                project.update(cx, |proj, cx| {
                    proj.commit(&msg, is_amend, cx).detach();
                });
                commit_panel_ref.update(cx, |cp, cx| {
                    cp.set_message(String::new(), cx);
                });
                if !is_amend {
                    if let Some(oid_hex) = previous_head_oid {
                        this.push_undo(
                            UndoEntry {
                                label: "Created commit".into(),
                                action: UndoAction::SoftResetHead(oid_hex),
                                created_at: Instant::now(),
                            },
                            cx,
                        );
                    }
                }
            }
            CommitPanelEvent::GenerateAiMessage => {
                commit_panel_ref.update(cx, |cp, cx| {
                    cp.set_ai_generating(true, cx);
                });

                let proj = project.read(cx);
                let repo_path = proj.repo_path().to_path_buf();
                let summary = proj.staged_summary();
                let ai_entity = ai.clone();
                let diff_repo_path = repo_path.clone();
                let settings_state = cx.global::<rgitui_settings::SettingsState>();
                let use_tools = settings_state.settings().ai.use_tools;
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let diff_text = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_staged_diff_text(&diff_repo_path)
                                .unwrap_or_default()
                        })
                        .await;
                    cx.update(|cx| {
                        ai_entity.update(cx, |ai_gen, cx| {
                            ai_gen
                                .generate_commit_message_with_tools(
                                    diff_text, summary, repo_path, use_tools, cx,
                                )
                                .detach();
                        });
                    });
                })
                .detach();
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_toolbar(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    toolbar: &Entity<Toolbar>,
) {
    let project = project.clone();

    cx.subscribe(toolbar, {
        move |this, _toolbar, event: &ToolbarEvent, cx| match event {
            ToolbarEvent::Fetch => {
                project.update(cx, |proj, cx| {
                    proj.fetch_default(cx).detach();
                });
            }
            ToolbarEvent::Pull => {
                project.update(cx, |proj, cx| {
                    proj.pull_default(cx).detach();
                });
            }
            ToolbarEvent::Push => {
                project.update(cx, |proj, cx| {
                    proj.push_default(false, cx).detach();
                });
            }
            ToolbarEvent::StashSave => {
                project.update(cx, |proj, cx| {
                    proj.stash_save(None, cx).detach();
                });
            }
            ToolbarEvent::StashPop => {
                project.update(cx, |proj, cx| {
                    proj.stash_pop(0, cx).detach();
                });
            }
            ToolbarEvent::Branch => {
                this.dialogs.branch_dialog.update(cx, |bd, cx| {
                    bd.show_visible(None, cx);
                });
            }
            ToolbarEvent::Refresh => {
                project.update(cx, |proj, cx| {
                    proj.refresh(cx).detach();
                });
            }
            ToolbarEvent::Settings => {
                this.overlays.settings_modal.update(cx, |sm, cx| {
                    sm.toggle_visible(cx);
                });
            }
            ToolbarEvent::Search => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    tab.graph.update(cx, |g, cx| {
                        g.toggle_search(cx);
                    });
                }
            }
            ToolbarEvent::OpenFileExplorer => {
                let repo_path = project.read(cx).repo_path().to_path_buf();
                super::layout::open_file_explorer(&repo_path);
            }
            ToolbarEvent::OpenTerminal => {
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let terminal_cmd = cx
                    .global::<rgitui_settings::SettingsState>()
                    .settings()
                    .terminal_command
                    .clone();
                super::layout::open_terminal(&repo_path, &terminal_cmd);
            }
            ToolbarEvent::OpenEditor => {
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let editor_cmd = cx
                    .global::<rgitui_settings::SettingsState>()
                    .settings()
                    .editor_command
                    .clone();
                super::layout::open_editor(&repo_path, &editor_cmd);
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_blame_view(
    cx: &mut Context<Workspace>,
    blame_view: &Entity<BlameView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();

    cx.subscribe(blame_view, {
        move |this, _bv, event: &BlameViewEvent, cx| match event {
            BlameViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            BlameViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_file_history_view(
    cx: &mut Context<Workspace>,
    file_history_view: &Entity<FileHistoryView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();

    cx.subscribe(file_history_view, {
        move |this, _fv, event: &FileHistoryViewEvent, cx| match event {
            FileHistoryViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            FileHistoryViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_reflog_view(
    cx: &mut Context<Workspace>,
    reflog_view: &Entity<ReflogView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();

    cx.subscribe(reflog_view, {
        move |this, _rv, event: &ReflogViewEvent, cx| match event {
            ReflogViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            ReflogViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_submodule_view(
    cx: &mut Context<Workspace>,
    submodule_view: &Entity<SubmoduleView>,
    project: &Entity<GitProject>,
) {
    let project = project.clone();
    let submodule_view = submodule_view.clone();

    cx.subscribe(&submodule_view, {
        move |this, _sv, event: &SubmoduleViewEvent, cx| match event {
            SubmoduleViewEvent::InitSubmodule(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.submodule_init_async(name.clone(), cx).detach();
                });
            }
            SubmoduleViewEvent::UpdateSubmodule(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.submodule_update_async(name.clone(), true, cx).detach();
                });
            }
            SubmoduleViewEvent::InitAll => {
                project.update(cx, |proj, cx| {
                    proj.submodule_init_all_async(cx).detach();
                });
            }
            SubmoduleViewEvent::UpdateAll => {
                project.update(cx, |proj, cx| {
                    proj.submodule_update_all_async(true, cx).detach();
                });
            }
            SubmoduleViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
        }
    })
    .detach();
}
