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
pub(crate) use undo::{UndoAction, UndoEntry, UndoStack};

use std::path::PathBuf;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    div, Bounds, Context, Entity, EventEmitter, Render, SharedString, Window, WindowHandle,
};
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
    GlobalSearch,
    Bisect,
}

/// Which view is active in the right panel column above the commit panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RightPanelMode {
    Details,
    Issues,
    PullRequests,
    BranchHealth,
    Stashes,
}

/// A single open project tab.
/// Shared LRU caches for blame and file history, populated in the background
/// when a diff is opened so that switching to blame/history is near-instant.
#[derive(Clone)]
pub(super) struct ViewCaches {
    pub blame: std::sync::Arc<
        std::sync::Mutex<crate::cache::LruCache<String, Vec<rgitui_git::BlameLine>>>,
    >,
    pub history: std::sync::Arc<
        std::sync::Mutex<crate::cache::LruCache<String, Vec<rgitui_git::CommitInfo>>>,
    >,
    pub diff: std::sync::Arc<
        std::sync::Mutex<crate::cache::LruCache<git2::Oid, std::sync::Arc<rgitui_git::CommitDiff>>>,
    >,
}

impl ViewCaches {
    fn new() -> Self {
        Self {
            blame: std::sync::Arc::new(std::sync::Mutex::new(crate::cache::LruCache::new(8))),
            history: std::sync::Arc::new(std::sync::Mutex::new(crate::cache::LruCache::new(8))),
            diff: std::sync::Arc::new(std::sync::Mutex::new(crate::cache::LruCache::new(200))),
        }
    }
}

#[derive(Clone)]
pub(super) struct ProjectTab {
    pub name: String,
    pub project: Entity<GitProject>,
    pub graph: Entity<rgitui_graph::GraphView>,
    pub diff_viewer: Entity<rgitui_diff::DiffViewer>,
    pub blame_view: Entity<crate::BlameView>,
    pub file_history_view: Entity<crate::FileHistoryView>,
    pub reflog_view: Entity<crate::ReflogView>,
    pub bisect_view: Entity<crate::BisectView>,
    pub submodule_view: Entity<crate::SubmoduleView>,
    pub detail_panel: Entity<crate::DetailPanel>,
    pub sidebar: Entity<crate::Sidebar>,
    pub commit_panel: Entity<crate::CommitPanel>,
    pub toolbar: Entity<crate::Toolbar>,
    pub issues_panel: Entity<crate::IssuesPanel>,
    pub prs_panel: Entity<crate::PrsPanel>,
    pub branch_health_panel: Entity<crate::BranchHealthPanel>,
    pub stashes_panel: Entity<crate::StashesPanel>,
    pub global_search_view: Entity<crate::GlobalSearchView>,
    pub right_panel_mode: RightPanelMode,
    pub bottom_panel_mode: BottomPanelMode,
    pub caches: ViewCaches,
    pub inspecting_worktree: Option<InspectingWorktree>,
}

#[derive(Clone, Debug)]
pub(super) struct InspectingWorktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
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

