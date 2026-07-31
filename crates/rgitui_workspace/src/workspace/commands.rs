use gpui::{Context, Window};

use crate::{CommandId, CommitPanelEvent, ConfirmAction, ToastKind};

use super::layout::{
    MAX_DETAIL_PANEL_WIDTH, MAX_DIFF_VIEWER_HEIGHT, MIN_DETAIL_PANEL_WIDTH, MIN_DIFF_VIEWER_HEIGHT,
};
use super::{
    BottomPanelMode, FocusedPanel, ProjectTab, RightPanelMode, ViewCacheEntry, ViewCacheKey,
    ViewCaches, Workspace,
};

/// Pixels the detail panel grows or shrinks by per keystroke.
const DETAIL_PANEL_STEP: f32 = 20.0;
/// Pixels the diff viewer grows or shrinks by per keystroke.
const DIFF_VIEWER_STEP: f32 = 30.0;

impl Workspace {
    /// Entry point for keyboard-invoked commands.
    ///
    /// The generated `on_action` handlers (see [`crate::keymap::attach_actions`])
    /// all land here. Commands that need a [`Window`] — to move focus or to
    /// focus an overlay's input — are handled directly; everything else goes to
    /// [`Self::execute_command`], which is also what the command palette calls.
    pub(super) fn dispatch_command(
        &mut self,
        cmd: CommandId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            CommandId::CommandPalette => {
                self.save_focus(window, cx);
                self.overlays.command_palette.update(cx, |palette, cx| {
                    palette.toggle(window, cx);
                });
            }
            CommandId::Settings => {
                self.save_focus(window, cx);
                self.open_or_focus_settings(cx);
            }
            CommandId::OpenRepo => {
                self.save_focus(window, cx);
                self.overlays.repo_opener.update(cx, |opener, cx| {
                    opener.toggle(window, cx);
                });
            }
            CommandId::Shortcuts => {
                self.save_focus(window, cx);
                self.overlays.shortcuts_help.update(cx, |help, cx| {
                    help.toggle(window, cx);
                });
            }
            // Switching branches means focusing the sidebar's branch list.
            CommandId::SwitchBranch => {
                self.focus_panel(FocusedPanel::Sidebar, window, cx);
            }
            CommandId::FocusSidebar => self.focus_panel(FocusedPanel::Sidebar, window, cx),
            CommandId::FocusGraph => self.focus_panel(FocusedPanel::Graph, window, cx),
            CommandId::FocusDetailPanel => self.focus_panel(FocusedPanel::DetailPanel, window, cx),
            CommandId::FocusDiffViewer => self.focus_panel(FocusedPanel::DiffViewer, window, cx),
            CommandId::FocusNextPanel => self.focus_next_panel(window, cx),
            CommandId::FocusPrevPanel => self.focus_prev_panel(window, cx),
            CommandId::Search => self.toggle_graph_search(window, cx),
            CommandId::GlobalSearch => self.toggle_global_search(window, cx),
            cmd => self.execute_command(cmd, cx),
        }
    }

    /// Runs a `graph::*` command against the active tab's commit graph.
    ///
    /// `GraphView` lives in `rgitui_graph`, which cannot depend on this crate and
    /// therefore cannot name the actions. The workspace root is an ancestor of
    /// the graph on every dispatch path, so handling them here still means the
    /// bindings only fire while the graph holds focus — that is what the
    /// `GraphView` key context on its root element is for.
    pub(super) fn dispatch_graph_command(
        &mut self,
        cmd: CommandId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(graph) = self.tabs.get(self.active_tab).map(|tab| tab.graph.clone()) else {
            cx.propagate();
            return;
        };
        // Squashing needs the project, the rebase dialog and the toast queue, so it
        // is handled on the workspace rather than inside `graph.update`.
        if cmd == CommandId::SquashSelected {
            self.squash_selected_commits(cx);
            return;
        }
        graph.update(cx, |graph, cx| match cmd {
            CommandId::GraphSelectNext => graph.select_next_row(cx),
            CommandId::GraphSelectPrev => graph.select_prev_row(cx),
            CommandId::GraphSelectFirst => graph.select_first_row(cx),
            CommandId::GraphSelectLast => graph.select_last_row(cx),
            CommandId::GraphExtendSelectionNext => graph.extend_selection_next(cx),
            CommandId::GraphExtendSelectionPrev => graph.extend_selection_prev(cx),
            CommandId::GraphCancel => graph.cancel(window, cx),
            CommandId::CopyCommitSha => graph.copy_selected_sha(cx),
            CommandId::CopyCommitMessage => graph.copy_selected_message(cx),
            _ => cx.propagate(),
        });
    }

    /// Pre-fills the interactive rebase dialog with a plan that squashes the
    /// commits selected in the graph into the oldest of them.
    ///
    /// Nothing is executed here: the plan goes into the dialog so the user can
    /// review it, adjust the actions and confirm through the dialog's own
    /// `Execute` path. Validation happens first — see [`crate::squash`] for why —
    /// and a rejection becomes a toast that names the condition that failed.
    ///
    /// Both routes to squashing end here: the `graph::SquashSelected` keystroke
    /// and the graph's "Squash selected commits" context-menu item, which only
    /// checks that two rows are selected before emitting.
    pub(super) fn squash_selected_commits(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let graph = tab.graph.clone();
        let project = tab.project.clone();
        let selected = graph.read(cx).selected_commit_oids();

        let planned = {
            let proj = project.read(cx);
            proj.head_oid_at(proj.repo_path()).map(|head_oid| {
                crate::squash::plan_squash(proj.recent_commits(), head_oid, &selected)
            })
        };

        match planned {
            Some(Ok(plan)) => {
                self.overlays.interactive_rebase.update(cx, |ir, cx| {
                    ir.show_visible(
                        plan.entries,
                        format!("{} (rebase onto)", plan.base_label),
                        cx,
                    );
                });
            }
            Some(Err(rejection)) => {
                self.show_toast(rejection.message(), ToastKind::Warning, cx);
            }
            None => {
                self.show_toast(
                    "Could not resolve HEAD, so there is nothing to squash onto. Refresh and \
                     try again.",
                    ToastKind::Warning,
                    cx,
                );
            }
        }
    }

    /// Runs a `diff::*` command against the active tab's diff viewer.
    ///
    /// Handled here for the same reason as [`Self::dispatch_graph_command`]:
    /// `rgitui_diff` sits below this crate and cannot name the actions.
    pub(super) fn dispatch_diff_command(
        &mut self,
        cmd: CommandId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(diff) = self
            .tabs
            .get(self.active_tab)
            .map(|tab| tab.diff_viewer.clone())
        else {
            cx.propagate();
            return;
        };
        diff.update(cx, |diff, cx| match cmd {
            CommandId::DiffSelectNext => diff.select_next_row(cx),
            CommandId::DiffSelectPrev => diff.select_prev_row(cx),
            CommandId::DiffSelectFirst => diff.select_first_row(cx),
            CommandId::DiffSelectLast => diff.select_last_row(cx),
            CommandId::NextHunk => diff.select_next_hunk(cx),
            CommandId::PrevHunk => diff.select_prev_hunk(cx),
            CommandId::ToggleDiffDisplayMode => diff.toggle_display_mode(cx),
            CommandId::TogglePartialSelection => diff.toggle_partial_mode(cx),
            CommandId::StageSelection => diff.stage_selection(cx),
            CommandId::UnstageSelection => diff.unstage_selection(cx),
            CommandId::StageCurrentHunk => diff.stage_current_hunk(cx),
            CommandId::UnstageCurrentHunk => diff.unstage_current_hunk(cx),
            CommandId::CopyDiffSelection => diff.copy_selection(cx),
            CommandId::SelectAllDiffLines => diff.select_all_lines(cx),
            _ => cx.propagate(),
        });
    }

    /// Toggles the commit graph's search field, focusing it when it opens.
    fn toggle_graph_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let graph = tab.graph.clone();
        graph.update(cx, |graph, cx| {
            graph.toggle_search_focused(window, cx);
        });
    }

    /// Widens (positive `delta`) or narrows the right-hand detail panel.
    fn resize_detail_panel(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.layout.detail_panel_width = (self.layout.detail_panel_width + delta)
            .clamp(MIN_DETAIL_PANEL_WIDTH, MAX_DETAIL_PANEL_WIDTH);
        self.schedule_layout_save(cx);
        cx.notify();
    }

    /// Heightens (positive `delta`) or shortens the bottom diff viewer.
    fn resize_diff_viewer(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.layout.diff_viewer_height = (self.layout.diff_viewer_height + delta)
            .clamp(MIN_DIFF_VIEWER_HEIGHT, MAX_DIFF_VIEWER_HEIGHT);
        self.schedule_layout_save(cx);
        cx.notify();
    }

    /// Swaps the bottom panel between the diff viewer and the working-tree search.
    fn toggle_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.bottom_panel_mode == BottomPanelMode::GlobalSearch {
            tab.global_search_view
                .update(cx, |search, cx| search.hide(cx));
            tab.bottom_panel_mode = BottomPanelMode::Diff;
        } else {
            tab.bottom_panel_mode = BottomPanelMode::GlobalSearch;
            tab.global_search_view.update(cx, |search, cx| {
                search.show(window, cx);
            });
        }
        cx.notify();
    }

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
            CommandId::OpenKeymap => {
                self.open_keymap_file(cx);
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
            CommandId::NextTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    cx.notify();
                }
            }
            CommandId::PrevTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = if self.active_tab == 0 {
                        self.tabs.len() - 1
                    } else {
                        self.active_tab - 1
                    };
                    cx.notify();
                }
            }
            CommandId::CloseTab => {
                if !self.tabs.is_empty() {
                    self.close_tab(self.active_tab, cx);
                }
            }
            CommandId::ShrinkDetailPanel => self.resize_detail_panel(-DETAIL_PANEL_STEP, cx),
            CommandId::GrowDetailPanel => self.resize_detail_panel(DETAIL_PANEL_STEP, cx),
            CommandId::ShrinkDiffViewer => self.resize_diff_viewer(-DIFF_VIEWER_STEP, cx),
            CommandId::GrowDiffViewer => self.resize_diff_viewer(DIFF_VIEWER_STEP, cx),
            // Toggling the palette needs a `Window`, so it is handled in
            // `dispatch_command`. It is `[hidden]`, so the palette never
            // dispatches it to itself. The panel-focus commands likewise need a
            // `Window` and are `[hidden]`.
            CommandId::CommandPalette
            | CommandId::FocusSidebar
            | CommandId::FocusGraph
            | CommandId::FocusDetailPanel
            | CommandId::FocusDiffViewer
            | CommandId::FocusNextPanel
            | CommandId::FocusPrevPanel => {}
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
                    cp.request_commit(cx);
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
                let hint = crate::keymap::shortcut(CommandId::FocusSidebar, cx)
                    .map(|keystrokes| format!("Press {keystrokes} to "))
                    .unwrap_or_else(|| "Use the sidebar to ".to_owned());
                self.show_toast(
                    format!("{hint}focus the sidebar for branch switching"),
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
            | CommandId::OpenKeymap
            | CommandId::WorkspaceHome
            | CommandId::RestoreLastWorkspace
            | CommandId::Undo
            | CommandId::OpenThemeEditor
            | CommandId::CommandPalette
            | CommandId::NextTab
            | CommandId::PrevTab
            | CommandId::CloseTab
            | CommandId::FocusSidebar
            | CommandId::FocusGraph
            | CommandId::FocusDetailPanel
            | CommandId::FocusDiffViewer
            | CommandId::FocusNextPanel
            | CommandId::FocusPrevPanel
            | CommandId::ShrinkDetailPanel
            | CommandId::GrowDetailPanel
            | CommandId::ShrinkDiffViewer
            | CommandId::GrowDiffViewer => {}
            // View-owned commands. Each is handled by the panel, overlay or
            // dialog whose key context scopes it — the shared `menu` commands on
            // whichever element holds the selection, the `graph` and `diff` ones
            // on the workspace root (see `dispatch_view_command`) because those
            // two views live in crates that cannot name these actions. They are
            // listed rather than swept up by a wildcard so that adding a command
            // forces a decision about where it is handled.
            CommandId::Cancel
            | CommandId::Confirm
            | CommandId::SelectNext
            | CommandId::SelectPrev
            | CommandId::SelectFirst
            | CommandId::SelectLast
            | CommandId::GraphSelectNext
            | CommandId::GraphSelectPrev
            | CommandId::GraphSelectFirst
            | CommandId::GraphSelectLast
            | CommandId::GraphExtendSelectionNext
            | CommandId::GraphExtendSelectionPrev
            | CommandId::SquashSelected
            | CommandId::GraphCancel
            | CommandId::CopyCommitSha
            | CommandId::CopyCommitMessage
            | CommandId::DiffSelectNext
            | CommandId::DiffSelectPrev
            | CommandId::DiffSelectFirst
            | CommandId::DiffSelectLast
            | CommandId::NextHunk
            | CommandId::PrevHunk
            | CommandId::ToggleDiffDisplayMode
            | CommandId::TogglePartialSelection
            | CommandId::StageSelection
            | CommandId::UnstageSelection
            | CommandId::StageCurrentHunk
            | CommandId::UnstageCurrentHunk
            | CommandId::CopyDiffSelection
            | CommandId::SelectAllDiffLines
            | CommandId::ToggleFileTree
            | CommandId::PrevCommitDetails
            | CommandId::NextCommitDetails
            | CommandId::FileSearch
            | CommandId::ToggleStageRow
            | CommandId::DiscardRow
            | CommandId::FilterBranches
            | CommandId::BlameShowDiff
            | CommandId::BlameShowHistory
            | CommandId::HistoryShowDiff
            | CommandId::HistoryShowBlame
            | CommandId::RebaseMoveUp
            | CommandId::RebaseMoveDown
            | CommandId::RebasePick
            | CommandId::RebaseReword
            | CommandId::RebaseSquash
            | CommandId::RebaseFixup
            | CommandId::RebaseDrop
            | CommandId::ThemeEditorNextField
            | CommandId::ThemeEditorPrevField
            | CommandId::SubmitPullRequest => {}
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
