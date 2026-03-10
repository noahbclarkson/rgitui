use anyhow::Result;
use chrono::Local;
use gpui::prelude::*;
use gpui::{
    canvas, div, px, Bounds, Context, DragMoveEvent, ElementId, Entity, EventEmitter, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, Render, SharedString, Window,
};
use rgitui_ai::{AiEvent, AiGenerator};
use rgitui_diff::{DiffViewer, DiffViewerEvent};
use rgitui_git::{
    GitOperationKind, GitOperationState, GitOperationUpdate, GitProject, GitProjectEvent,
};
use rgitui_graph::{GraphView, GraphViewEvent};
use rgitui_settings::{LayoutSettings, StoredWorkspace};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize, Tab,
    TabBar,
};

use crate::{
    BranchDialog, BranchDialogEvent, CommandPalette, CommandPaletteEvent, CommitPanel,
    CommitPanelEvent, ConfirmAction, ConfirmDialog, ConfirmDialogEvent, DetailPanel,
    DetailPanelEvent, InteractiveRebase, InteractiveRebaseEvent, RepoOpener, RepoOpenerEvent,
    SettingsModal, SettingsModalEvent, ShortcutsHelp, ShortcutsHelpEvent, Sidebar, SidebarEvent,
    StatusBar, TitleBar, ToastKind, ToastLayer, Toolbar, ToolbarEvent,
};

/// Marker types for drag-resize handles — each implements Render to serve as the drag ghost view.
#[derive(Clone)]
struct SidebarResize;
impl Render for SidebarResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
struct DetailPanelResize;
impl Render for DetailPanelResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
struct DiffViewerResize;
impl Render for DiffViewerResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
struct CommitInputResize;
impl Render for CommitInputResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A single open project tab.
struct ProjectTab {
    name: String,
    project: Entity<GitProject>,
    graph: Entity<GraphView>,
    diff_viewer: Entity<DiffViewer>,
    detail_panel: Entity<DetailPanel>,
    sidebar: Entity<Sidebar>,
    commit_panel: Entity<CommitPanel>,
    toolbar: Entity<Toolbar>,
}

/// Events from the workspace.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    OpenRepo(String),
}

/// The root workspace view.
pub struct Workspace {
    tabs: Vec<ProjectTab>,
    active_tab: usize,
    ai: Entity<AiGenerator>,
    command_palette: Entity<CommandPalette>,
    interactive_rebase: Entity<InteractiveRebase>,
    settings_modal: Entity<SettingsModal>,
    status_message: Option<String>,
    toast_layer: Entity<ToastLayer>,
    active_workspace_id: Option<String>,
    sidebar_width: f32,
    detail_panel_width: f32,
    diff_viewer_height: f32,
    commit_input_height: f32,
    branch_dialog: Entity<BranchDialog>,
    confirm_dialog: Entity<ConfirmDialog>,
    repo_opener: Entity<RepoOpener>,
    shortcuts_help: Entity<ShortcutsHelp>,
    content_bounds: Bounds<Pixels>,
    right_panel_bounds: Bounds<Pixels>,
    is_loading: bool,
    loading_message: Option<String>,
    active_git_operation: Option<GitOperationUpdate>,
    last_failed_git_operation: Option<GitOperationUpdate>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ai = cx.new(|_cx| AiGenerator::new());
        let command_palette = cx.new(CommandPalette::new);
        let interactive_rebase = cx.new(InteractiveRebase::new);
        let settings_modal = cx.new(SettingsModal::new);
        let toast_layer = cx.new(ToastLayer::new);

        // Subscribe to settings modal events
        cx.subscribe(
            &settings_modal,
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

        // Subscribe to interactive rebase events
        cx.subscribe(
            &interactive_rebase,
            |this, _ir, event: &InteractiveRebaseEvent, cx| match event {
                InteractiveRebaseEvent::Execute(entries) => {
                    let count = entries.len();
                    let msg = format!("Interactive rebase started with {} commits.", count);
                    this.status_message = Some(msg.clone());
                    this.show_toast(msg, ToastKind::Info, cx);
                }
                InteractiveRebaseEvent::Cancel => {
                    cx.notify();
                }
            },
        )
        .detach();

        // Subscribe to AI events to update commit panel when generation completes
        cx.subscribe(&ai, |this, _ai, event: &AiEvent, cx| match event {
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

        // Subscribe to command palette events
        cx.subscribe(
            &command_palette,
            |this, _cp, event: &CommandPaletteEvent, cx| match event {
                CommandPaletteEvent::CommandSelected(cmd_id) => {
                    this.execute_command(cmd_id, cx);
                }
                CommandPaletteEvent::Dismissed => {}
            },
        )
        .detach();

        let branch_dialog = cx.new(BranchDialog::new);
        let confirm_dialog = cx.new(ConfirmDialog::new);
        let repo_opener = cx.new(RepoOpener::new);
        let shortcuts_help = cx.new(ShortcutsHelp::new);

        // Subscribe to branch dialog events
        cx.subscribe(
            &branch_dialog,
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

        // Subscribe to confirm dialog events
        cx.subscribe(
            &confirm_dialog,
            |this, _cd, event: &ConfirmDialogEvent, cx| {
                match event {
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
                                    let name = name.clone();
                                    project.update(cx, |proj, cx| {
                                        proj.delete_branch(&name, cx).detach();
                                    });
                                }
                                ConfirmAction::StashDrop(index) => {
                                    let index = *index;
                                    project.update(cx, |proj, cx| {
                                        proj.stash_drop(index, cx).detach();
                                    });
                                }
                                ConfirmAction::DiscardAll => {
                                    // Discard all unstaged changes
                                    let paths: Vec<std::path::PathBuf> = project
                                        .read(cx)
                                        .status()
                                        .unstaged
                                        .iter()
                                        .map(|f| f.path.clone())
                                        .collect();
                                    if !paths.is_empty() {
                                        project.update(cx, |proj, cx| {
                                            proj.discard_changes(&paths, cx).detach();
                                        });
                                    }
                                }
                                ConfirmAction::TagDelete(name) => {
                                    let name = name.clone();
                                    project.update(cx, |proj, cx| {
                                        proj.delete_tag(&name, cx).detach();
                                    });
                                }
                                ConfirmAction::ResetHard(_target) => {
                                    project.update(cx, |proj, cx| {
                                        proj.reset_hard(cx).detach();
                                    });
                                }
                                ConfirmAction::RemoveRemote(name) => {
                                    let name = name.clone();
                                    project.update(cx, |proj, cx| {
                                        proj.remove_remote(&name, cx).detach();
                                    });
                                }
                            }
                        }
                    }
                    ConfirmDialogEvent::Cancelled => {}
                }
            },
        )
        .detach();

        // Subscribe to repo opener events
        cx.subscribe(
            &repo_opener,
            |this, _ro, event: &RepoOpenerEvent, cx| match event {
                RepoOpenerEvent::OpenRepo(path) => {
                    if let Err(e) = this.open_repo(path.clone(), cx) {
                        this.show_toast(format!("Failed to open: {}", e), ToastKind::Error, cx);
                    }
                }
                RepoOpenerEvent::Dismissed => {}
            },
        )
        .detach();

