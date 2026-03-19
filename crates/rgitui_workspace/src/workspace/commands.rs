use gpui::Context;

use crate::{
    CommandId, CommitPanelEvent, ConfirmAction, ToastKind,
};

use super::{BottomPanelMode, ProjectTab, Workspace};

impl Workspace {
    pub(super) fn execute_command(&mut self, cmd: CommandId, cx: &mut Context<Self>) {
        match cmd {
            CommandId::Settings => {
                self.overlays.settings_modal.update(cx, |sm, cx| {
                    sm.toggle_visible(cx);
                });
            }
            CommandId::CreateBranch => {
                self.dialogs.branch_dialog.update(cx, |bd, cx| {
                    bd.show_visible(None, cx);
                });
            }
            CommandId::OpenRepo => {
                self.overlays.repo_opener.update(cx, |ro, cx| {
                    ro.toggle_visible(cx);
                });
            }
            CommandId::Shortcuts => {
                self.overlays.shortcuts_help.update(cx, |sh, cx| {
                    sh.toggle_visible(cx);
                });
            }
            CommandId::WorkspaceHome => {
                self.go_home(cx);
            }
            CommandId::RestoreLastWorkspace => {
                self.restore_last_workspace(cx);
            }
            CommandId::Undo => {
                self.execute_undo(cx);
            }
            cmd => {
                let Some(tab) = self.tabs.get(self.active_tab).cloned() else {
                    return;
                };
                self.execute_tab_command(cmd, &tab, cx);
            }
        }
    }

