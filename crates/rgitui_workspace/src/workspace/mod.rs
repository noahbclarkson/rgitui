mod commands;
mod events;
mod key_handler;
mod layout;
mod operations;
mod state;
mod tabs;
mod undo;
mod update_checker;

pub(crate) use state::*;
pub(crate) use undo::{UndoAction, UndoEntry};

use std::time::Instant;

use gpui::prelude::*;
use gpui::{div, Bounds, Context, Entity, EventEmitter, Render, SharedString, Window};
use rgitui_ai::AiGenerator;
use rgitui_git::GitProject;

use crate::{ToastKind, ToastLayer};

/// Marker types for drag-resize handles — each implements Render to serve as the drag ghost view.
#[derive(Clone)]
pub(super) struct SidebarResize;
impl Render for SidebarResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
pub(super) struct DetailPanelResize;
impl Render for DetailPanelResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
pub(super) struct DiffViewerResize;
impl Render for DiffViewerResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
pub(super) struct CommitInputResize;
impl Render for CommitInputResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Which view is active in the bottom panel (diff viewer area).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BottomPanelMode {
    Diff,
    Blame,
    FileHistory,
    Reflog,
    Submodules,
}

/// Which view is active in the right panel column above the commit panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RightPanelMode {
    Details,
    Issues,
}

/// A single open project tab.
#[derive(Clone)]
pub(super) struct ProjectTab {
    pub name: String,
    pub project: Entity<GitProject>,
    pub graph: Entity<rgitui_graph::GraphView>,
    pub diff_viewer: Entity<rgitui_diff::DiffViewer>,
    pub blame_view: Entity<crate::BlameView>,
    pub file_history_view: Entity<crate::FileHistoryView>,
    pub reflog_view: Entity<crate::ReflogView>,
    pub submodule_view: Entity<crate::SubmoduleView>,
    pub detail_panel: Entity<crate::DetailPanel>,
    pub sidebar: Entity<crate::Sidebar>,
    pub commit_panel: Entity<crate::CommitPanel>,
    pub toolbar: Entity<crate::Toolbar>,
    pub issues_panel: Entity<crate::IssuesPanel>,
    pub right_panel_mode: RightPanelMode,
    pub bottom_panel_mode: BottomPanelMode,
}

/// Events from the workspace.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    OpenRepo(String),
}

/// Which panel had focus before a modal was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusedPanel {
    Sidebar,
    Graph,
    DetailPanel,
    DiffViewer,
}

/// Tracks a long-running git operation in progress.
pub(super) struct ActiveOperation {
    pub id: u64,
    pub label: SharedString,
    pub started_at: Instant,
}

/// Stores the result of a completed git operation for display in the output bar.
pub(super) struct OperationOutput {
    pub operation: SharedString,
    pub output: String,
    pub is_error: bool,
    pub timestamp: Instant,
    pub expanded: bool,
}

pub(super) const OPERATION_OUTPUT_AUTO_HIDE_SECS: u64 = 10;

/// The root workspace view.
pub struct Workspace {
    pub(super) tabs: Vec<ProjectTab>,
    pub(super) active_tab: usize,
    pub(super) ai: Entity<AiGenerator>,
    pub(super) layout: LayoutState,
    pub(super) dialogs: DialogState,
    pub(super) overlays: OverlayState,
    pub(super) operations: OperationState,
    pub(super) focus: FocusState,
    pub(super) toast_layer: Entity<ToastLayer>,
    pub(super) active_workspace_id: Option<String>,
    pub(super) status_message: Option<String>,
    pub(super) undo_history: Vec<UndoEntry>,
    pub(super) layout_save_task: Option<gpui::Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ai = cx.new(|_cx| AiGenerator::new());
        let command_palette = cx.new(crate::CommandPalette::new);
        let interactive_rebase = cx.new(crate::InteractiveRebase::new);
        let settings_modal = cx.new(crate::SettingsModal::new);
        let toast_layer = cx.new(ToastLayer::new);