        // Subscribe to shortcuts help events
        cx.subscribe(
            &shortcuts_help,
            |_this, _sh, _event: &ShortcutsHelpEvent, _cx| {
                // Nothing to do on dismiss
            },
        )
        .detach();

        // Restore layout dimensions from saved settings
        let (sidebar_width, detail_panel_width, diff_viewer_height, commit_input_height) =
            if let Some(state) = cx.try_global::<rgitui_settings::SettingsState>() {
                let layout = &state.settings().layout;
                (
                    layout.sidebar_width,
                    layout.detail_panel_width,
                    layout.diff_viewer_height,
                    layout.commit_input_height,
                )
            } else {
                (240.0, 320.0, 300.0, 200.0)
            };

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            ai,
            command_palette,
            interactive_rebase,
            settings_modal,
            status_message: None,
            toast_layer,
            active_workspace_id: None,
            sidebar_width,
            detail_panel_width,
            diff_viewer_height,
            commit_input_height,
            branch_dialog,
            confirm_dialog,
            repo_opener,
            shortcuts_help,
            content_bounds: Bounds::default(),
            right_panel_bounds: Bounds::default(),
            is_loading: false,
            loading_message: None,
            active_git_operation: None,
            last_failed_git_operation: None,
        }
    }

    /// Execute a command by ID (from command palette or keybindings).
    fn execute_command(&mut self, cmd_id: &str, cx: &mut Context<Self>) {
        // Commands that don't require an active tab
        if cmd_id == "settings" {
            self.settings_modal.update(cx, |sm, cx| {
                sm.toggle_visible(cx);
            });
            return;
        }

        if cmd_id == "create_branch" {
            self.branch_dialog.update(cx, |bd, cx| {
                bd.show_visible(None, cx);
            });
            return;
        }

        if cmd_id == "open_repo" {
            self.repo_opener.update(cx, |ro, cx| {
                ro.toggle_visible(cx);
            });
            return;
        }

        if cmd_id == "shortcuts" {
            self.shortcuts_help.update(cx, |sh, cx| {
                sh.toggle_visible(cx);
            });
            return;
        }

        if cmd_id == "workspace_home" {
            self.go_home(cx);
            return;
        }

        if cmd_id == "restore_last_workspace" {
            self.restore_last_workspace(cx);
            return;
        }

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };

        match cmd_id {
            "fetch" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.fetch_default(cx).detach();
                });
            }
            "pull" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.pull_default(cx).detach();
                });
            }
            "push" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.push_default(false, cx).detach();
                });
            }
            "commit" => {
                let commit_panel = tab.commit_panel.clone();
                commit_panel.update(cx, |cp, cx| {
                    if !cp.message().is_empty() {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: cp.message().to_string(),
                            amend: false,
                        });
                    }
                });
            }
            "stage_all" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.stage_all(cx).detach();
                });
            }
            "unstage_all" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.unstage_all(cx).detach();
                });
            }
            "stash_save" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.stash_save(None, cx).detach();
                });
            }
            "stash_pop" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.stash_pop(0, cx).detach();
                });
            }
            "toggle_diff_mode" => {
                let diff_viewer = tab.diff_viewer.clone();
                diff_viewer.update(cx, |dv, cx| {
                    dv.toggle_display_mode(cx);
                });
            }
            "ai_message" => {
                let commit_panel = tab.commit_panel.clone();
                commit_panel.update(cx, |_cp, cx| {
                    cx.emit(CommitPanelEvent::GenerateAiMessage);
                });
            }
            "merge_branch" => {
                let project = tab.project.clone();
                let head = project.read(cx).head_branch().unwrap_or("HEAD").to_string();
                let msg = format!("Use the sidebar to merge a branch into '{}'", head);
                self.show_toast(msg, ToastKind::Info, cx);
            }
            "refresh" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.refresh(cx).detach();
                });
            }
            "search" => {
                if let Some(tab) = self.tabs.get(self.active_tab) {
                    tab.graph.update(cx, |g, cx| {
                        g.toggle_search(cx);
                    });
                }
            }
            "interactive_rebase" => {
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
                    let target = head_branch;
                    self.interactive_rebase.update(cx, |ir, cx| {
                        ir.show_visible(entries, target, cx);
                    });
                }
            }
            "stash_drop" => {
                let project = tab.project.clone();
                let has_stashes = !project.read(cx).stashes().is_empty();
                if has_stashes {
                    project.update(cx, |proj, cx| {
                        proj.stash_drop(0, cx).detach();
                    });
                } else {
                    self.show_toast("No stashes to drop", ToastKind::Warning, cx);
                }
            }
            "stash_apply" => {
                let project = tab.project.clone();
                let has_stashes = !project.read(cx).stashes().is_empty();
                if has_stashes {
                    project.update(cx, |proj, cx| {
                        proj.stash_apply(0, cx).detach();
                    });
                } else {
                    self.show_toast("No stashes to apply", ToastKind::Warning, cx);
                }
            }
            "force_push" => {
                self.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Force Push",
                        "This will overwrite the remote branch. Are you sure?",
                        ConfirmAction::ForcePush,
                        cx,
                    );
                });
            }
            "discard_all" => {
                self.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Discard All Changes",
                        "This will permanently discard all uncommitted changes.",
                        ConfirmAction::DiscardAll,
                        cx,
                    );
                });
            }
            "create_tag" | "rename_branch" | "cherry_pick" | "revert_commit" | "delete_branch" => {
                let msg = format!(
                    "Use the sidebar context menu for '{}'",
                    cmd_id.replace('_', " ")
                );
                self.show_toast(msg, ToastKind::Info, cx);
            }
            _ => {}
        }
    }

    /// Show a temporary toast notification.
    fn show_toast(&mut self, text: impl Into<String>, kind: ToastKind, cx: &mut Context<Self>) {
        let message = text.into();
        self.toast_layer
            .update(cx, |layer, cx| layer.show_toast(message.clone(), kind, cx));
    }

    fn retry_git_operation(&mut self, update: &GitOperationUpdate, cx: &mut Context<Self>) {
        let Some(project) = self.active_project().cloned() else {
            self.show_toast("No active repository to retry.", ToastKind::Warning, cx);
            return;
        };

        let handled = project.update(cx, |proj, cx| match update.kind {
            GitOperationKind::Fetch => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.fetch(remote_name, cx).detach();
                    true
                } else {
                    proj.fetch_default(cx).detach();
                    true
                }
            }
            GitOperationKind::Pull => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.pull(remote_name, cx).detach();
                    true
                } else {
                    proj.pull_default(cx).detach();
                    true
                }
            }
            GitOperationKind::Push => {
                let force = update.summary.to_ascii_lowercase().contains("force push");
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.push(remote_name, force, cx).detach();
                    true
                } else {
                    proj.push_default(force, cx).detach();
                    true
                }
            }
            GitOperationKind::Checkout => {
                if let Some(branch_name) = update.branch_name.as_deref() {
                    proj.checkout_branch(branch_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            GitOperationKind::Merge => {
                if let Some(branch_name) = update.branch_name.as_deref() {
                    proj.merge_branch(branch_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            GitOperationKind::RemoveRemote => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.remove_remote(remote_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            _ => false,
        });

        if !handled {
            self.show_toast(
                "This operation cannot be retried automatically.",
                ToastKind::Info,
                cx,
            );
        } else {
            self.last_failed_git_operation = None;
        }
    }

    fn current_layout_settings(&self) -> LayoutSettings {
        LayoutSettings {
            sidebar_width: self.sidebar_width,
            detail_panel_width: self.detail_panel_width,
            diff_viewer_height: self.diff_viewer_height,
            commit_input_height: self.commit_input_height,
        }
    }

    fn apply_layout_settings(&mut self, layout: &LayoutSettings) {
        self.sidebar_width = layout.sidebar_width;
        self.detail_panel_width = layout.detail_panel_width;
        self.diff_viewer_height = layout.diff_viewer_height;
        self.commit_input_height = layout.commit_input_height;
    }

    fn persist_workspace_snapshot(&mut self, cx: &mut Context<Self>) {
        let repos: Vec<std::path::PathBuf> = self
            .tabs
            .iter()
            .map(|t| t.project.read(cx).repo_path().to_path_buf())
            .collect();

        if cx.try_global::<rgitui_settings::SettingsState>().is_none() {
            return;
        }

        let settings = cx.global_mut::<rgitui_settings::SettingsState>();
        for repo in &repos {
            settings.add_recent_repo(repo.clone());
        }

        if let Some(workspace_id) = settings.save_workspace_snapshot(
            self.active_workspace_id.as_deref(),
            repos,
            self.active_tab,
            self.current_layout_settings(),
        ) {
            self.active_workspace_id = Some(workspace_id);
        }

        if let Err(error) = settings.save() {
            log::error!("Failed to persist workspace snapshot: {}", error);
        }
    }

    fn clear_active_workspace_state(&mut self, cx: &mut Context<Self>) {
        self.active_workspace_id = None;
        self.status_message = None;
        self.is_loading = false;
        self.loading_message = None;
        self.active_git_operation = None;
        self.last_failed_git_operation = None;

        if cx.try_global::<rgitui_settings::SettingsState>().is_some() {
            let settings = cx.global_mut::<rgitui_settings::SettingsState>();
            settings.clear_active_workspace();
            if let Err(error) = settings.save() {
                log::error!("Failed to clear active workspace: {}", error);
            }
        }
    }

    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        self.tabs.clear();
        self.active_tab = 0;
        self.save_layout(cx);
        self.clear_active_workspace_state(cx);
        cx.notify();
    }

    pub fn restore_workspace_snapshot(
        &mut self,
        snapshot: StoredWorkspace,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        log::info!(
            "restoring workspace '{}' with {} repos",
            snapshot.name,
            snapshot.repos.len()
        );

        self.tabs.clear();
        self.active_tab = 0;
        self.active_workspace_id = Some(snapshot.id.clone());
        self.apply_layout_settings(&snapshot.layout);

        let mut opened_any = false;
        for repo_path in snapshot.repos.iter().filter(|path| path.exists()) {
            match self.open_repo(repo_path.clone(), cx) {
                Ok(()) => opened_any = true,
                Err(error) => {
                    log::error!(
                        "Failed to restore repo '{}' from workspace '{}': {}",
                        repo_path.display(),
                        snapshot.name,
                        error
                    );
                }
            }
        }

        if !opened_any {
            self.go_home(cx);
            anyhow::bail!(
                "Workspace '{}' has no available repositories",
                snapshot.name
            );
        }

        self.active_tab = snapshot
            .active_repo_index
            .min(self.tabs.len().saturating_sub(1));
        self.status_message = Some(format!("Opened workspace '{}'", snapshot.name));
        self.persist_workspace_snapshot(cx);
        cx.notify();
        Ok(())
    }

    fn restore_last_workspace(&mut self, cx: &mut Context<Self>) {
        let snapshot = cx
            .try_global::<rgitui_settings::SettingsState>()
            .and_then(|settings| settings.active_workspace().cloned());

        if let Some(snapshot) = snapshot {
            if let Err(error) = self.restore_workspace_snapshot(snapshot, cx) {
                self.show_toast(error.to_string(), ToastKind::Error, cx);
            }
        } else {
            self.show_toast("No saved workspace available.", ToastKind::Info, cx);
        }
    }

    /// Open a repository as a new tab.
    pub fn open_repo(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) -> Result<()> {
        // Check if already open
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.project.read(cx).repo_path() == path)
        {
            self.active_tab = idx;
            cx.notify();
            return Ok(());
        }

        // Validate that path is a valid git repository before creating the entity
        if !path.exists() {
            anyhow::bail!("Path does not exist: {}", path.display());
        }
        if git2::Repository::discover(&path).is_err() {
            anyhow::bail!("Not a Git repository: {}", path.display());
        }

        let project = cx.new(|cx| {
            GitProject::open(path.clone(), cx).expect("Already validated path is a git repo")
        });

        let graph = cx.new(|cx| GraphView::new(cx));
        let diff_viewer = cx.new(|_cx| DiffViewer::new());
        let detail_panel = cx.new(|_cx| DetailPanel::new());
        let sidebar = cx.new(|_cx| Sidebar::new());
        let commit_panel = cx.new(CommitPanel::new);
        let toolbar = cx.new(|_cx| Toolbar::new());

        // Set the repo name on the sidebar header
        let repo_display_name = project.read(cx).repo_name().to_string();
        sidebar.update(cx, |s, cx| {
            s.set_repo_name(repo_display_name, cx);
        });

        // Set up subscriptions

        // When project changes, update child views
        cx.subscribe(&project, {
            let graph = graph.clone();
            let sidebar = sidebar.clone();
            let commit_panel = commit_panel.clone();
            let toolbar = toolbar.clone();
            move |this, project, event: &GitProjectEvent, cx| {
                match event {
                    GitProjectEvent::StatusChanged
                    | GitProjectEvent::HeadChanged
                    | GitProjectEvent::RefsChanged => {
                        // Update graph
                        let commits = project.read(cx).recent_commits().to_vec();
                        // Resolve avatars for any new authors
                        let mut seen = std::collections::HashSet::new();
                        let authors: Vec<(String, String)> = commits
                            .iter()
                            .filter(|c| seen.insert(c.author.email.clone()))
                            .map(|c| (c.author.name.clone(), c.author.email.clone()))
                            .collect();
                        crate::avatar_resolver::resolve_avatars(authors, cx);
                        graph.update(cx, |g, cx| g.set_commits(commits, cx));

                        // Update sidebar
                        let branches = project.read(cx).branches().to_vec();
                        let tags = project.read(cx).tags().to_vec();
                        let remotes = project.read(cx).remotes().to_vec();
                        let stashes = project.read(cx).stashes().to_vec();
                        let status = project.read(cx).status().clone();

                        sidebar.update(cx, |s, cx| {
                            s.update_branches(branches, cx);
                            s.update_tags(tags, cx);
                            s.update_remotes(remotes, cx);
                            s.update_stashes(stashes, cx);
                            s.update_status(status.staged.clone(), status.unstaged.clone(), cx);
                        });

                        // Update commit panel
                        let staged_count = project.read(cx).status().staged.len();
                        commit_panel.update(cx, |cp, cx| cp.set_staged_count(staged_count, cx));

                        // Update toolbar
                        let has_stashes = !project.read(cx).stashes().is_empty();
                        let has_changes = project.read(cx).has_changes();
                        let (ahead, behind) = project
                            .read(cx)
                            .branches()
                            .iter()
                            .find(|b| b.is_head)
                            .map(|b| (b.ahead, b.behind))
                            .unwrap_or((0, 0));
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
                                this.is_loading = true;
                                this.loading_message = Some(update.summary.clone());
                                this.status_message = Some(update.summary.clone());
                                this.active_git_operation = Some(update.clone());
                                this.show_toast(update.summary.clone(), ToastKind::Info, cx);
                            }
                            GitOperationState::Succeeded => {
                                this.is_loading = false;
                                this.loading_message = None;
                                this.status_message = Some(update.summary.clone());
                                if this
                                    .active_git_operation
                                    .as_ref()
                                    .is_some_and(|op| op.id == operation_id)
                                {
                                    this.active_git_operation = None;
                                }
                                if this
                                    .last_failed_git_operation
                                    .as_ref()
                                    .is_some_and(|op| op.kind == update.kind)
                                {
                                    this.last_failed_git_operation = None;
                                }
                                this.show_toast(update.summary.clone(), ToastKind::Success, cx);
                            }
                            GitOperationState::Failed => {
                                this.is_loading = false;
                                this.loading_message = None;
                                if this
                                    .active_git_operation
                                    .as_ref()
                                    .is_some_and(|op| op.id == operation_id)
                                {
                                    this.active_git_operation = None;
                                }
                                this.last_failed_git_operation = Some(update.clone());
                                this.status_message = Some(failure_message.clone());
                                this.show_toast(failure_message, ToastKind::Error, cx);
                            }
                        }

                        toolbar.update(cx, |tb, cx| {
                            tb.set_fetching(
                                is_running && update.kind == GitOperationKind::Fetch,
                                cx,
                            );
                            tb.set_pulling(is_running && update.kind == GitOperationKind::Pull, cx);
                            tb.set_pushing(is_running && update.kind == GitOperationKind::Push, cx);
                        });
                    }
                }
            }
        })
        .detach();

        // When sidebar emits events, handle them
        cx.subscribe(&sidebar, {
            let project = project.clone();
            let diff_viewer = diff_viewer.clone();
            let detail_panel_ref = detail_panel.clone();
            move |this, _sidebar, event: &SidebarEvent, cx| {
                match event {
                    SidebarEvent::FileSelected { path, staged } => {
                        let path_buf = std::path::PathBuf::from(path);
                        let is_staged = *staged;
                        let proj = project.read(cx);
                        match proj.diff_file(&path_buf, is_staged) {
                            Ok(diff) => {
                                let p = path.clone();
                                diff_viewer.update(cx, |dv, cx| {
                                    dv.set_diff(diff, p, is_staged, cx);
                                });
                                // Clear detail panel when viewing working tree files
                                detail_panel_ref.update(cx, |dp, cx| dp.clear(cx));
                            }
                            Err(e) => {
                                log::error!("Failed to get diff for {}: {}", path, e);
                            }
                        }
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
                        this.confirm_dialog.update(cx, |cd, cx| {
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
                        this.confirm_dialog.update(cx, |cd, cx| {
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
                        let proj = project.read(cx);
                        match proj.diff_stash(idx) {
                            Ok(commit_diff) => {
                                if let Some(first_file) = commit_diff.files.first() {
                                    let path = first_file.path.display().to_string();
                                    diff_viewer.update(cx, |dv, cx| {
                                        dv.set_diff(first_file.clone(), path, false, cx);
                                    });
                                }
                                detail_panel_ref.update(cx, |dp, cx| dp.clear(cx));
                            }
                            Err(e) => {
                                log::error!("Failed to get stash diff: {}", e);
                            }
                        }
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
                        this.branch_dialog.update(cx, |bd, cx| {
                            bd.show_visible(None, cx);
                        });
                    }
                    SidebarEvent::BranchDelete(name) => {
                        let name = name.clone();
                        this.confirm_dialog.update(cx, |cd, cx| {
                            cd.show_visible(
                                "Delete Branch",
                                format!("Are you sure you want to delete branch '{}'?", name),
                                ConfirmAction::BranchDelete(name),
                                cx,
                            );
                        });
                    }
                    SidebarEvent::OpenRepo => {
                        this.repo_opener.update(cx, |ro, cx| {
                            ro.toggle_visible(cx);
                        });
                    }
                }
            }
        })
        .detach();

        // When graph emits events, handle commit selection
        cx.subscribe(&graph, {
            let project = project.clone();
            let diff_viewer = diff_viewer.clone();
            let detail_panel_ref = detail_panel.clone();
            move |this, _graph, event: &GraphViewEvent, cx| {
                match event {
                    GraphViewEvent::CommitSelected(oid) => {
                        let proj = project.read(cx);

                        // Find commit info for the detail panel
                        let commit_info = proj
                            .recent_commits()
                            .iter()
                            .find(|c| c.oid == *oid)
                            .cloned();

                        match proj.diff_commit(*oid) {
                            Ok(commit_diff) => {
                                // Update detail panel with commit metadata
                                if let Some(info) = commit_info {
                                    detail_panel_ref.update(cx, |dp, cx| {
                                        dp.set_commit(info, commit_diff.clone(), cx);
                                    });
                                }

                                // Show the first file's diff if any
                                if let Some(first_file) = commit_diff.files.first() {
                                    let path = first_file.path.display().to_string();
                                    diff_viewer.update(cx, |dv, cx| {
                                        dv.set_diff(first_file.clone(), path, false, cx);
                                    });
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to get commit diff: {}", e);
                            }
                        }
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
                        this.branch_dialog.update(cx, |bd, cx| {
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
                    }
                }
            }
        })
        .detach();

        // When detail panel emits events (file selection, copy sha, cherry-pick), handle them
        cx.subscribe(&detail_panel, {
            let diff_viewer = diff_viewer.clone();
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

        // When diff viewer emits events, handle hunk staging
        cx.subscribe(&diff_viewer, {
            let project = project.clone();
            let diff_viewer_ref = diff_viewer.clone();
            move |_this, _dv, event: &DiffViewerEvent, cx| {
                let dv = diff_viewer_ref.read(cx);
                let file_path = dv.file_path().map(|p| std::path::PathBuf::from(p));
                let _is_staged = dv.is_staged();
                let _ = dv;

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

        // When commit panel emits events, handle commit/AI
        cx.subscribe(&commit_panel, {
            let project = project.clone();
            let ai = self.ai.clone();
            let commit_panel_ref = commit_panel.clone();
            move |_this, _cp, event: &CommitPanelEvent, cx| match event {
                CommitPanelEvent::CommitRequested { message, amend } => {
                    let msg = message.clone();
                    let amend = *amend;
                    project.update(cx, |proj, cx| {
                        proj.commit(&msg, amend, cx).detach();
                    });
                    commit_panel_ref.update(cx, |cp, cx| {
                        cp.set_message(String::new(), cx);
                    });
                }
                CommitPanelEvent::GenerateAiMessage => {
                    commit_panel_ref.update(cx, |cp, cx| {
                        cp.set_ai_generating(true, cx);
                    });

                    let proj = project.read(cx);
                    let diff_text = proj.staged_diff_text().unwrap_or_default();
                    let summary = proj.staged_summary();

                    let ai_entity = ai.clone();
                    ai_entity.update(cx, |ai_gen, cx| {
                        ai_gen
                            .generate_commit_message(diff_text, summary, cx)
                            .detach();
                    });
                }
            }
        })
        .detach();

        // When toolbar emits events, handle fetch/pull/push/stash
        cx.subscribe(&toolbar, {
            let project = project.clone();
            move |this, _toolbar, event: &ToolbarEvent, cx| {
                match event {
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
                        this.branch_dialog.update(cx, |bd, cx| {
                            bd.show_visible(None, cx);
                        });
                    }
                    ToolbarEvent::Refresh => {
                        project.update(cx, |proj, cx| {
                            proj.refresh(cx).detach();
                        });
                    }
                    ToolbarEvent::Settings => {
                        this.settings_modal.update(cx, |sm, cx| {
                            sm.toggle_visible(cx);
                        });
                    }
                    ToolbarEvent::Search => {
                        // Toggle graph search if we have an active tab
                        if let Some(tab) = this.tabs.get(this.active_tab) {
                            tab.graph.update(cx, |g, cx| {
                                g.toggle_search(cx);
                            });
                        }
                    }
                    ToolbarEvent::Undo | ToolbarEvent::Redo => {
                        // Not yet implemented
                    }
                }
            }
        })
        .detach();

        // Initial sync
        {
            let commits = project.read(cx).recent_commits().to_vec();
            // Resolve avatars for unique authors
            let mut seen = std::collections::HashSet::new();
            let authors: Vec<(String, String)> = commits
                .iter()
                .filter(|c| seen.insert(c.author.email.clone()))
                .map(|c| (c.author.name.clone(), c.author.email.clone()))
                .collect();
            crate::avatar_resolver::resolve_avatars(authors, cx);
            graph.update(cx, |g, cx| g.set_commits(commits, cx));

            let branches = project.read(cx).branches().to_vec();
            let tags = project.read(cx).tags().to_vec();
            let remotes = project.read(cx).remotes().to_vec();
            let stashes = project.read(cx).stashes().to_vec();
            let status = project.read(cx).status().clone();

            sidebar.update(cx, |s, cx| {
                s.update_branches(branches, cx);
                s.update_tags(tags, cx);
                s.update_remotes(remotes, cx);
                s.update_stashes(stashes, cx);
                s.update_status(status.staged.clone(), status.unstaged.clone(), cx);
            });

            let staged_count = project.read(cx).status().staged.len();
            commit_panel.update(cx, |cp, cx| cp.set_staged_count(staged_count, cx));
        }

        let name = project.read(cx).repo_name().to_string();
        self.tabs.push(ProjectTab {
            name,
            project,
            graph,
            diff_viewer,
            detail_panel,
            sidebar,
            commit_panel,
            toolbar,
        });
        self.active_tab = self.tabs.len() - 1;
        self.persist_workspace_snapshot(cx);

        cx.notify();
        Ok(())
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab = self.tabs.len() - 1;
            }
            self.save_workspace_state(cx);
            cx.notify();
        }
    }

    /// Persist the current set of open repo paths and layout to settings.
    fn save_workspace_state(&mut self, cx: &mut Context<Self>) {
        self.save_layout(cx);
        if self.tabs.is_empty() {
            self.clear_active_workspace_state(cx);
        } else {
            self.persist_workspace_snapshot(cx);
        }
    }

    /// Schedule a debounced layout save (avoids writing to disk on every resize pixel).
    fn schedule_layout_save(&self, cx: &mut Context<Self>) {
        let sw = self.sidebar_width;
        let dpw = self.detail_panel_width;
        let dvh = self.diff_viewer_height;
        let cih = self.commit_input_height;
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                this.update(cx, |this, cx| {
                    // Only save if dimensions haven't changed (i.e., drag stopped)
                    if (this.sidebar_width - sw).abs() < 0.1
                        && (this.detail_panel_width - dpw).abs() < 0.1
                        && (this.diff_viewer_height - dvh).abs() < 0.1
                        && (this.commit_input_height - cih).abs() < 0.1
                    {
                        this.save_layout(cx);
                    }
                })
                .ok();
            },
        )
        .detach();
    }

    /// Persist current layout dimensions to settings.
    fn save_layout(&self, cx: &mut Context<Self>) {
        if cx.try_global::<rgitui_settings::SettingsState>().is_some() {
            let settings = cx.global_mut::<rgitui_settings::SettingsState>();
            settings.settings_mut().layout.sidebar_width = self.sidebar_width;
            settings.settings_mut().layout.detail_panel_width = self.detail_panel_width;
            settings.settings_mut().layout.diff_viewer_height = self.diff_viewer_height;
            settings.settings_mut().layout.commit_input_height = self.commit_input_height;
            if let Err(e) = settings.save() {
                log::error!("Failed to save layout: {}", e);
            }
        }
    }

    pub fn active_project(&self) -> Option<&Entity<GitProject>> {
        self.tabs.get(self.active_tab).map(|t| &t.project)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let modifiers = &keystroke.modifiers;

        // Dismiss settings modal on Escape
        if key == "escape" && self.settings_modal.read(cx).is_visible() {
            self.settings_modal.update(cx, |sm, cx| {
                sm.dismiss(cx);
            });
            return;
        }

        // Dismiss interactive rebase dialog on Escape
        if key == "escape" && self.interactive_rebase.read(cx).is_visible() {
            self.interactive_rebase.update(cx, |ir, cx| {
                ir.dismiss(cx);
            });
            return;
        }

        // Dismiss confirm dialog on Escape
        if key == "escape" && self.confirm_dialog.read(cx).is_visible() {
            self.confirm_dialog.update(cx, |cd, cx| {
                cd.cancel(cx);
            });
            return;
        }

        // Dismiss branch dialog on Escape
        if key == "escape" && self.branch_dialog.read(cx).is_visible() {
            self.branch_dialog.update(cx, |bd, cx| {
                bd.dismiss(cx);
            });
            return;
        }

        // Dismiss repo opener on Escape
        if key == "escape" && self.repo_opener.read(cx).is_visible() {
            self.repo_opener.update(cx, |ro, cx| {
                ro.dismiss(cx);
            });
            return;
        }

        // Dismiss shortcuts help on Escape
        if key == "escape" && self.shortcuts_help.read(cx).is_visible() {
            self.shortcuts_help.update(cx, |sh, cx| {
                sh.dismiss(cx);
            });
            return;
        }

        // Ctrl+F to toggle graph search
        if (modifiers.control || modifiers.platform) && !modifiers.shift && key == "f" {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                graph.update(cx, |g, cx| {
                    g.toggle_search_focused(window, cx);
                });
            }
            return;
        }

        // Ctrl+Shift+P or Cmd+Shift+P to open command palette
        if (modifiers.control || modifiers.platform) && modifiers.shift && key == "p" {
            self.command_palette.update(cx, |cp, cx| {
                cp.toggle(window, cx);
            });
        }

        // Ctrl+, to open settings
        if (modifiers.control || modifiers.platform) && key == "," {
            self.settings_modal.update(cx, |sm, cx| {
                sm.toggle(window, cx);
            });
            return;
        }

        // F5 to refresh
        if key == "f5" {
            self.execute_command("refresh", cx);
        }

        // Ctrl+O to open repo opener
        if (modifiers.control || modifiers.platform) && key == "o" {
            self.repo_opener.update(cx, |ro, cx| {
                ro.toggle(window, cx);
            });
            return;
        }

        // ? to toggle shortcuts help (without modifiers)
        if key == "?" && !modifiers.control && !modifiers.platform && !modifiers.alt {
            self.shortcuts_help.update(cx, |sh, cx| {
                sh.toggle(window, cx);
            });
            return;
        }

        // j/k vim-style navigation in the commit graph
        if !modifiers.control && !modifiers.alt && !modifiers.shift && !modifiers.platform {
            match key {
                "j" => {
                    if let Some(tab) = self.tabs.get(self.active_tab) {
                        let graph = tab.graph.clone();
                        graph.update(cx, |g, cx| {
                            let next = g
                                .selected_index()
                                .map(|i| (i + 1).min(g.commit_count().saturating_sub(1)))
                                .unwrap_or(0);
                            g.select_index(next, cx);
                        });
                    }
                }
                "k" => {
                    if let Some(tab) = self.tabs.get(self.active_tab) {
                        let graph = tab.graph.clone();
                        graph.update(cx, |g, cx| {
                            if let Some(i) = g.selected_index() {
                                if i > 0 {
                                    g.select_index(i - 1, cx);
                                }
                            }
                        });
                    }
                }
                _ => {}
            }
        }

        // Ctrl+[ / Ctrl+] to resize detail panel width
        if modifiers.control && !modifiers.shift && !modifiers.alt {
            match key {
                "[" | "bracketleft" => {
                    self.detail_panel_width = (self.detail_panel_width - 20.0).max(180.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "]" | "bracketright" => {
                    self.detail_panel_width = (self.detail_panel_width + 20.0).min(480.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                // Ctrl+Up / Ctrl+Down to resize diff viewer height
                "up" => {
                    self.diff_viewer_height = (self.diff_viewer_height - 30.0).max(100.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "down" => {
                    self.diff_viewer_height = (self.diff_viewer_height + 30.0).min(600.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                _ => {}
            }
        }

        // Ctrl+S to stage all
        if modifiers.control && !modifiers.shift && key == "s" {
            self.execute_command("stage_all", cx);
            return;
        }

        // Ctrl+Shift+S to unstage all
        if modifiers.control && modifiers.shift && key == "s" {
            self.execute_command("unstage_all", cx);
            return;
        }

        // Ctrl+B to create branch
        if modifiers.control && !modifiers.shift && key == "b" {
            self.execute_command("create_branch", cx);
            return;
        }

        // Ctrl+Enter to commit
        if modifiers.control && !modifiers.shift && key == "enter" {
            self.execute_command("commit", cx);
            return;
        }

        // Ctrl+Z to stash save
        if modifiers.control && !modifiers.shift && key == "z" {
            self.execute_command("stash_save", cx);
            return;
        }

        // Ctrl+Shift+Z to stash pop
        if modifiers.control && modifiers.shift && key == "z" {
            self.execute_command("stash_pop", cx);
            return;
        }

        // Ctrl+Tab to switch to next tab
        if modifiers.control && !modifiers.shift && key == "tab" {
            if !self.tabs.is_empty() {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                cx.notify();
            }
            return;
        }

        // Ctrl+Shift+Tab to switch to previous tab
        if modifiers.control && modifiers.shift && key == "tab" {
            if !self.tabs.is_empty() {
                if self.active_tab == 0 {
                    self.active_tab = self.tabs.len() - 1;
                } else {
                    self.active_tab -= 1;
                }
                cx.notify();
            }
            return;
        }

        // Ctrl+W to close current tab
        if modifiers.control && !modifiers.shift && key == "w" {
            if !self.tabs.is_empty() {
                self.close_tab(self.active_tab, cx);
            }
            return;
        }

        // Ctrl+Shift+W to return to workspace home
        if modifiers.control && modifiers.shift && key == "w" {
            self.go_home(cx);
            return;
        }

        // (? shortcut is already handled above via shortcuts_help toggle)
    }

    fn render_welcome_interactive(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let recent_workspaces = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|settings| settings.recent_workspaces(6))
            .unwrap_or_default();
        let recent_repos = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|settings| settings.settings().recent_repos.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|path| path.exists())
            .take(6)
            .collect::<Vec<_>>();

        let mut content = div()
            .v_flex()
            .gap(px(18.))
            .items_center()
            .max_w(px(620.))
            .w_full()
            // Logo area
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(64.))
                    .h(px(64.))
                    .rounded(px(16.))
                    .bg(colors.element_background)
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::Large)
                            .color(Color::Accent),
                    ),
            )
            .child(
                Label::new("rgitui")
                    .size(LabelSize::Large)
                    .weight(gpui::FontWeight::BOLD),
            )
            .child(
                Label::new("A workspace-oriented desktop Git client")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .mt(px(4.))
                    .child(
                        Button::new("workspace-home-open-repo", "Open Repository")
                            .style(ButtonStyle::Filled)
                            .icon(IconName::Folder)
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.repo_opener.update(cx, |opener, cx| {
                                    opener.toggle_visible(cx);
                                });
                            })),
                    )
                    .child(
                        Button::new("workspace-home-new", "New Workspace")
                            .style(ButtonStyle::Outlined)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.go_home(cx);
                                this.repo_opener.update(cx, |opener, cx| {
                                    opener.toggle_visible(cx);
                                });
                            })),
                    )
                    .when(!recent_workspaces.is_empty(), |buttons| {
                        buttons.child(
                            Button::new("workspace-home-restore", "Restore Last")
                                .style(ButtonStyle::Subtle)
                                .icon(IconName::Clock)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.restore_last_workspace(cx);
                                })),
                        )
                    }),
            );

        if !recent_workspaces.is_empty() {
            let mut workspaces_list = div().v_flex().w_full().mt(px(8.)).gap(px(6.)).child(
                Label::new("Recent Workspaces")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

            for (i, workspace) in recent_workspaces.iter().enumerate() {
                let workspace_id = workspace.id.clone();
                let workspace_name: SharedString = workspace.name.clone().into();
                let summary: SharedString = format!(
                    "{} repositories · updated {}",
                    workspace.repos.len(),
                    workspace
                        .last_opened_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                )
                .into();
                let repo_preview: SharedString = workspace
                    .repos
                    .iter()
                    .take(2)
                    .map(|repo| {
                        repo.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| repo.display().to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
                    .into();

                workspaces_list = workspaces_list.child(
                    div()
                        .id(ElementId::NamedInteger("recent-workspace".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .min_h(px(64.))
                        .px_3()
                        .py_2()
                        .gap_3()
                        .items_start()
                        .rounded(px(8.))
                        .cursor_pointer()
                        .bg(colors.ghost_element_background)
                        .border_1()
                        .border_color(colors.border_variant)
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                            let snapshot = cx
                                .try_global::<rgitui_settings::SettingsState>()
                                .and_then(|settings| settings.workspace(&workspace_id).cloned());
                            if let Some(snapshot) = snapshot {
                                if let Err(error) = this.restore_workspace_snapshot(snapshot, cx) {
                                    this.show_toast(error.to_string(), ToastKind::Error, cx);
                                }
                            }
                        }))
                        .child(
                            Icon::new(IconName::Stash)
                                .size(IconSize::Medium)
                                .color(Color::Accent),
                        )
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Label::new(workspace_name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM),
                                )
                                .child(
                                    Label::new(summary)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(repo_preview)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        ),
                );
            }

            content = content.child(workspaces_list);
        }

        if !recent_repos.is_empty() {
            let mut repos_list = div().v_flex().w_full().gap(px(4.)).child(
                Label::new("Recent Repositories")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

            for (i, repo_path) in recent_repos.iter().enumerate() {
                let repo_name: SharedString = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| repo_path.display().to_string())
                    .into();
                let repo_dir: SharedString = repo_path.display().to_string().into();
                let path = repo_path.clone();

                repos_list = repos_list.child(
                    div()
                        .id(ElementId::NamedInteger("recent-repo".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(40.))
                        .px_3()
                        .gap_2()
                        .items_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                            if let Err(error) = this.open_repo(path.clone(), cx) {
                                this.show_toast(error.to_string(), ToastKind::Error, cx);
                            }
                        }))
                        .child(
                            Icon::new(IconName::Folder)
                                .size(IconSize::Small)
                                .color(Color::Accent),
                        )
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Label::new(repo_name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM),
                                )
                                .child(
                                    Label::new(repo_dir)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        ),
                );
            }

            content = content.child(repos_list);
        }

        // Keyboard shortcut hints
        content = content.child(
            div()
                .v_flex()
                .gap(px(8.))
                .mt(px(12.))
                .w_full()
                .items_center()
                .child(self.shortcut_hint("Open Repository", "Ctrl+O", &colors))
                .child(self.shortcut_hint("Go Home", "Ctrl+Shift+W", &colors))
                .child(self.shortcut_hint("Command Palette", "Ctrl+Shift+P", &colors))
                .child(self.shortcut_hint("Settings", "Ctrl+,", &colors)),
        );

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.background)
            .child(content)
    }

    fn shortcut_hint(
        &self,
        action: &str,
        shortcut: &str,
        colors: &rgitui_theme::ThemeColors,
    ) -> impl IntoElement {
        div()
            .h_flex()
            .w(px(260.))
            .justify_between()
            .items_center()
            .child(
                Label::new(SharedString::from(action.to_string()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .h_flex()
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .bg(colors.element_background)
                    .items_center()
                    .child(
                        Label::new(SharedString::from(shortcut.to_string()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        // If no tabs, show welcome screen
        if self.tabs.is_empty() {
            return div()
                .id("workspace-root")
                .size_full()
                .bg(colors.background)
                .on_key_down(cx.listener(Self::handle_key_down))
                .child(self.render_welcome_interactive(cx))
                .child(self.toast_layer.clone())
                .child(self.command_palette.clone())
                .child(self.interactive_rebase.clone())
                .child(self.settings_modal.clone())
                .child(self.branch_dialog.clone())
                .child(self.repo_opener.clone())
                .child(self.shortcuts_help.clone())
                .into_any_element();
        }

        let active_tab = &self.tabs[self.active_tab];
        let project = active_tab.project.read(cx);
        let repo_name: SharedString = project.repo_name().to_string().into();
        let branch_name: SharedString = project
            .head_branch()
            .unwrap_or("detached")
            .to_string()
            .into();
        let has_changes = project.has_changes();
        let staged_count = project.status().staged.len();
        let unstaged_count = project.status().unstaged.len();
        let overlays_active = self.command_palette.read(cx).is_visible()
            || self.interactive_rebase.read(cx).is_visible()
            || self.settings_modal.read(cx).is_visible()
            || self.branch_dialog.read(cx).is_visible()
            || self.repo_opener.read(cx).is_visible()
            || self.confirm_dialog.read(cx).is_visible()
            || self.shortcuts_help.read(cx).is_visible();

        // Find head branch info for ahead/behind
        let (ahead, behind) = project
            .branches()
            .iter()
            .find(|b| b.is_head)
            .map(|b| (b.ahead, b.behind))
            .unwrap_or((0, 0));

        // Build tab bar
        let mut tab_bar = TabBar::new();
        let workspace_handle = cx.entity().downgrade();
        for (i, tab) in self.tabs.iter().enumerate() {
            let tab_name: SharedString = tab.name.clone().into();
            let ws = workspace_handle.clone();
            let ws_close = workspace_handle.clone();
            tab_bar = tab_bar.tab(
                Tab::new(
                    ElementId::NamedInteger("project-tab".into(), i as u64),
                    tab_name,
                )
                .active(i == self.active_tab)
                .closeable(true)
                .on_click(move |_event, _window, cx| {
                    ws.update(cx, |ws, cx| {
                        ws.active_tab = i;
                        cx.notify();
                    })
                    .ok();
                })
                .on_close(move |_event, _window, cx| {
                    ws_close
                        .update(cx, |ws, cx| {
                            ws.close_tab(i, cx);
                        })
                        .ok();
                }),
            );
        }

        // Add workspace and repo actions to tab bar
        let ws_home = workspace_handle.clone();
        let ws_open = workspace_handle.clone();
        tab_bar = tab_bar.end_slot(
            div()
                .h_flex()
                .gap_1()
                .child(
                    IconButton::new("tab-bar-home", IconName::Folder)
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .on_click(move |_: &gpui::ClickEvent, _, cx| {
                            ws_home
                                .update(cx, |ws, cx| {
                                    ws.go_home(cx);
                                })
                                .ok();
                        }),
                )
                .child(
                    IconButton::new("tab-bar-add", IconName::Plus)
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .on_click(move |_: &gpui::ClickEvent, _, cx| {
                            ws_open
                                .update(cx, |ws, cx| {
                                    ws.repo_opener.update(cx, |ro, cx| {
                                        ro.toggle_visible(cx);
                                    });
                                })
                                .ok();
                        }),
                ),
        );

        // Status bar with operation message
        let status_bar = if let Some(msg) = &self.status_message {
            StatusBar::new()
                .branch(branch_name.clone())
                .ahead_behind(ahead, behind)
                .changes(staged_count, unstaged_count)
                .operation_message(msg.clone())
        } else {
            StatusBar::new()
                .branch(branch_name.clone())
                .ahead_behind(ahead, behind)
                .changes(staged_count, unstaged_count)
        };

        let operation_banner = if let Some(update) = self
            .active_git_operation
            .clone()
            .or_else(|| self.last_failed_git_operation.clone())
        {
            let is_failure = update.state == GitOperationState::Failed;
            let accent = if is_failure {
                cx.status().error
            } else {
                cx.status().info
            };
            let bg = if is_failure {
                cx.status().error_background
            } else {
                cx.status().info_background
            };
            let icon = if is_failure {
                IconName::FileConflict
            } else {
                IconName::Refresh
            };
            let details = update.details.clone();

            Some(
                div()
                    .h_flex()
                    .w_full()
                    .min_h(px(34.))
                    .px(px(12.))
                    .py(px(6.))
                    .gap(px(8.))
                    .items_center()
                    .bg(bg)
                    .border_b_1()
                    .border_color(accent)
                    .child(Icon::new(icon).size(IconSize::Small).color(if is_failure {
                        Color::Error
                    } else {
                        Color::Info
                    }))
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                Label::new(SharedString::from(update.summary.clone()))
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate(),
                            )
                            .when_some(details, |el, details| {
                                el.child(
                                    Label::new(SharedString::from(details))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                )
                            }),
                    )
                    .when(is_failure && update.retryable, |el| {
                        let retry_update = update.clone();
                        el.child(
                            Button::new("operation-retry", "Retry")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Filled)
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                    this.retry_git_operation(&retry_update, cx);
                                })),
                        )
                    })
                    .when(is_failure, |el| {
                        el.child(
                            IconButton::new("operation-dismiss", IconName::X)
                                .size(ButtonSize::Compact)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.last_failed_git_operation = None;
                                    cx.notify();
                                })),
                        )
                    }),
            )
        } else {
            None
        };

        div()
            .id("workspace-root")
            .v_flex()
            .size_full()
            .bg(colors.background)
            .on_key_down(cx.listener(Self::handle_key_down))
            // Title bar
            .child(TitleBar::new(repo_name.clone(), branch_name.clone()).has_changes(has_changes))
            // Toolbar
            .child(active_tab.toolbar.clone())
            .when_some(operation_banner, |el, banner| el.child(banner))
            // Tab bar
            .child(tab_bar)
            // Main content area — drag_move listeners live here so they fire globally
            .child({
                let entity = cx.entity();
                div()
                    .id("main-content")
                    .h_flex()
                    .flex_1()
                    .min_h_0()
                    // Capture content area bounds each frame for use in resize calculations
                    .child(
                        canvas(
                            {
                                let entity = entity.clone();
                                move |bounds, _, cx| {
                                    entity.update(cx, |this, _| this.content_bounds = bounds);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    // Global resize drag listeners
                    .on_drag_move::<SidebarResize>(cx.listener(
                        |this, e: &DragMoveEvent<SidebarResize>, _, cx| {
                            let new_w = f32::from(e.event.position.x - this.content_bounds.left())
                                .clamp(120., 600.);
                            this.sidebar_width = new_w;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DetailPanelResize>(cx.listener(
                        |this, e: &DragMoveEvent<DetailPanelResize>, _, cx| {
                            let new_w = f32::from(this.content_bounds.right() - e.event.position.x)
                                .clamp(180., 600.);
                            this.detail_panel_width = new_w;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DiffViewerResize>(cx.listener(
                        |this, e: &DragMoveEvent<DiffViewerResize>, _, cx| {
                            let new_h =
                                f32::from(this.content_bounds.bottom() - e.event.position.y)
                                    .clamp(60., 500.);
                            this.diff_viewer_height = new_h;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<CommitInputResize>(cx.listener(
                        |this, e: &DragMoveEvent<CommitInputResize>, _, cx| {
                            let new_h =
                                f32::from(this.right_panel_bounds.bottom() - e.event.position.y)
                                    .clamp(100., 400.);
                            this.commit_input_height = new_h;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    // Left sidebar — branches
                    .child(
                        div()
                            .relative()
                            .w(px(self.sidebar_width))
                            .h_full()
                            .flex_shrink_0()
                            .child(active_tab.sidebar.clone())
                            // Resize handle straddles the right border
                            .child(
                                div()
                                    .id("sidebar-resize-handle")
                                    .absolute()
                                    .top_0()
                                    .right(px(-4.))
                                    .h_full()
                                    .w(px(6.))
                                    .when(!overlays_active, |el| {
                                        el.cursor_col_resize()
                                            .hover(|s| s.bg(colors.border_focused))
                                            .on_drag(SidebarResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            ),
                    )
                    // Center: loading indicator + graph (flex) + resize strip + diff viewer (fixed height)
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            // Loading indicator
                            .when(self.is_loading, |el| {
                                let msg: SharedString =
                                    self.loading_message.clone().unwrap_or_default().into();
                                el.child(
                                    div()
                                        .h_flex()
                                        .w_full()
                                        .h(px(28.))
                                        .px_3()
                                        .items_center()
                                        .gap_2()
                                        .bg(colors.surface_background)
                                        .border_b_1()
                                        .border_color(colors.border_variant)
                                        .child(
                                            // Pulsing dot indicator
                                            div()
                                                .w(px(8.))
                                                .h(px(8.))
                                                .rounded_full()
                                                .bg(colors.border_focused),
                                        )
                                        .child(
                                            Label::new(msg)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Accent),
                                        ),
                                )
                            })
                            // Graph view
                            .child(div().flex_1().min_h_0().child(active_tab.graph.clone()))
                            // Drag-to-resize strip between graph and diff viewer
                            .child(
                                div()
                                    .id("diff-resize-handle")
                                    .w_full()
                                    .h(px(2.))
                                    .flex_shrink_0()
                                    .border_t_1()
                                    .border_color(colors.border_variant)
                                    .when(!overlays_active, |el| {
                                        el.cursor_row_resize()
                                            .hover(|s| s.bg(colors.border_focused))
                                            .on_drag(DiffViewerResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                            // Diff viewer
                            .child(
                                div()
                                    .h(px(self.diff_viewer_height))
                                    .flex_shrink_0()
                                    .child(active_tab.diff_viewer.clone()),
                            ),
                    )
                    // Right panel: detail + resize handle + commit input
                    .child({
                        let commit_input_height = self.commit_input_height;
                        div()
                            .relative()
                            .w(px(self.detail_panel_width))
                            .h_full()
                            .flex_shrink_0()
                            .v_flex()
                            .border_l_1()
                            .border_color(colors.border_variant)
                            // Bounds tracking canvas for commit input resize
                            .child(
                                canvas(
                                    {
                                        let entity = entity.clone();
                                        move |bounds, _, cx| {
                                            entity.update(cx, |this, _| {
                                                this.right_panel_bounds = bounds
                                            });
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            // Detail panel (commit info + file list) — takes remaining space
                            .child(
                                div()
                                    .id("detail-panel-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(active_tab.detail_panel.clone()),
                            )
                            // Resize handle between detail and commit input
                            .child(
                                div()
                                    .id("commit-input-resize-handle")
                                    .w_full()
                                    .h(px(2.))
                                    .flex_shrink_0()
                                    .border_t_1()
                                    .border_color(colors.border_variant)
                                    .when(!overlays_active, |el| {
                                        el.cursor_row_resize()
                                            .hover(|s| s.bg(colors.border_focused))
                                            .on_drag(CommitInputResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                            // Commit panel at bottom
                            .child(
                                div()
                                    .h(px(commit_input_height))
                                    .flex_shrink_0()
                                    .child(active_tab.commit_panel.clone()),
                            )
                            // Width resize handle on left edge
                            .child(
                                div()
                                    .id("detail-panel-resize-handle")
                                    .absolute()
                                    .top_0()
                                    .left(px(-3.))
                                    .h_full()
                                    .w(px(6.))
                                    .when(!overlays_active, |el| {
                                        el.cursor_col_resize()
                                            .hover(|s| s.bg(colors.border_focused))
                                            .on_drag(DetailPanelResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                    })
            })
            // Status bar
            .child(status_bar)
            .child(self.toast_layer.clone())
            // Command palette overlay (rendered last to be on top)
            .child(self.command_palette.clone())
            // Interactive rebase dialog overlay
            .child(self.interactive_rebase.clone())
            // Settings modal overlay
            .child(self.settings_modal.clone())
            // Branch dialog overlay
            .child(self.branch_dialog.clone())
            // Repo opener overlay
            .child(self.repo_opener.clone())
            // Confirm dialog overlay
            .child(self.confirm_dialog.clone())
            // Shortcuts help overlay
            .child(self.shortcuts_help.clone())
            .into_any_element()
    }
}
