use anyhow::Result;
use gpui::prelude::*;
use gpui::Context;
use rgitui_git::GitProject;
use rgitui_settings::{LayoutSettings, StoredWorkspace};

use crate::command_palette::CommandContext;
use crate::ToastKind;

use super::{BottomPanelMode, ProjectTab, RightPanelMode, Workspace};

impl Workspace {
    /// Open a repository as a new tab.
    pub fn open_repo(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) -> Result<()> {
        log::info!("open_repo: path={}", path.display());
        // Check if already open
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.project.read(cx).repo_path() == path)
        {
            log::debug!("open_repo: already open at tab {}", idx);
            self.active_tab = idx;
            // Update command palette with the switched-to project's context.
            let proj = self.tabs[idx].project.read(cx);
            let prs_panel = &self.tabs[idx].prs_panel;
            let ctx = CommandContext {
                has_remotes: !proj.remotes().is_empty(),
                has_changes: proj.has_changes(),
                worktree_clean: proj.repo_state().is_clean(),
                is_bisecting: matches!(proj.repo_state(), rgitui_git::RepoState::Bisect),
                has_stashes: !proj.stashes().is_empty(),
                has_staged: !proj.status().staged.is_empty(),
                in_progress_operation: matches!(
                    proj.repo_state(),
                    rgitui_git::RepoState::Merge
                        | rgitui_git::RepoState::Rebase
                        | rgitui_git::RepoState::RebaseInteractive
                        | rgitui_git::RepoState::RebaseMerge
                        | rgitui_git::RepoState::CherryPick
                        | rgitui_git::RepoState::CherryPickSequence
                        | rgitui_git::RepoState::Revert
                        | rgitui_git::RepoState::RevertSequence
                ),
                has_github_token: prs_panel.read(cx).github_token().is_some(),
            };
            self.overlays.command_palette.update(cx, |cp, _cx| {
                cp.set_context(ctx);
            });
            cx.notify();
            return Ok(());
        }

        // Validate that path exists and can be opened as a git repository.
        // Use Repository::open (not discover) to match what GitProject::open calls internally,
        // preventing false positives when a non-repo path is inside a parent repo.
        if !path.exists() {
            anyhow::bail!("Path does not exist: {}", path.display());
        }
        git2::Repository::open(&path).map_err(|e| {
            anyhow::anyhow!("Failed to open repository at {}: {}", path.display(), e)
        })?;

