use anyhow::Result;
use gpui::prelude::*;
use gpui::{
    div, px, App, Bounds, ClickEvent, Context, DragMoveEvent, ElementId, Entity, EventEmitter,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render, SharedString, Window, canvas,
};
use rgitui_ai::{AiEvent, AiGenerator};
use rgitui_diff::{DiffViewer, DiffViewerEvent};
use rgitui_git::{GitProject, GitProjectEvent};
use rgitui_graph::{GraphView, GraphViewEvent};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, Label, LabelSize, Tab, TabBar};

use crate::{
    CommandPalette, CommandPaletteEvent, CommitPanel, CommitPanelEvent, DetailPanel,
    DetailPanelEvent, Sidebar, SidebarEvent, StatusBar, TitleBar, Toolbar, ToolbarEvent,
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
    status_message: Option<String>,
    sidebar_width: f32,
    detail_panel_width: f32,
    diff_viewer_height: f32,
    commit_input_height: f32,
    show_branch_creation: bool,
    content_bounds: Bounds<Pixels>,
    right_panel_bounds: Bounds<Pixels>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ai = cx.new(|_cx| AiGenerator::new());
        let command_palette = cx.new(|cx| CommandPalette::new(cx));

        // Subscribe to AI events to update commit panel when generation completes
        cx.subscribe(&ai, |this, _ai, event: &AiEvent, cx| {
            match event {
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
                    this.status_message = Some(format!("AI error: {}", err));
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        tab.commit_panel.update(cx, |cp, cx| {
                            cp.set_ai_generating(false, cx);
                        });
                    }
                    cx.notify();
                }
                AiEvent::GenerationStarted => {
                    this.status_message = Some("Generating AI commit message...".into());
                    cx.notify();
                }
            }
        })
        .detach();

        // Subscribe to command palette events
        cx.subscribe(&command_palette, |this, _cp, event: &CommandPaletteEvent, cx| {
            match event {
                CommandPaletteEvent::CommandSelected(cmd_id) => {
                    this.execute_command(cmd_id, cx);
                }
                CommandPaletteEvent::Dismissed => {}
            }
        })
        .detach();

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            ai,
            command_palette,
            status_message: None,
            sidebar_width: 240.0,
            detail_panel_width: 320.0,
            diff_viewer_height: 300.0,
            commit_input_height: 200.0,
            show_branch_creation: false,
            content_bounds: Bounds::default(),
            right_panel_bounds: Bounds::default(),
        }
    }

    /// Execute a command by ID (from command palette or keybindings).
    fn execute_command(&mut self, cmd_id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };

        match cmd_id {
            "fetch" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.fetch("origin", cx).detach();
                });
            }
            "pull" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.pull("origin", cx).detach();
                });
            }
            "push" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.push("origin", false, cx).detach();
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
            "refresh" => {
                let project = tab.project.clone();
                project.update(cx, |proj, cx| {
                    proj.refresh(cx).detach();
                });
            }
            _ => {}
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

        let project = cx.new(|cx| {
            GitProject::open(path.clone(), cx)
                .unwrap_or_else(|e| panic!("Failed to open repo at {}: {}", path.display(), e))
        });

        let graph = cx.new(|_cx| GraphView::new());
        let diff_viewer = cx.new(|_cx| DiffViewer::new());
        let detail_panel = cx.new(|_cx| DetailPanel::new());
        let sidebar = cx.new(|_cx| Sidebar::new());
        let commit_panel = cx.new(|cx| CommitPanel::new(cx));
        let toolbar = cx.new(|_cx| Toolbar::new());

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
                        commit_panel
                            .update(cx, |cp, cx| cp.set_staged_count(staged_count, cx));

                        // Update toolbar
                        let has_stashes = !project.read(cx).stashes().is_empty();
                        let has_changes = project.read(cx).has_changes();
                        toolbar.update(cx, |tb, cx| {
                            tb.set_state(true, true, has_stashes, has_changes, cx);
                        });
                    }
                    GitProjectEvent::OperationStarted(msg) => {
                        this.status_message = Some(msg.clone());
                        cx.notify();
                    }
                    GitProjectEvent::OperationCompleted(msg) => {
                        this.status_message = Some(msg.clone());
                        cx.notify();
                    }
                    GitProjectEvent::OperationFailed(op, err) => {
                        this.status_message = Some(format!("{} failed: {}", op, err));
                        cx.notify();
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
            move |_this, _sidebar, event: &SidebarEvent, cx| {
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
                    SidebarEvent::DiscardFile(path) => {
                        let path_buf = std::path::PathBuf::from(path);
                        project.update(cx, |proj, cx| {
                            proj.discard_changes(&[path_buf], cx).detach();
                        });
                    }
                    SidebarEvent::StashSelected(index) => {
                        log::info!("Stash {} selected", index);
                        // TODO: show stash diff when stash diff API is available
                    }
                    SidebarEvent::TagSelected(name) => {
                        log::info!("Tag {} selected", name);
                        // TODO: scroll graph to tag commit
                    }
                    SidebarEvent::BranchSelected(_)
                    | SidebarEvent::BranchCreate
                    | SidebarEvent::BranchDelete(_) => {
                        // These are informational / future features
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
            move |_this, _graph, event: &GraphViewEvent, cx| {
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
                }
            }
        })
        .detach();

        // When detail panel emits events (file selection), update diff viewer
        cx.subscribe(&detail_panel, {
            let diff_viewer = diff_viewer.clone();
            move |_this, _dp, event: &DetailPanelEvent, cx| {
                match event {
                    DetailPanelEvent::FileSelected(file_diff, path) => {
                        let p = path.clone();
                        let fd = file_diff.clone();
                        diff_viewer.update(cx, |dv, cx| {
                            dv.set_diff(fd, p, false, cx);
                        });
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
            move |_this, _cp, event: &CommitPanelEvent, cx| {
                match event {
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
                            proj.fetch("origin", cx).detach();
                        });
                    }
                    ToolbarEvent::Pull => {
                        project.update(cx, |proj, cx| {
                            proj.pull("origin", cx).detach();
                        });
                    }
                    ToolbarEvent::Push => {
                        project.update(cx, |proj, cx| {
                            proj.push("origin", false, cx).detach();
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
                        this.show_branch_creation = true;
                        cx.notify();
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
        cx.notify();
        Ok(())
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab = self.tabs.len() - 1;
            }
            cx.notify();
        }
    }

    pub fn active_project(&self) -> Option<&Entity<GitProject>> {
        self.tabs.get(self.active_tab).map(|t| &t.project)
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let modifiers = &keystroke.modifiers;

        // Dismiss branch creation dialog on Escape
        if key == "escape" && self.show_branch_creation {
            self.show_branch_creation = false;
            cx.notify();
            return;
        }

        // Ctrl+Shift+P or Cmd+Shift+P to open command palette
        if (modifiers.control || modifiers.platform) && modifiers.shift && key == "p" {
            self.command_palette.update(cx, |cp, cx| {
                cp.toggle(window, cx);
            });
        }

        // F5 to refresh
        if key == "f5" {
            self.execute_command("refresh", cx);
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
                    cx.notify();
                }
                "]" | "bracketright" => {
                    self.detail_panel_width = (self.detail_panel_width + 20.0).min(480.0);
                    cx.notify();
                }
                // Ctrl+Up / Ctrl+Down to resize diff viewer height
                "up" => {
                    self.diff_viewer_height = (self.diff_viewer_height - 30.0).max(100.0);
                    cx.notify();
                }
                "down" => {
                    self.diff_viewer_height = (self.diff_viewer_height + 30.0).min(600.0);
                    cx.notify();
                }
                _ => {}
            }
        }
    }

    fn render_welcome(&self, cx: &App) -> impl IntoElement {
        let colors = cx.colors();

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.background)
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        Label::new("rgitui")
                            .size(LabelSize::Large)
                            .weight(gpui::FontWeight::BOLD)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("Open a repository to get started")
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .child(
                        Label::new("Ctrl+Shift+P for Command Palette")
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
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
                .child(self.render_welcome(cx))
                .child(self.command_palette.clone())
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
        let tab_count = self.tabs.len();
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
                .closeable(tab_count > 1)
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

        div()
            .id("workspace-root")
            .v_flex()
            .size_full()
            .bg(colors.background)
            .on_key_down(cx.listener(Self::handle_key_down))
            // Title bar
            .child(
                TitleBar::new(repo_name.clone(), branch_name.clone()).has_changes(has_changes),
            )
            // Toolbar
            .child(active_tab.toolbar.clone())
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
                            let new_w =
                                f32::from(e.event.position.x - this.content_bounds.left())
                                    .clamp(120., 600.);
                            this.sidebar_width = new_w;
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DetailPanelResize>(cx.listener(
                        |this, e: &DragMoveEvent<DetailPanelResize>, _, cx| {
                            let new_w =
                                f32::from(this.content_bounds.right() - e.event.position.x)
                                    .clamp(180., 600.);
                            this.detail_panel_width = new_w;
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DiffViewerResize>(cx.listener(
                        |this, e: &DragMoveEvent<DiffViewerResize>, _, cx| {
                            let new_h =
                                f32::from(this.content_bounds.bottom() - e.event.position.y)
                                    .clamp(60., 500.);
                            this.diff_viewer_height = new_h;
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<CommitInputResize>(cx.listener(
                        |this, e: &DragMoveEvent<CommitInputResize>, _, cx| {
                            let new_h =
                                f32::from(this.right_panel_bounds.bottom() - e.event.position.y)
                                    .clamp(100., 400.);
                            this.commit_input_height = new_h;
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
                                    .w(px(8.))
                                    .cursor_col_resize()
                                    .on_drag(SidebarResize, |val, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| val.clone())
                                    })
                                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                    }),
                            ),
                    )
                    // Center: graph (flex) + resize strip + diff viewer (fixed height)
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            // Graph view
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(active_tab.graph.clone()),
                            )
                            // Drag-to-resize strip between graph and diff viewer
                            .child(
                                div()
                                    .id("diff-resize-handle")
                                    .w_full()
                                    .h(px(8.))
                                    .flex_shrink_0()
                                    .cursor_row_resize()
                                    .bg(colors.border_variant)
                                    .hover(|s| s.bg(colors.border_focused))
                                    .on_drag(DiffViewerResize, |val, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| val.clone())
                                    })
                                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
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
                                            entity.update(cx, |this, _| this.right_panel_bounds = bounds);
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
                                    .h(px(8.))
                                    .flex_shrink_0()
                                    .cursor_row_resize()
                                    .bg(colors.border_variant)
                                    .hover(|s| s.bg(colors.border_focused))
                                    .on_drag(CommitInputResize, |val, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| val.clone())
                                    })
                                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
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
                                    .left(px(-4.))
                                    .h_full()
                                    .w(px(8.))
                                    .cursor_col_resize()
                                    .on_drag(DetailPanelResize, |val, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| val.clone())
                                    })
                                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                    }),
                            )
                    })
            })
            // Status bar
            .child(status_bar)
            // Command palette overlay (rendered last to be on top)
            .child(self.command_palette.clone())
            // Branch creation dialog overlay
            .when(self.show_branch_creation, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::Hsla {
                            h: 0.0,
                            s: 0.0,
                            l: 0.0,
                            a: 0.5,
                        })
                        .child(
                            div()
                                .id("branch-creation-dialog")
                                .w(px(420.))
                                .elevation_3(cx)
                                .p_4()
                                .v_flex()
                                .gap_3()
                                .child(
                                    Label::new("Create Branch")
                                        .size(LabelSize::Large)
                                        .weight(gpui::FontWeight::BOLD)
                                        .color(Color::Default),
                                )
                                .child(
                                    Label::new("Run the following command in your terminal:")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .bg(colors.editor_background)
                                        .rounded_md()
                                        .border_1()
                                        .border_color(colors.border_variant)
                                        .child(
                                            Label::new("git checkout -b <branch-name>")
                                                .size(LabelSize::Small)
                                                .color(Color::Accent),
                                        ),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Label::new("Press Escape or click Close to dismiss")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            Button::new("close-branch-dialog", "Close")
                                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                                    this.show_branch_creation = false;
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}
