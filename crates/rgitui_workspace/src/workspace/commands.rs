use gpui::Context;

use crate::{CommandId, CommitPanelEvent, ConfirmAction, ToastKind};

use super::{
    BottomPanelMode, ProjectTab, RightPanelMode, ViewCacheEntry, ViewCacheKey, ViewCaches,
    Workspace,
};

impl Workspace {
    pub(super) fn execute_command(&mut self, cmd: CommandId, cx: &mut Context<Self>) {
        match cmd {
            CommandId::Settings => {
                self.open_or_focus_settings(cx);
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
            CommandId::PushAll => {
                let count = self.tabs.len();
                if count == 0 {
                    return;
                }
                self.show_toast(
                    format!("Pushing to all {} repositories...", count),
                    ToastKind::Info,
                    cx,
                );
                for tab in &self.tabs {
                    tab.project.update(cx, |proj, cx| {
                        proj.push_default(false, cx).detach();
                    });
                }
            }
            CommandId::PullAll => {
                let count = self.tabs.len();
                if count == 0 {
                    return;
                }
                self.show_toast(
                    format!("Pulling in all {} repositories...", count),
                    ToastKind::Info,
                    cx,
                );
                for tab in &self.tabs {
                    tab.project.update(cx, |proj, cx| {
                        proj.pull_default(cx).detach();
                    });
                }
            }
            CommandId::OpenThemeEditor => {
                self.overlays.theme_editor.update(cx, |te, cx| {
                    te.show_for_active_theme(cx);
                });
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
            // PushAll and PullAll are handled in execute_command (iterates all tabs).
            // Adding no-op arms here to satisfy exhaustiveness checker since
            // the cmd=> catchall can theoretically pass them to execute_tab_command.
            CommandId::PushAll | CommandId::PullAll => {}
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
                self.dialogs.stash_save_dialog.update(cx, |d, cx| {
                    d.show_visible(cx);
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
            CommandId::CleanUntracked => {
                self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Clean Untracked Files",
                        "This will permanently remove all untracked files and directories. This action cannot be undone.",
                        ConfirmAction::CleanUntracked,
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
                        "Cannot continue -- resolve all conflicts first",
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
            CommandId::CreateWorktree => {
                let proj = tab.project.read(cx);
                let branch = proj.head_branch().map(String::from);
                self.dialogs.worktree_dialog.update(cx, |wd, cx| {
                    wd.show_visible(branch, cx);
                });
            }
            CommandId::CreatePr => {
                self.open_create_pr_dialog(cx);
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
                    self.show_toast("No branch to rename (detached HEAD)", ToastKind::Error, cx);
                }
            }
            CommandId::CherryPick | CommandId::RevertCommit | CommandId::DeleteBranch => {
                let msg = format!("Use the sidebar context menu for '{}'", cmd.display_label());
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
            CommandId::FileHistory => {
                self.toggle_file_history_view(tab, cx);
            }
            CommandId::Reflog => {
                self.toggle_reflog_view(tab, cx);
            }
            CommandId::Bisect => {
                self.toggle_bisect_view(tab, cx);
            }
            CommandId::Submodules => {
                self.toggle_submodule_view(tab, cx);
            }
            CommandId::GlobalSearch => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    if tab.bottom_panel_mode == BottomPanelMode::GlobalSearch {
                        tab.global_search_view
                            .update(cx, |search, cx| search.hide(cx));
                        tab.bottom_panel_mode = BottomPanelMode::Diff;
                    } else {
                        tab.bottom_panel_mode = BottomPanelMode::GlobalSearch;
                        tab.global_search_view
                            .update(cx, |search, cx| search.show_without_focus(cx));
                    }
                    cx.notify();
                }
            }
            CommandId::BisectStart => {
                let state = tab.project.read(cx).repo_state();
                if matches!(state, rgitui_git::RepoState::Bisect) {
                    self.show_toast("Bisect already in progress", ToastKind::Warning, cx);
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.bisect_start(cx).detach();
                    });
                }
            }
            CommandId::BisectGood => {
                let state = tab.project.read(cx).repo_state();
                if !matches!(state, rgitui_git::RepoState::Bisect) {
                    self.show_toast(
                        "No bisect in progress. Use 'Bisect Start' first.",
                        ToastKind::Warning,
                        cx,
                    );
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.bisect_good(None, cx).detach();
                    });
                }
            }
            CommandId::BisectBad => {
                let state = tab.project.read(cx).repo_state();
                if !matches!(state, rgitui_git::RepoState::Bisect) {
                    self.show_toast(
                        "No bisect in progress. Use 'Bisect Start' first.",
                        ToastKind::Warning,
                        cx,
                    );
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.bisect_bad(None, cx).detach();
                    });
                }
            }
            CommandId::BisectReset => {
                let state = tab.project.read(cx).repo_state();
                if !matches!(state, rgitui_git::RepoState::Bisect) {
                    self.show_toast("No bisect in progress to reset", ToastKind::Warning, cx);
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.bisect_reset(cx).detach();
                    });
                }
            }
            CommandId::BisectSkip => {
                let state = tab.project.read(cx).repo_state();
                if !matches!(state, rgitui_git::RepoState::Bisect) {
                    self.show_toast(
                        "No bisect in progress. Use 'Bisect Start' first.",
                        ToastKind::Warning,
                        cx,
                    );
                } else {
                    tab.project.update(cx, |proj, cx| {
                        proj.bisect_skip(None, cx).detach();
                    });
                }
            }
            CommandId::ToggleIssues => {
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    if active_tab.right_panel_mode == RightPanelMode::Issues {
                        active_tab.right_panel_mode = RightPanelMode::Details;
                    } else {
                        active_tab.right_panel_mode = RightPanelMode::Issues;
                        let ip = active_tab.issues_panel.clone();
                        ip.update(cx, |panel, cx| {
                            if !panel.has_issues_loaded() && !panel.is_loading() {
                                panel.fetch_issues(cx);
                            }
                        });
                    }
                    cx.notify();
                }
            }
            CommandId::TogglePullRequests => {
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    if active_tab.right_panel_mode == RightPanelMode::PullRequests {
                        active_tab.right_panel_mode = RightPanelMode::Details;
                    } else {
                        active_tab.right_panel_mode = RightPanelMode::PullRequests;
                        let pp = active_tab.prs_panel.clone();
                        pp.update(cx, |panel, cx| {
                            if !panel.has_prs_loaded() && !panel.is_loading() {
                                panel.fetch_prs(cx);
                            }
                        });
                    }
                    cx.notify();
                }
            }
            CommandId::ToggleBranchHealth => {
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    if active_tab.right_panel_mode == RightPanelMode::BranchHealth {
                        active_tab.right_panel_mode = RightPanelMode::Details;
                    } else {
                        active_tab.right_panel_mode = RightPanelMode::BranchHealth;
                        let bh = active_tab.branch_health_panel.clone();
                        bh.update(cx, |panel, cx| {
                            panel.refresh(cx);
                        });
                    }
                    cx.notify();
                }
            }
            CommandId::ToggleStashes => {
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    if active_tab.right_panel_mode == RightPanelMode::Stashes {
                        active_tab.right_panel_mode = RightPanelMode::Details;
                    } else {
                        active_tab.right_panel_mode = RightPanelMode::Stashes;
                        let sp = active_tab.stashes_panel.clone();
                        sp.update(cx, |panel, cx| {
                            panel.refresh(cx);
                        });
                    }
                    cx.notify();
                }
            }
            CommandId::StashBranch => {
                self.show_toast(
                    "Right-click a stash in the sidebar to create a branch",
                    ToastKind::Info,
                    cx,
                );
            }
            CommandId::Settings
            | CommandId::CreateBranch
            | CommandId::OpenRepo
            | CommandId::Shortcuts
            | CommandId::WorkspaceHome
            | CommandId::RestoreLastWorkspace
            | CommandId::Undo
            | CommandId::OpenThemeEditor => {}
        }
    }

    fn toggle_blame_view(&mut self, tab: &ProjectTab, cx: &mut Context<Self>) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::Blame {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let Some(cache_key) = tab.current_view_cache_key(cx) else {
            self.show_toast(
                "No file selected. Select a file first to view blame.",
                ToastKind::Info,
                cx,
            );
            return;
        };

        // Check cache first — instant switch if available.
        if let Ok(mut cache) = tab.caches.blame.lock() {
            if let Some(ViewCacheEntry::Ready(lines)) = cache.get(&cache_key) {
                let display_path = cache_key.file_path.clone();
                tab.blame_view.update(cx, |bv, cx| {
                    bv.set_blame(lines, display_path, cx);
                });
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    active_tab.bottom_panel_mode = BottomPanelMode::Blame;
                }
                cx.notify();
                return;
            }
        }

        let entry = tab
            .caches
            .blame
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&cache_key));
        match entry {
            Some(ViewCacheEntry::Loading) => self.show_toast(
                "Blame is still being prepared for this file.",
                ToastKind::Info,
                cx,
            ),
            Some(ViewCacheEntry::Unavailable(reason)) => {
                log::debug!("Blame unavailable for {}: {}", cache_key.file_path, reason);
                self.show_toast(
                    "Blame is unavailable because this file has no version in the selected commit.",
                    ToastKind::Info,
                    cx,
                );
            }
            Some(ViewCacheEntry::Ready(_)) => {}
            None => {
                Self::prefetch_blame_and_history(cache_key, tab.caches.clone(), cx);
                self.show_toast("Preparing blame for this file...", ToastKind::Info, cx);
            }
        }
    }

    fn toggle_file_history_view(&mut self, tab: &ProjectTab, cx: &mut Context<Self>) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::FileHistory {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let Some(cache_key) = tab.current_view_cache_key(cx) else {
            self.show_toast(
                "No file selected. Select a file first to view history.",
                ToastKind::Info,
                cx,
            );
            return;
        };

        // Check cache first.
        if let Ok(mut cache) = tab.caches.history.lock() {
            if let Some(ViewCacheEntry::Ready(commits)) = cache.get(&cache_key) {
                let display_path = cache_key.file_path.clone();
                tab.file_history_view.update(cx, |fv, cx| {
                    fv.set_history(commits, display_path, cx);
                });
                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    active_tab.bottom_panel_mode = BottomPanelMode::FileHistory;
                }
                cx.notify();
                return;
            }
        }

        let entry = tab
            .caches
            .history
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&cache_key));
        match entry {
            Some(ViewCacheEntry::Loading) => {
                self.show_toast("File history is still being prepared.", ToastKind::Info, cx)
            }
            Some(ViewCacheEntry::Unavailable(reason)) => {
                log::debug!(
                    "History unavailable for {}: {}",
                    cache_key.file_path,
                    reason
                );
                self.show_toast(
                    "No committed history is available for this file.",
                    ToastKind::Info,
                    cx,
                );
            }
            Some(ViewCacheEntry::Ready(_)) => {}
            None => {
                Self::prefetch_blame_and_history(cache_key, tab.caches.clone(), cx);
                self.show_toast("Preparing file history...", ToastKind::Info, cx);
            }
        }
    }

    fn toggle_reflog_view(&mut self, tab: &ProjectTab, cx: &mut Context<Self>) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::Reflog {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let project = tab.project.clone();
        let reflog_view = tab.reflog_view.clone();
        let active_tab_index = self.active_tab;

        let task = project.update(cx, |proj, cx| proj.reflog_async("HEAD".to_string(), cx));

        cx.spawn(
            async move |this, cx: &mut gpui::AsyncApp| match task.await {
                Ok(entries) => {
                    cx.update(|cx| {
                        reflog_view.update(cx, |rv, cx| {
                            rv.set_entries(entries, cx);
                        });
                        let _ = this.update(cx, |workspace, cx| {
                            if let Some(active_tab) = workspace.tabs.get_mut(active_tab_index) {
                                active_tab.bottom_panel_mode = BottomPanelMode::Reflog;
                            }
                            cx.notify();
                        });
                    });
                }
                Err(e) => {
                    cx.update(|cx| {
                        let _ = this.update(cx, |workspace, cx| {
                            workspace.show_toast(
                                format!("Failed to compute reflog: {}", e),
                                ToastKind::Error,
                                cx,
                            );
                        });
                    });
                }
            },
        )
        .detach();
    }

    fn toggle_submodule_view(&mut self, tab: &ProjectTab, cx: &mut Context<Self>) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::Submodules {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let project = tab.project.clone();
        let submodule_view = tab.submodule_view.clone();
        let active_tab_index = self.active_tab;

        let task = project.update(cx, |proj, cx| proj.submodules_async(cx));

        cx.spawn(
            async move |this, cx: &mut gpui::AsyncApp| match task.await {
                Ok(submodules) => {
                    cx.update(|cx| {
                        submodule_view.update(cx, |sv, cx| {
                            sv.set_submodules(submodules, cx);
                        });
                        let _ = this.update(cx, |workspace, cx| {
                            if let Some(active_tab) = workspace.tabs.get_mut(active_tab_index) {
                                active_tab.bottom_panel_mode = BottomPanelMode::Submodules;
                            }
                            cx.notify();
                        });
                    });
                }
                Err(e) => {
                    cx.update(|cx| {
                        let _ = this.update(cx, |workspace, cx| {
                            workspace.show_toast(
                                format!("Failed to compute submodules: {}", e),
                                ToastKind::Error,
                                cx,
                            );
                        });
                    });
                }
            },
        )
        .detach();
    }

    fn toggle_bisect_view(&mut self, tab: &ProjectTab, cx: &mut Context<Self>) {
        if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
            if active_tab.bottom_panel_mode == BottomPanelMode::Bisect {
                active_tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();
                return;
            }
        }

        let project = tab.project.clone();
        let bisect_view = tab.bisect_view.clone();
        let active_tab_index = self.active_tab;

        let task = project.update(cx, |proj, cx| proj.bisect_log_async(cx));

        cx.spawn(
            async move |this, cx: &mut gpui::AsyncApp| match task.await {
                Ok(entries) => {
                    cx.update(|cx| {
                        bisect_view.update(cx, |bv, cx| {
                            bv.set_entries(entries, cx);
                        });
                        let _ = this.update(cx, |workspace, cx| {
                            if let Some(active_tab) = workspace.tabs.get_mut(active_tab_index) {
                                active_tab.bottom_panel_mode = BottomPanelMode::Bisect;
                            }
                            cx.notify();
                        });
                    });
                }
                Err(e) => {
                    cx.update(|cx| {
                        let _ = this.update(cx, |workspace, cx| {
                            workspace.show_toast(
                                format!("Failed to load bisect log: {}", e),
                                ToastKind::Error,
                                cx,
                            );
                        });
                    });
                }
            },
        )
        .detach();
    }

    /// Prefetch blame and file history for a file in the background.
    /// Called when a diff is opened so switching is near-instant.
    pub(super) fn prefetch_blame_and_history(
        cache_key: ViewCacheKey,
        caches: ViewCaches,
        cx: &mut Context<Self>,
    ) {
        let blame_cache = caches.blame.clone();
        let history_cache = caches.history.clone();
        let blame_path = std::path::PathBuf::from(&cache_key.file_path);
        let history_path = blame_path.clone();
        let repo1 = cache_key.repo_path.clone();
        let repo2 = cache_key.repo_path.clone();
        let commit_oid = cache_key
            .commit_id
            .as_deref()
            .and_then(|oid| git2::Oid::from_str(oid).ok());
        let invalid_commit = cache_key.commit_id.is_some() && commit_oid.is_none();

        // Reserve each missing entry before spawning. A click or a second
        // DiffChanged event now observes Loading instead of starting duplicate
        // Git commands.
        let run_blame = blame_cache
            .lock()
            .map(|mut cache| {
                if cache.contains(&cache_key) {
                    false
                } else {
                    cache.insert(cache_key.clone(), ViewCacheEntry::Loading);
                    true
                }
            })
            .unwrap_or(false);
        let run_history = history_cache
            .lock()
            .map(|mut cache| {
                if cache.contains(&cache_key) {
                    false
                } else {
                    cache.insert(cache_key.clone(), ViewCacheEntry::Loading);
                    true
                }
            })
            .unwrap_or(false);
        if !run_blame && !run_history {
            return;
        }

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                // Run both in parallel on the background executor.
                let blame_key = cache_key.clone();
                let history_key = cache_key;

                let blame_fut = cx.background_executor().spawn({
                    let cache = blame_cache.clone();
                    async move {
                        if !run_blame {
                            return;
                        }
                        let entry = if invalid_commit {
                            ViewCacheEntry::Unavailable("Invalid commit identifier".to_string())
                        } else {
                            match rgitui_git::compute_blame(&repo1, &blame_path, commit_oid) {
                                Ok(lines) if !lines.is_empty() => ViewCacheEntry::Ready(lines),
                                Ok(_) => ViewCacheEntry::Unavailable(
                                    "The file has no blameable lines".to_string(),
                                ),
                                Err(error) => ViewCacheEntry::Unavailable(error.to_string()),
                            }
                        };
                        if let Ok(mut cache) = cache.lock() {
                            cache.insert(blame_key, entry);
                        }
                    }
                });

                let history_fut = cx.background_executor().spawn({
                    let cache = history_cache.clone();
                    async move {
                        if !run_history {
                            return;
                        }
                        let entry = if invalid_commit {
                            ViewCacheEntry::Unavailable("Invalid commit identifier".to_string())
                        } else {
                            match rgitui_git::compute_file_history_at(
                                &repo2,
                                &history_path,
                                50,
                                commit_oid,
                            ) {
                                Ok(commits) if !commits.is_empty() => {
                                    ViewCacheEntry::Ready(commits)
                                }
                                Ok(_) => ViewCacheEntry::Unavailable(
                                    "The file has no committed history".to_string(),
                                ),
                                Err(error) => ViewCacheEntry::Unavailable(error.to_string()),
                            }
                        };
                        if let Ok(mut cache) = cache.lock() {
                            cache.insert(history_key, entry);
                        }
                    }
                });

                blame_fut.await;
                history_fut.await;
                this.update(cx, |_workspace, cx| cx.notify()).ok();
            },
        )
        .detach();
    }
}