        let mut open_error: Option<String> = None;
        let project = cx.new(|cx| match GitProject::open(path.clone(), cx) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to open git project at {}: {}", path.display(), e);
                open_error = Some(format!("Failed to open repository: {}", e));
                GitProject::empty_at(path.clone())
            }
        });
        if let Some(err) = open_error {
            anyhow::bail!("{}", err);
        }
        project.update(cx, |proj, cx| {
            proj.refresh(cx).detach();
        });

        let graph = cx.new(rgitui_graph::GraphView::new);
        let diff_viewer = cx.new(rgitui_diff::DiffViewer::new);
        let blame_view = cx.new(crate::BlameView::new);
        let file_history_view = cx.new(crate::FileHistoryView::new);
        let reflog_view = cx.new(crate::ReflogView::new);
        let submodule_view = cx.new(crate::SubmoduleView::new);
        let detail_panel = cx.new(crate::DetailPanel::new);
        let sidebar = cx.new(crate::Sidebar::new);
        let commit_panel = cx.new(crate::CommitPanel::new);
        let toolbar = cx.new(|_cx| crate::Toolbar::new());
        let global_search_view = cx.new(crate::GlobalSearchView::new);

        // Set the repo name on the sidebar header
        let repo_display_name = project.read(cx).repo_name().to_string();
        sidebar.update(cx, |s, cx| {
            s.set_repo_name(repo_display_name, cx);
        });

        // Set up subscriptions for child component events
        super::events::subscribe_project(cx, &project, &graph, &sidebar, &commit_panel, &toolbar);
        super::events::subscribe_sidebar(cx, &project, &sidebar, &diff_viewer, &detail_panel);
        super::events::subscribe_graph(cx, &project, &graph, &diff_viewer, &detail_panel);
        super::events::subscribe_detail_panel(cx, &project, &diff_viewer, &detail_panel);
        super::events::subscribe_diff_viewer(cx, &project, &diff_viewer);
        super::events::subscribe_commit_panel(cx, &project, &self.ai.clone(), &commit_panel);
        super::events::subscribe_toolbar(cx, &project, &toolbar);
        super::events::subscribe_blame_view(cx, &blame_view, &graph);
        super::events::subscribe_file_history_view(cx, &file_history_view, &graph);
        super::events::subscribe_reflog_view(cx, &project, &reflog_view, &graph);
        super::events::subscribe_submodule_view(cx, &submodule_view, &project);
        super::events::subscribe_global_search(cx, &global_search_view);

        // Initial sync
        {
            let proj = project.read(cx);
            let commits = proj.recent_commits_arc();
            let has_more = proj.has_more_commits();
            let init_status = proj.status_arc();
            let branches = proj.branches().to_vec();
            let tags = proj.tags().to_vec();
            let remotes = proj.remotes().to_vec();
            let stashes = proj.stashes().to_vec();
            let worktrees = proj.worktrees().to_vec();
            let staged_count = init_status.staged.len();
            let mut seen = std::collections::HashSet::new();
            let authors: Vec<(String, String)> = commits
                .iter()
                .filter(|c| seen.insert(c.author.email.clone()))
                .map(|c| (c.author.name.clone(), c.author.email.clone()))
                .collect();
            crate::avatar_resolver::resolve_avatars(authors, cx);
            let init_staged = init_status.staged.len();
            let init_unstaged = init_status.unstaged.len();
            let init_staged_bd = rgitui_graph::compute_breakdown(&init_status.staged);
            let init_unstaged_bd = rgitui_graph::compute_breakdown(&init_status.unstaged);
            graph.update(cx, |g, cx| {
                g.set_commits(commits, cx);
                g.set_all_loaded(!has_more);
                g.set_working_tree_status(
                    init_staged,
                    init_unstaged,
                    init_staged_bd,
                    init_unstaged_bd,
                    cx,
                );
            });

            sidebar.update(cx, |s, cx| {
                s.update_branches(branches, cx);
                s.update_tags(tags, cx);
                s.update_remotes(remotes, cx);
                s.update_stashes(stashes, cx);
                s.update_worktrees(worktrees, cx);
                s.update_status(init_status.staged.clone(), init_status.unstaged.clone(), cx);
            });

            commit_panel.update(cx, |cp, cx| cp.set_staged_count(staged_count, cx));
        }

        // Compute ahead/behind for all branches in the background after the
        // initial fast refresh. This avoids blocking the first render with
        // expensive graph walks (particularly impactful on repos with many branches).
        project.update(cx, |proj, cx| {
            proj.refresh_ahead_behind(cx);
        });

        let issues_panel = cx.new(crate::IssuesPanel::new);
        let workspace_weak = cx.entity().downgrade();
        let prs_panel = cx.new(|cx| crate::PrsPanel::new(cx, workspace_weak));

        // Configure issues and PRs panels with GitHub remote info and token
        {
            let remotes = project.read(cx).remotes();
            let remote_url = remotes
                .iter()
                .find(|r| r.name == "origin")
                .or_else(|| remotes.first())
                .and_then(|r| r.url.clone());

            if let Some(url) = remote_url {
                if let Some((owner, repo_name)) = crate::issues_panel::parse_github_owner_repo(&url)
                {
                    let token = rgitui_settings::current_auth_runtime()
                        .git
                        .providers
                        .iter()
                        .find(|p| p.host == "github.com")
                        .and_then(|p| p.token.clone());

                    issues_panel.update(cx, |ip, cx| {
                        ip.configure(token.clone(), owner.clone(), repo_name.clone(), cx);
                    });
                    prs_panel.update(cx, |pp, cx| {
                        pp.configure(token, owner, repo_name, cx);
                    });
                }
            }
        }

        let proj = project.read(cx);
        let name = proj.repo_name().to_string();
        let ctx = CommandContext {
            has_remotes: !proj.remotes().is_empty(),
            has_changes: proj.has_changes(),
            worktree_clean: proj.repo_state().is_clean(),
            is_bisecting: matches!(proj.repo_state(), rgitui_git::RepoState::Bisect),
            has_stashes: !proj.stashes().is_empty(),
            has_staged: !proj.status().staged.is_empty(),
            in_progress_operation: matches!(
                proj.repo_state(),
                rgitui_git::RepoState::Merge
                    | rgitui_git::RepoState::Rebase
                    | rgitui_git::RepoState::RebaseInteractive
                    | rgitui_git::RepoState::RebaseMerge
                    | rgitui_git::RepoState::CherryPick
                    | rgitui_git::RepoState::CherryPickSequence
                    | rgitui_git::RepoState::Revert
                    | rgitui_git::RepoState::RevertSequence
            ),
            has_github_token: prs_panel.read(cx).github_token().is_some(),
        };
        self.tabs.push(ProjectTab {
            name,
            project,
            graph,
            diff_viewer,
            blame_view,
            file_history_view,
            reflog_view,
            submodule_view,
            detail_panel,
            sidebar,
            commit_panel,
            toolbar,
            issues_panel,
            prs_panel,
            global_search_view,
            right_panel_mode: RightPanelMode::Details,
            bottom_panel_mode: BottomPanelMode::Diff,
            caches: super::ViewCaches::new(),
        });
        self.active_tab = self.tabs.len() - 1;
        log::info!("open_repo: opened as tab {}", self.tabs.len() - 1);
        self.overlays.command_palette.update(cx, |cp, _cx| {
            cp.set_context(ctx);
        });
        self.persist_workspace_snapshot(cx);

        cx.notify();
        Ok(())
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        log::info!("close_tab: index={}", index);
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab = self.tabs.len() - 1;
            }
            self.save_workspace_state(cx);
            cx.notify();
        }
    }

    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        // Mark clean exit before clearing workspace
        self.mark_clean_exit(cx);
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
        self.set_status_message(format!("Opened workspace '{}'", snapshot.name), cx);
        self.persist_workspace_snapshot(cx);
        cx.notify();
        Ok(())
    }

    pub(super) fn restore_last_workspace(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn current_layout_settings(&self) -> LayoutSettings {
        LayoutSettings {
            sidebar_width: self.layout.sidebar_width,
            detail_panel_width: self.layout.detail_panel_width,
            diff_viewer_height: self.layout.diff_viewer_height,
            commit_input_height: self.layout.commit_input_height,
        }
    }

    pub(super) fn apply_layout_settings(&mut self, layout: &LayoutSettings) {
        self.layout.sidebar_width = layout.sidebar_width;
        self.layout.detail_panel_width = layout.detail_panel_width;
        self.layout.diff_viewer_height = layout.diff_viewer_height;
        self.layout.commit_input_height = layout.commit_input_height.max(300.0);
    }

    pub(super) fn persist_workspace_snapshot(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn clear_active_workspace_state(&mut self, cx: &mut Context<Self>) {
        self.active_workspace_id = None;
        self.status_message = None;
        self.operations.is_loading = false;
        self.operations.loading_message = None;
        self.operations.active_git_operation = None;
        self.operations.last_failed_git_operation = None;
        self.operations.active_operations.clear();
        self.operations.last_operation_output = None;

        if cx.try_global::<rgitui_settings::SettingsState>().is_some() {
            let settings = cx.global_mut::<rgitui_settings::SettingsState>();
            settings.clear_active_workspace();
            if let Err(error) = settings.save() {
                log::error!("Failed to clear active workspace: {}", error);
            }
        }
    }

    /// Persist the current set of open repo paths and layout to settings.
    pub(super) fn save_workspace_state(&mut self, cx: &mut Context<Self>) {
        self.save_layout(cx);
        if self.tabs.is_empty() {
            self.clear_active_workspace_state(cx);
        } else {
            self.persist_workspace_snapshot(cx);
        }
    }
}