/// A pending "new release available" banner shown above the status bar.
#[derive(Debug, Clone)]
pub(super) struct UpdateNotification {
    pub latest_version: String,
    pub current_version: String,
    pub release_url: String,
}

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
    pub(super) status_message_gen: u64,
    pub(super) undo_stack: UndoStack,
    pub(super) layout_save_task: Option<gpui::Task<()>>,
    pub(super) cached_ui_font: Option<(String, gpui::Font)>,
    pub(super) update_notification: Option<UpdateNotification>,
    pub(super) settings_window: Option<WindowHandle<crate::SettingsWindow>>,
    pub(super) _settings_window_closed_subscription: Option<gpui::Subscription>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Install the cross-window action channel before any child entity that
        // might want to send through it can be created. The settings window
        // lives in a separate OS window, so it can only reach us via this
        // App-scoped global. The receiver pumps actions back onto the
        // workspace entity until the entity is dropped.
        let (settings_action_tx, settings_action_rx) = crate::settings_window::channel();
        cx.set_global(crate::SettingsWindowActionGlobal(settings_action_tx));
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(action) = settings_action_rx.recv().await {
                if this.upgrade().is_none() {
                    break;
                }
                let weak = this.clone();
                cx.update(|app| {
                    if let Some(entity) = weak.upgrade() {
                        entity.update(app, |workspace, cx| {
                            workspace.handle_settings_window_action(action, cx);
                        });
                    }
                });
            }
        })
        .detach();

        let ai = cx.new(|_cx| AiGenerator::new());
        let command_palette = cx.new(crate::CommandPalette::new);
        let interactive_rebase = cx.new(crate::InteractiveRebase::new);
        let global_search = cx.new(crate::GlobalSearchView::new);
        let theme_editor = cx.new(crate::ThemeEditorDialog::new_for_active_theme);
        let toast_layer = cx.new(ToastLayer::new);

        let branch_dialog = cx.new(crate::BranchDialog::new);
        let tag_dialog = cx.new(crate::TagDialog::new);
        let worktree_dialog = cx.new(crate::WorktreeDialog::new);
        let rename_dialog = cx.new(crate::RenameDialog::new);
        let confirm_dialog = cx.new(crate::ConfirmDialog::new);
        let stash_branch_dialog = cx.new(crate::StashBranchDialog::new);
        let create_pr_dialog = cx.new(crate::CreatePrDialog::new);
        let repo_opener = cx.new(crate::RepoOpener::new);
        let shortcuts_help = cx.new(crate::ShortcutsHelp::new);

        // Set up all event subscriptions
        events::subscribe_interactive_rebase(cx, &interactive_rebase);
        events::subscribe_ai(cx, &ai);
        events::subscribe_command_palette(cx, &command_palette);
        events::subscribe_branch_dialog(cx, &branch_dialog);
        events::subscribe_tag_dialog(cx, &tag_dialog);
        events::subscribe_worktree_dialog(cx, &worktree_dialog);
        events::subscribe_rename_dialog(cx, &rename_dialog);
        events::subscribe_confirm_dialog(cx, &confirm_dialog);
        events::subscribe_stash_branch_dialog(cx, &stash_branch_dialog);
        events::subscribe_create_pr_dialog(cx, &create_pr_dialog);
        events::subscribe_repo_opener(cx, &repo_opener);
        events::subscribe_shortcuts_help(cx, &shortcuts_help);
        events::subscribe_global_search(cx, &global_search);

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
        let commit_input_height = layout_settings.commit_input_height.max(300.0);

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
                worktree_dialog,
                stash_branch_dialog,
                create_pr_dialog,
            },
            overlays: OverlayState {
                command_palette,
                interactive_rebase,
                repo_opener,
                shortcuts_help,
                global_search,
                theme_editor,
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
            status_message_gen: 0,
            undo_stack: UndoStack::new(),
            layout_save_task: None,
            cached_ui_font: None,
            update_notification: None,
            settings_window: None,
            _settings_window_closed_subscription: None,
        }
    }

    /// Open the settings window, or focus it if one is already open.
    ///
    /// Maintains a single `WindowHandle<SettingsWindow>` on the workspace as a
    /// duplicate-window guard. Stale handles (a window that closed before the
    /// `on_window_closed` observer cleared the field) are detected and replaced.
    pub(crate) fn open_or_focus_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window {
            match handle.update(cx, |_, window, _| window.activate_window()) {
                Ok(()) => return,
                Err(_) => {
                    self.settings_window = None;
                }
            }
        }

        let options = crate::settings_window_options(cx);
        let opened = cx.open_window(options, |_, cx| cx.new(crate::SettingsWindow::new));

        let handle = match opened {
            Ok(handle) => handle,
            Err(e) => {
                log::error!("failed to open settings window: {}", e);
                return;
            }
        };

        self.settings_window = Some(handle);

        let target_id = handle.window_id();
        let weak = cx.entity().downgrade();
        let subscription = cx.on_window_closed(move |app| {
            if app.windows().iter().any(|w| w.window_id() == target_id) {
                return;
            }
            if let Some(this) = weak.upgrade() {
                this.update(app, |this, _| {
                    if this
                        .settings_window
                        .map(|h| h.window_id() == target_id)
                        .unwrap_or(false)
                    {
                        this.settings_window = None;
                    }
                });
            }
            // Flush in-memory bounds updates accumulated during the window's
            // lifetime to disk now that the window is gone.
            if app.has_global::<rgitui_settings::SettingsState>() {
                app.update_global::<rgitui_settings::SettingsState, _>(|state, _| {
                    if let Err(error) = state.save() {
                        log::warn!("Failed to persist settings window bounds: {}", error);
                    }
                });
            }
        });
        self._settings_window_closed_subscription = Some(subscription);
    }

    /// Set a status bar message that auto-clears after 5 seconds.
    pub(super) fn set_status_message(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.status_message = Some(msg.into());
        self.status_message_gen += 1;
        let gen = self.status_message_gen;
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
                this.update(cx, |this, cx| {
                    if this.status_message_gen == gen {
                        this.status_message = None;
                        cx.notify();
                    }
                })
                .ok();
            },
        )
        .detach();
    }

    /// Start background tasks like update checking.
    pub fn start_background_tasks(&self, cx: &mut Context<Self>) {
        // Check for app updates in the background
        update_checker::check_for_updates(cx.entity().downgrade(), cx);
    }

    /// Called by the update checker when a newer release is detected.
    pub(super) fn set_update_notification(
        &mut self,
        notification: UpdateNotification,
        cx: &mut Context<Self>,
    ) {
        self.update_notification = Some(notification);
        cx.notify();
    }

    /// Dismiss the in-app update notification.
    pub(super) fn dismiss_update_notification(&mut self, cx: &mut Context<Self>) {
        if self.update_notification.take().is_some() {
            cx.notify();
        }
    }

    /// Dispatch a [`crate::SettingsWindowAction`] received from the settings
    /// window's cross-window channel.
    fn handle_settings_window_action(
        &mut self,
        action: crate::SettingsWindowAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            crate::SettingsWindowAction::OpenThemeEditor => {
                self.overlays
                    .theme_editor
                    .update(cx, |te, cx| te.show_for_active_theme(cx));
            }
            crate::SettingsWindowAction::ThemeChanged(_) => {
                cx.notify();
            }
            crate::SettingsWindowAction::SettingsChanged => {
                self.on_settings_changed(cx);
            }
            crate::SettingsWindowAction::Toast(kind, message) => {
                self.show_toast(message, kind, cx);
            }
        }
    }

    /// Re-configure issues and PR panels with the latest GitHub token after
    /// settings have been saved. Called when the settings window dispatches
    /// `SettingsWindowAction::SettingsChanged`.
    pub(super) fn on_settings_changed(&mut self, cx: &mut Context<Self>) {
        let token = rgitui_settings::current_auth_runtime()
            .git
            .providers
            .iter()
            .find(|p| p.host == "github.com")
            .and_then(|p| p.token.clone());

        for tab in &self.tabs {
            let remotes = tab.project.read(cx).remotes();
            let remote_url = remotes
                .iter()
                .find(|r| r.name == "origin")
                .or_else(|| remotes.first())
                .and_then(|r| r.url.clone());

            if let Some(url) = remote_url {
                if let Some((owner, repo_name)) = crate::issues_panel::parse_github_owner_repo(&url)
                {
                    tab.issues_panel.update(cx, |ip, cx| {
                        ip.configure(token.clone(), owner.clone(), repo_name.clone(), cx);
                    });
                    tab.prs_panel.update(cx, |pp, cx| {
                        pp.configure(token.clone(), owner.clone(), repo_name.clone(), cx);
                    });
                }
            }
        }

        cx.notify();
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

    pub(super) fn effective_worktree_path(&self, cx: &gpui::App) -> PathBuf {
        self.tabs
            .get(self.active_tab)
            .and_then(|tab| tab.inspecting_worktree.as_ref().map(|w| w.path.clone()))
            .unwrap_or_else(|| {
                self.tabs
                    .get(self.active_tab)
                    .map(|tab| tab.project.read(cx).repo_path().to_path_buf())
                    .unwrap_or_default()
            })
    }

    /// Open the GitHub PR creation dialog with the current branch as head
    /// and "main" as the default base.
    pub fn open_create_pr_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let head_branch = self
            .active_project()
            .and_then(|p| p.read(cx).head_branch().map(String::from));
        let Some(head) = head_branch else { return };
        let token = tab.prs_panel.read(cx).github_token().map(String::from);
        let owner = tab.prs_panel.read(cx).github_owner().to_string();
        let repo = tab.prs_panel.read(cx).github_repo().to_string();
        self.dialogs.create_pr_dialog.update(cx, |d, cx| {
            d.configure(token, owner, repo, cx);
            // Default base is "main" — user can edit in the dialog
            d.show_visible(head, "main".to_string(), cx);
        });
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

    pub(super) fn build_ui_font(primary: String) -> gpui::Font {
        let candidates = [
            "JetBrains Mono",
            "JetBrainsMono Nerd Font",
            "Lilex",
            #[cfg(target_os = "windows")]
            "Cascadia Code",
            #[cfg(target_os = "macos")]
            "SF Mono",
            #[cfg(target_os = "macos")]
            "Menlo",
            #[cfg(target_os = "linux")]
            "DejaVu Sans Mono",
            "Courier New",
        ];
        let fallbacks: Vec<String> = candidates
            .iter()
            .filter(|c| **c != primary)
            .map(|c| c.to_string())
            .collect();

        let mut f = gpui::font(SharedString::from(primary));
        f.fallbacks = Some(gpui::FontFallbacks::from_fonts(fallbacks));
        f
    }
}