        let branch_dialog = cx.new(crate::BranchDialog::new);
        let tag_dialog = cx.new(crate::TagDialog::new);
        let rename_dialog = cx.new(crate::RenameDialog::new);
        let confirm_dialog = cx.new(crate::ConfirmDialog::new);
        let repo_opener = cx.new(crate::RepoOpener::new);
        let shortcuts_help = cx.new(crate::ShortcutsHelp::new);

        // Set up all event subscriptions
        events::subscribe_settings_modal(cx, &settings_modal);
        events::subscribe_interactive_rebase(cx, &interactive_rebase);
        events::subscribe_ai(cx, &ai);
        events::subscribe_command_palette(cx, &command_palette);
        events::subscribe_branch_dialog(cx, &branch_dialog);
        events::subscribe_tag_dialog(cx, &tag_dialog);
        events::subscribe_rename_dialog(cx, &rename_dialog);
        events::subscribe_confirm_dialog(cx, &confirm_dialog);
        events::subscribe_repo_opener(cx, &repo_opener);
        events::subscribe_shortcuts_help(cx, &shortcuts_help);

        // Restore layout dimensions from saved settings
        let layout_settings = if let Some(state) = cx.try_global::<rgitui_settings::SettingsState>()
        {
            state.settings().layout.clone()
        } else {
            rgitui_settings::LayoutSettings::default()
        };
        let sidebar_width = layout_settings.sidebar_width;
        let detail_panel_width = layout_settings.detail_panel_width;
        let diff_viewer_height = layout_settings.diff_viewer_height;
        let commit_input_height = layout_settings.commit_input_height.max(240.0);

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            ai,
            layout: LayoutState {
                sidebar_width,
                detail_panel_width,
                diff_viewer_height,
                commit_input_height,
                content_bounds: Bounds::default(),
                right_panel_bounds: Bounds::default(),
            },
            dialogs: DialogState {
                branch_dialog,
                tag_dialog,
                rename_dialog,
                confirm_dialog,
            },
            overlays: OverlayState {
                command_palette,
                interactive_rebase,
                settings_modal,
                repo_opener,
                shortcuts_help,
            },
            operations: OperationState {
                active_git_operation: None,
                last_failed_git_operation: None,
                active_operations: Vec::new(),
                last_operation_output: None,
                is_loading: false,
                loading_message: None,
            },
            focus: FocusState {
                last_focused_panel: None,
                pending_focus_restore: false,
                crash_recovery_available: false,
                crash_recovery_shown: false,
            },
            toast_layer,
            active_workspace_id: None,
            status_message: None,
            undo_history: Vec::new(),
            layout_save_task: None,
        }
    }

    /// Start background tasks like update checking.
    pub fn start_background_tasks(&self, cx: &mut Context<Self>) {
        // Check for app updates in the background
        update_checker::check_for_updates(cx.entity().downgrade(), cx);
    }

    /// Set whether crash recovery is available (previous session didn't exit cleanly).
    pub fn set_crash_recovery_available(&mut self, available: bool) {
        self.focus.crash_recovery_available = available;
    }

    /// Show crash recovery toast if available. Called after workspace is fully loaded.
    pub fn show_crash_recovery_toast(&mut self, cx: &mut Context<Self>) {
        if self.focus.crash_recovery_available && !self.focus.crash_recovery_shown {
            self.focus.crash_recovery_shown = true;
            // The workspace was already restored, just inform the user
            self.show_toast(
                "Restored from previous session (unclean exit detected)",
                ToastKind::Info,
                cx,
            );
        }
    }

    /// Mark a clean exit when the user explicitly closes or goes home.
    pub fn mark_clean_exit(&self, cx: &mut Context<Self>) {
        cx.update_global::<rgitui_settings::SettingsState, _>(|settings, _| {
            settings.mark_clean_exit();
        });
    }

    /// Show a temporary toast notification.
    pub(super) fn show_toast(
        &mut self,
        text: impl Into<String>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        let message = text.into();
        self.toast_layer
            .update(cx, |layer, cx| layer.show_toast(message.clone(), kind, cx));
    }

    pub fn active_project(&self) -> Option<&Entity<GitProject>> {
        self.tabs.get(self.active_tab).map(|t| &t.project)
    }

    /// Detect which panel is currently focused and save it for later restoration.
    pub(super) fn save_focus(&mut self, window: &Window, cx: &Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if tab.sidebar.read(cx).is_focused(window) {
                self.focus.last_focused_panel = Some(FocusedPanel::Sidebar);
            } else if tab.graph.read(cx).is_focused(window) {
                self.focus.last_focused_panel = Some(FocusedPanel::Graph);
            } else if tab.detail_panel.read(cx).is_focused(window) {
                self.focus.last_focused_panel = Some(FocusedPanel::DetailPanel);
            } else if tab.diff_viewer.read(cx).is_focused(window)
                || tab.blame_view.read(cx).is_focused(window)
            {
                self.focus.last_focused_panel = Some(FocusedPanel::DiffViewer);
            }
        }
    }

    /// Detect which panel currently has focus.
    pub(super) fn current_focused_panel(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<FocusedPanel> {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if tab.sidebar.read(cx).is_focused(window) {
                return Some(FocusedPanel::Sidebar);
            }
            if tab.graph.read(cx).is_focused(window) {
                return Some(FocusedPanel::Graph);
            }
            if tab.detail_panel.read(cx).is_focused(window) {
                return Some(FocusedPanel::DetailPanel);
            }
            if tab.diff_viewer.read(cx).is_focused(window)
                || tab.blame_view.read(cx).is_focused(window)
            {
                return Some(FocusedPanel::DiffViewer);
            }
        }
        None
    }

    /// Cycle focus to the next panel in order.
    pub(super) fn focus_next_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_focused_panel(window, cx);
        let next = match current {
            Some(FocusedPanel::Sidebar) => FocusedPanel::Graph,
            Some(FocusedPanel::Graph) => FocusedPanel::DetailPanel,
            Some(FocusedPanel::DetailPanel) => FocusedPanel::DiffViewer,
            Some(FocusedPanel::DiffViewer) => FocusedPanel::Sidebar,
            None => FocusedPanel::Graph,
        };
        self.focus_panel(next, window, cx);
    }

    /// Cycle focus to the previous panel in order.
    pub(super) fn focus_prev_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_focused_panel(window, cx);
        let prev = match current {
            Some(FocusedPanel::Sidebar) => FocusedPanel::DiffViewer,
            Some(FocusedPanel::Graph) => FocusedPanel::Sidebar,
            Some(FocusedPanel::DetailPanel) => FocusedPanel::Graph,
            Some(FocusedPanel::DiffViewer) => FocusedPanel::DetailPanel,
            None => FocusedPanel::Graph,
        };
        self.focus_panel(prev, window, cx);
    }

    /// Focus a specific panel.
    pub(super) fn focus_panel(
        &mut self,
        panel: FocusedPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            match panel {
                FocusedPanel::Sidebar => {
                    tab.sidebar.update(cx, |s, cx| s.focus(window, cx));
                }
                FocusedPanel::Graph => {
                    tab.graph.update(cx, |g, cx| g.focus(window, cx));
                }
                FocusedPanel::DetailPanel => {
                    tab.detail_panel.update(cx, |d, cx| d.focus(window, cx));
                }
                FocusedPanel::DiffViewer => {
                    if tab.bottom_panel_mode == BottomPanelMode::Blame {
                        tab.blame_view.update(cx, |bv, cx| bv.focus(window, cx));
                    } else {
                        tab.diff_viewer.update(cx, |d, cx| d.focus(window, cx));
                    }
                }
            }
        }
    }

    /// Restore focus to the previously focused panel.
    pub(super) fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.focus.last_focused_panel.take();
        if let Some(panel) = panel {
            self.focus_panel(panel, window, cx);
        }
    }
}