    pub(super) fn execute_tab_command(
        &mut self,
        cmd: CommandId,
        tab: &ProjectTab,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            CommandId::Fetch => {
                tab.project.update(cx, |proj, cx| {
                    proj.fetch_default(cx).detach();
                });
            }
            CommandId::Pull => {
                tab.project.update(cx, |proj, cx| {
                    proj.pull_default(cx).detach();
                });
            }
            CommandId::Push => {
                tab.project.update(cx, |proj, cx| {
                    proj.push_default(false, cx).detach();
                });
            }
            CommandId::Commit => {
                tab.commit_panel.update(cx, |cp, cx| {
                    let msg = cp.message(cx);
                    if !msg.is_empty() {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: msg,
                            amend: false,
                        });
                    }
                });
            }
            CommandId::StageAll => {
                tab.project.update(cx, |proj, cx| {
                    proj.stage_all(cx).detach();
                });
            }
            CommandId::UnstageAll => {
                tab.project.update(cx, |proj, cx| {
                    proj.unstage_all(cx).detach();
                });
            }
            CommandId::StashSave => {
                tab.project.update(cx, |proj, cx| {
                    proj.stash_save(None, cx).detach();
                });
            }
            CommandId::StashPop => {
                tab.project.update(cx, |proj, cx| {
                    proj.stash_pop(0, cx).detach();
                });
            }
            CommandId::ToggleDiffMode => {
                tab.diff_viewer.update(cx, |dv, cx| {
                    dv.toggle_display_mode(cx);
                });
            }
            CommandId::AiMessage => {
                tab.commit_panel.update(cx, |_cp, cx| {
                    cx.emit(CommitPanelEvent::GenerateAiMessage);
                });
            }
            CommandId::MergeBranch => {
                let head = tab
                    .project
                    .read(cx)
                    .head_branch()
                    .unwrap_or("HEAD")
                    .to_string();
                let msg = format!("Use the sidebar to merge a branch into '{}'", head);
                self.show_toast(msg, ToastKind::Info, cx);
            }
            CommandId::Refresh => {
                tab.project.update(cx, |proj, cx| {
                    proj.refresh(cx).detach();
                });
            }
            CommandId::Search => {
                tab.graph.update(cx, |g, cx| {
                    g.toggle_search(cx);
                });
            }
            CommandId::InteractiveRebase => {
                use crate::interactive_rebase::{RebaseAction, RebaseEntry};
                let project = tab.project.read(cx);
                let head_branch = project.head_branch().unwrap_or("HEAD").to_string();
                let commits = project.recent_commits();
                let entries: Vec<RebaseEntry> = commits
                    .iter()
                    .take(20)
                    .map(|c| RebaseEntry {
                        oid: c.oid.to_string(),
                        original_message: c.summary.clone(),
                        author: c.author.name.clone(),
                        action: RebaseAction::Pick,
                    })
                    .collect();

                if entries.is_empty() {
                    self.status_message =
                        Some("No commits available for interactive rebase.".into());
                    self.show_toast(
                        "No commits available for interactive rebase.",
                        ToastKind::Info,
                        cx,
                    );
                } else {
                    self.overlays.interactive_rebase.update(cx, |ir, cx| {
                        ir.show_visible(entries, head_branch, cx);
                    });
                }
            }
            CommandId::StashDrop => {
                let has_stashes = !tab.project.read(cx).stashes().is_empty();
                if has_stashes {
                    tab.project.update(cx, |proj, cx| {
                        proj.stash_drop(0, cx).detach();
                    });
                } else {
                    self.show_toast("No stashes to drop", ToastKind::Warning, cx);
                }
            }
            CommandId::StashApply => {
                let has_stashes = !tab.project.read(cx).stashes().is_empty();
                if has_stashes {
                    tab.project.update(cx, |proj, cx| {
                        proj.stash_apply(0, cx).detach();
                    });
                } else {
                    self.show_toast("No stashes to apply", ToastKind::Warning, cx);
                }
            }
            CommandId::ForcePush => {
                self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Force Push",
                        "This will overwrite the remote branch. Are you sure?",
                        ConfirmAction::ForcePush,
                        cx,
                    );
                });
            }
            CommandId::DiscardAll => {
                self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Discard All Changes",
                        "This will permanently discard all uncommitted changes.",
                        ConfirmAction::DiscardAll,
                        cx,
                    );
                });
            }
            CommandId::AbortOperation => {
                let state = tab.project.read(cx).repo_state();
                if state.is_clean() {
                    self.show_toast("No operation in progress to abort", ToastKind::Warning, cx);
                } else {
                    let state_label = state.label().to_string();
                    self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                        cd.show_visible(
                            format!("Abort {}", state_label),
                            format!(
                                "This will abort the current {} and reset to HEAD. All conflict resolution progress will be lost.",
                                state_label.to_lowercase()
                            ),
                            ConfirmAction::AbortMerge,
                            cx,
                        );
                    });
                }
            }
            CommandId::ContinueMerge => {
                let state = tab.project.read(cx).repo_state();
                if state.is_clean() {
                    self.show_toast("No operation in progress", ToastKind::Warning, cx);
                } else if tab.project.read(cx).has_conflicts() {
                    self.show_toast(
                        "Cannot continue — resolve all conflicts first",
                        ToastKind::Error,
                        cx,
                    );
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.continue_merge(cx).detach();
                    });
                }
            }
            CommandId::CreateTag => {
                let proj = tab.project.read(cx);
                if let Some(head) = proj.recent_commits().first() {
                    let oid = head.oid;
                    self.dialogs.tag_dialog.update(cx, |td, cx| {
                        td.show_visible(oid, cx);
                    });
                } else {
                    self.show_toast("No HEAD commit to tag", ToastKind::Error, cx);
                }
            }
            CommandId::ResetHard => {
                self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Reset Hard",
                        "Hard reset to HEAD? All staged and unstaged changes will be permanently discarded.",
                        ConfirmAction::ResetHard("HEAD".to_string()),
                        cx,
                    );
                });
            }
            CommandId::RenameBranch => {
                let proj = tab.project.read(cx);
                if let Some(head) = proj.head_branch() {
                    let name = head.to_string();
                    self.dialogs.rename_dialog.update(cx, |rd, cx| {
                        rd.show_visible(name, cx);
                    });
                } else {
                    self.show_toast(
                        "No branch to rename (detached HEAD)",
                        ToastKind::Error,
                        cx,
                    );
                }
            }
            CommandId::CherryPick | CommandId::RevertCommit | CommandId::DeleteBranch => {
                let msg = format!(
                    "Use the sidebar context menu for '{}'",
                    cmd.display_label()
                );
                self.show_toast(msg, ToastKind::Info, cx);
            }
            CommandId::SwitchBranch => {
                self.show_toast(
                    "Press Ctrl+Shift+B or use Alt+1 to focus the sidebar for branch switching",
                    ToastKind::Info,
                    cx,
                );
            }
            CommandId::Blame => {
                self.toggle_blame_view(tab, cx);
            }
            CommandId::Settings
            | CommandId::CreateBranch
            | CommandId::OpenRepo
            | CommandId::Shortcuts
            | CommandId::WorkspaceHome
            | CommandId::RestoreLastWorkspace
            | CommandId::Undo => {}
        }
    }

    fn toggle_blame_view(
        &mut self,
        tab: &ProjectTab,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::Blame {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let file_path = tab.diff_viewer.read(cx).file_path().map(String::from);
        let Some(file_path) = file_path else {
            self.show_toast(
                "No file selected. Select a file first to view blame.",
                ToastKind::Info,
                cx,
            );
            return;
        };

        let project = tab.project.clone();
        let blame_view = tab.blame_view.clone();
        let path_for_blame = std::path::PathBuf::from(&file_path);
        let display_path = file_path.clone();
        let active_tab_index = self.active_tab;

        let task = project.update(cx, |proj, cx| {
            proj.blame_file_async(&path_for_blame, None, cx)
        });

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            match task.await {
                Ok(lines) => {
                    let _ = cx.update(|cx| {
                        blame_view.update(cx, |bv, cx| {
                            bv.set_blame(lines, display_path, cx);
                        });
                        let _ = this.update(cx, |workspace, cx| {
                            if let Some(active_tab) = workspace.tabs.get_mut(active_tab_index) {
                                active_tab.bottom_panel_mode = BottomPanelMode::Blame;
                            }
                            cx.notify();
                        });
                    });
                }
                Err(e) => {
                    let _ = cx.update(|cx| {
                        let _ = this.update(cx, |workspace, cx| {
                            workspace.show_toast(
                                format!("Failed to compute blame: {}", e),
                                ToastKind::Error,
                                cx,
                            );
                        });
                    });
                }
            }
        })
        .detach();
    }
}
