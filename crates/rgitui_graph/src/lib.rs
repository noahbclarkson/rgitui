use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, point, px, uniform_list, App, Bounds, ClickEvent, Context, CursorStyle,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, ListSizingBehavior,
    MouseButton, MouseDownEvent, ObjectFit, PathBuilder, Pixels, Point, Render, ScrollStrategy,
    SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{compute_graph, CommitInfo, FileChangeKind, GraphEdge, GraphRow, RefLabel};
use rgitui_settings::{GraphStyle, SettingsState};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    AvatarCache, Badge, CheckState, Checkbox, Icon, IconName, IconSize, Label, LabelSize,
};

/// Events emitted by the graph view.
#[derive(Debug, Clone)]
pub enum GraphViewEvent {
    CommitSelected(git2::Oid),
    CherryPick(git2::Oid),
    RevertCommit(git2::Oid),
    CreateBranchAtCommit(git2::Oid),
    CheckoutCommit(git2::Oid),
    CopyCommitSha(String),
    CreateTagAtCommit(git2::Oid),
    ResetToCommit(git2::Oid, String),
    /// Request to load more commits beyond the current set.
    LoadMoreCommits,
    /// The virtual "working tree" row was selected.
    WorkingTreeSelected,
    /// Mark commit as "good" during bisect.
    BisectGood(git2::Oid),
    /// Mark commit as "bad" during bisect.
    BisectBad(git2::Oid),
}

/// State for the right-click context menu.
struct ContextMenuState {
    /// The index of the commit that was right-clicked.
    commit_index: usize,
    /// Screen position where the menu should appear.
    position: Point<Pixels>,
}

/// The commit graph panel.
pub struct GraphView {
    commits: Arc<Vec<CommitInfo>>,
    graph_rows: Arc<Vec<GraphRow>>,
    global_max_lane: usize,
    selected_index: Option<usize>,
    selected_oid: Option<git2::Oid>,
    row_height: f32,
    scroll_handle: UniformListScrollHandle,
    context_menu: Option<ContextMenuState>,
    show_search: bool,
    filter_matches: Vec<usize>,
    filter_match_set: HashSet<usize>,
    filter_match_set_arc: Arc<HashSet<usize>>,
    current_match: usize,
    search_editor: Entity<rgitui_ui::TextInput>,
    graph_focus: FocusHandle,
    all_commits_loaded: bool,
    /// OID to scroll to once the commit list is refreshed and the OID is present.
    /// Used when scroll_to_commit is called for a commit not yet in the loaded list.
    pending_scroll_oid: Option<git2::Oid>,
    cached_graph_hash: u64,
    search_debounce_task: Option<gpui::Task<()>>,
    staged_count: usize,
    unstaged_count: usize,
    staged_breakdown: HashMap<FileChangeKind, usize>,
    unstaged_breakdown: HashMap<FileChangeKind, usize>,
    cached_merge_breakdown: HashMap<FileChangeKind, usize>,
    show_settings_popover: bool,
    show_full_hash: bool,
    show_author_column: bool,
    show_date_column: bool,
    show_avatars: bool,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl GraphView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_editor = cx.new(|cx| {
            let mut ti = rgitui_ui::TextInput::new(cx);
            ti.set_placeholder("Search commits...");
            ti
        });

        cx.subscribe(
            &search_editor,
            |this: &mut Self, _, event: &rgitui_ui::TextInputEvent, cx| match event {
                rgitui_ui::TextInputEvent::Changed(_) => {
                    this.schedule_search_filter(cx);
                }
                rgitui_ui::TextInputEvent::Submit => {
                    this.jump_to_next_match(cx);
                }
            },
        )
        .detach();

        Self {
            commits: Arc::new(Vec::new()),
            graph_rows: Arc::new(Vec::new()),
            global_max_lane: 0,
            selected_index: None,
            selected_oid: None,
            row_height: 32.0,
            scroll_handle: UniformListScrollHandle::new(),
            context_menu: None,
            show_search: false,
            filter_matches: Vec::new(),
            filter_match_set: HashSet::new(),
            filter_match_set_arc: Arc::new(HashSet::new()),
            current_match: 0,
            search_editor,
            graph_focus: cx.focus_handle(),
            all_commits_loaded: false,
            pending_scroll_oid: None,
            cached_graph_hash: 0,
            search_debounce_task: None,
            staged_count: 0,
            unstaged_count: 0,
            staged_breakdown: HashMap::new(),
            unstaged_breakdown: HashMap::new(),
            cached_merge_breakdown: HashMap::new(),
            show_settings_popover: false,
            show_full_hash: false,
            show_author_column: true,
            show_date_column: true,
            show_avatars: true,
        }
    }

    /// Focus the graph view for keyboard navigation.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.graph_focus.focus(window, cx);
        cx.notify();
    }

    /// Check if the graph view is currently focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.graph_focus.is_focused(window)
    }

    pub fn set_commits(&mut self, commits: Arc<Vec<CommitInfo>>, cx: &mut Context<Self>) {
        // Compute a simple hash to detect if commits actually changed
        let new_hash = Self::compute_commits_hash(&commits);
        if new_hash == self.cached_graph_hash && !self.commits.is_empty() {
            return;
        }
        self.cached_graph_hash = new_hash;

        // Preserve selection by OID across refreshes
        let prev_selected_oid = self.selected_oid;

        let graph_rows = compute_graph(&commits);
        self.global_max_lane = graph_rows
            .iter()
            .map(|r| r.lane_count)
            .max()
            .unwrap_or(1)
            .max(1);
        self.graph_rows = Arc::new(graph_rows);
        self.commits = commits;

        // Restore selection if the previously selected commit still exists
        let offset = self.working_tree_offset();
        if let Some(prev_oid) = prev_selected_oid {
            if let Some(new_index) = self.commits.iter().position(|c| c.oid == prev_oid) {
                self.selected_index = Some(new_index + offset);
            } else {
                self.selected_index = None;
                self.selected_oid = None;
            }
        } else if self.selected_index == Some(0) && offset > 0 {
            // Working tree row was selected; keep it selected
            self.selected_index = Some(0);
        } else {
            self.selected_index = None;
        }

        if self.show_search && !self.search_editor.read(cx).is_empty() {
            self.update_search_filter(cx);
        }

        // Check if a pending scroll target has just been loaded.
        if let Some(pending_oid) = self.pending_scroll_oid {
            if let Some(index) = self.commits.iter().position(|c| c.oid == pending_oid) {
                let list_index = index + offset;
                self.select_list_index(list_index, cx);
                self.scroll_handle
                    .scroll_to_item(list_index, ScrollStrategy::Top);
            }
            // Keep pending_scroll_oid set if not found yet — more loads may be needed.
            // Only clear it when all commits are loaded and still not found.
            if self.all_commits_loaded {
                self.pending_scroll_oid = None;
            }
        }

        cx.notify();
    }

    /// Mark that all available commits have been loaded (disables "load more").
    pub fn set_all_loaded(&mut self, loaded: bool) {
        self.all_commits_loaded = loaded;
    }

    /// Update the working tree status counts and file-type breakdowns.
    pub fn set_working_tree_status(
        &mut self,
        staged: usize,
        unstaged: usize,
        staged_breakdown: HashMap<FileChangeKind, usize>,
        unstaged_breakdown: HashMap<FileChangeKind, usize>,
        cx: &mut Context<Self>,
    ) {
        let changed = self.staged_count != staged
            || self.unstaged_count != unstaged
            || self.staged_breakdown != staged_breakdown
            || self.unstaged_breakdown != unstaged_breakdown;
        self.staged_count = staged;
        self.unstaged_count = unstaged;
        self.cached_merge_breakdown = merge_breakdowns(&staged_breakdown, &unstaged_breakdown);
        self.staged_breakdown = staged_breakdown;
        self.unstaged_breakdown = unstaged_breakdown;
        if changed {
            cx.notify();
        }
    }

    /// Whether we show the virtual working tree row (only when there are uncommitted changes).
    fn has_working_tree_row(&self) -> bool {
        !self.commits.is_empty() && (self.staged_count > 0 || self.unstaged_count > 0)
    }

    /// The offset added to commit indices when the working tree row is visible.
    fn working_tree_offset(&self) -> usize {
        if self.has_working_tree_row() {
            1
        } else {
            0
        }
    }

    /// Total number of list items (working tree row + commits).
    fn total_list_items(&self) -> usize {
        self.commits.len() + self.working_tree_offset()
    }

    pub fn selected_commit(&self) -> Option<&CommitInfo> {
        self.selected_index.and_then(|i| {
            let offset = self.working_tree_offset();
            if i < offset {
                None
            } else {
                self.commits.get(i - offset)
            }
        })
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Total number of rows in the list (includes the virtual working tree row).
    pub fn row_count(&self) -> usize {
        self.total_list_items()
    }

    fn dismiss_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.is_some() {
            self.context_menu = None;
            cx.notify();
        }
    }

    fn show_context_menu(&mut self, index: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some(ContextMenuState {
            commit_index: index,
            position,
        });
        cx.notify();
    }

    /// Scroll to the commit with the given OID, selecting it and emitting CommitSelected.
    pub fn scroll_to_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) {
        let offset = self.working_tree_offset();
        if let Some(index) = self.commits.iter().position(|c| c.oid == oid) {
            let list_index = index + offset;
            self.select_list_index(list_index, cx);
            self.scroll_handle
                .scroll_to_item(list_index, ScrollStrategy::Top);
        } else if !self.all_commits_loaded {
            // Commit not in loaded list — set as pending and trigger loading.
            self.pending_scroll_oid = Some(oid);
            cx.emit(GraphViewEvent::LoadMoreCommits);
        }
    }

    /// Select an item by its index in the uniform list.
    /// This is the public API used by external callers (workspace vim-key navigation).
    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        self.select_list_index(index, cx);
    }

    /// Select an item by its index in the uniform list (accounts for working tree row).
    fn select_list_index(&mut self, list_index: usize, cx: &mut Context<Self>) {
        let offset = self.working_tree_offset();
        let total = self.total_list_items();
        if list_index >= total {
            return;
        }
        self.selected_index = Some(list_index);
        if list_index < offset {
            // Working tree row selected
            self.selected_oid = None;
            cx.emit(GraphViewEvent::WorkingTreeSelected);
        } else {
            let commit_index = list_index - offset;
            if let Some(commit) = self.commits.get(commit_index) {
                self.selected_oid = Some(commit.oid);
                cx.emit(GraphViewEvent::CommitSelected(commit.oid));
            }
        }
        cx.notify();
    }

    /// Toggle the search bar visibility. Clears query when hiding.
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if !self.show_search {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
            self.filter_match_set_arc = Arc::new(HashSet::new());
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Toggle search with window access (focuses the search input).
    pub fn toggle_search_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if self.show_search {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
        } else {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
            self.filter_match_set_arc = Arc::new(HashSet::new());
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Returns whether the search bar is currently visible.
    pub fn is_search_visible(&self) -> bool {
        self.show_search
    }

    /// Compute a simple hash of the commit list for change detection.
    fn compute_commits_hash(commits: &[CommitInfo]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        commits.len().hash(&mut hasher);
        for c in commits {
            c.oid.as_bytes().hash(&mut hasher);
            c.refs.len().hash(&mut hasher);
            for r in &c.refs {
                r.display_name().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Update the filter_matches list based on the current search_query.
    /// Matches case-insensitively against commit message, author name, author email, and short SHA.
    fn schedule_search_filter(&mut self, cx: &mut Context<Self>) {
        self.search_debounce_task = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            smol::Timer::after(std::time::Duration::from_millis(150)).await;
            cx.update(|cx| {
                let _ = this.update(cx, |this, cx| {
                    this.update_search_filter(cx);
                    this.jump_to_first_match(cx);
                    cx.notify();
                });
            });
        }));
    }

    fn update_search_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_matches.clear();
        self.filter_match_set.clear();
        self.current_match = 0;

        if self.search_editor.read(cx).is_empty() {
            self.filter_match_set_arc = Arc::new(HashSet::new());
            return;
        }

        let query = self.search_editor.read(cx).text().to_lowercase();
        for (i, commit) in self.commits.iter().enumerate() {
            if commit.summary.to_lowercase().contains(&query)
                || commit.message.to_lowercase().contains(&query)
                || commit.author.name.to_lowercase().contains(&query)
                || commit.author.email.to_lowercase().contains(&query)
                || commit.short_id.to_lowercase().contains(&query)
            {
                self.filter_matches.push(i);
                self.filter_match_set.insert(i);
            }
        }
        self.filter_match_set_arc = Arc::new(self.filter_match_set.clone());
    }

    /// Jump to the next search match, selecting and scrolling to it.
    fn jump_to_next_match(&mut self, cx: &mut Context<Self>) {
        if self.filter_matches.is_empty() {
            return;
        }
        self.current_match = (self.current_match + 1) % self.filter_matches.len();
        let commit_index = self.filter_matches[self.current_match];
        let list_index = commit_index + self.working_tree_offset();
        self.select_list_index(list_index, cx);
        self.scroll_handle
            .scroll_to_item(list_index, ScrollStrategy::Top);
    }

    /// Jump to the previous search match.
    fn jump_to_prev_match(&mut self, cx: &mut Context<Self>) {
        if self.filter_matches.is_empty() {
            return;
        }
        if self.current_match == 0 {
            self.current_match = self.filter_matches.len() - 1;
        } else {
            self.current_match -= 1;
        }
        let commit_index = self.filter_matches[self.current_match];
        let list_index = commit_index + self.working_tree_offset();
        self.select_list_index(list_index, cx);
        self.scroll_handle
            .scroll_to_item(list_index, ScrollStrategy::Top);
    }

    /// Jump to first match after updating the search filter.
    fn jump_to_first_match(&mut self, cx: &mut Context<Self>) {
        if !self.filter_matches.is_empty() {
            self.current_match = 0;
            let commit_index = self.filter_matches[0];
            let list_index = commit_index + self.working_tree_offset();
            self.select_list_index(list_index, cx);
            self.scroll_handle
                .scroll_to_item(list_index, ScrollStrategy::Top);
        }
    }

    /// Handle key events on the focused search input.
    fn handle_graph_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .search_editor
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            if event.keystroke.key.as_str() == "escape" {
                self.show_search = false;
                self.search_editor
                    .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
                self.filter_matches.clear();
                self.filter_match_set.clear();
                self.filter_match_set_arc = Arc::new(HashSet::new());
                self.current_match = 0;
                self.graph_focus.focus(window, cx);
                cx.notify();
            }
            return;
        }
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        let total = self.total_list_items();
        match key {
            "j" | "down" if !ctrl => {
                let next = match self.selected_index {
                    Some(i) if i + 1 < total => i + 1,
                    None if total > 0 => 0,
                    _ => return,
                };
                self.select_list_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "k" | "up" if !ctrl => {
                let next = match self.selected_index {
                    Some(i) if i > 0 => i - 1,
                    None if total > 0 => 0,
                    _ => return,
                };
                self.select_list_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "g" if !ctrl && !keystroke.modifiers.shift
                && total > 0 => {
                    self.select_list_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            "g" if keystroke.modifiers.shift
                && total > 0 => {
                    let last = total - 1;
                    self.select_list_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            "end"
                if total > 0 => {
                    let last = total - 1;
                    self.select_list_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            "home"
                if total > 0 => {
                    self.select_list_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            "/" if !ctrl => {
                self.show_search = true;
                self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
                cx.notify();
            }
            "escape"
                // Dismiss context menu or deselect
                if self.context_menu.is_some() => {
                    self.dismiss_context_menu(cx);
                }
            "y" | "Y" if !ctrl && !keystroke.modifiers.shift => {
                // Copy SHA of selected commit (standard GitKraken shortcut)
                if let Some(idx) = self.selected_index {
                    if let Some(commit) = self.commits.get(idx) {
                        let sha = format!("{}", commit.oid);
                        cx.emit(GraphViewEvent::CopyCommitSha(sha));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Render for GraphView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.commits.is_empty() {
            return div()
                .id("graph-view")
                .v_flex()
                .size_full()
                .bg(colors.panel_background)
                // Header bar — consistent with other panels
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .pl(px(10.))
                        .pr(px(8.))
                        .gap(px(4.))
                        .items_center()
                        .bg(colors.surface_background)
                        .border_b_1()
                        .border_color(colors.border_variant)
                        .child(
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Graph")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    div().flex_1().flex().items_center().justify_center().child(
                        div()
                            .v_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::GitCommit)
                                    .size(IconSize::Medium)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("No commits to display")
                                    .color(Color::Muted)
                                    .size(LabelSize::Small),
                            ),
                    ),
                )
                .into_any_element();
        }

        // Extract colors before closure (can't call cx inside uniform_list closure)
        let selected_bg = colors.element_selected;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;
        let panel_bg = colors.panel_background;
        let border_color = colors.border_variant;
        let accent_border = colors.border_focused;
        let selected_border = colors.border_focused;

        // Search highlight colors (derived from theme accent with adjusted alpha)
        let search_match_bg = gpui::Hsla {
            a: 0.12,
            ..colors.text_accent
        };
        let search_current_bg = gpui::Hsla {
            a: 0.28,
            ..colors.text_accent
        };

        // HEAD emphasis: accent-tinted background for the HEAD row
        let head_row_bg = gpui::Hsla {
            a: 0.08,
            ..colors.text_accent
        };

        // Working tree row color (warning/yellow tint)
        let status_colors = cx.status();
        let working_tree_bg = gpui::Hsla {
            a: 0.06,
            ..status_colors.warning
        };
        let working_tree_border_color = gpui::Hsla {
            a: 0.6,
            ..status_colors.warning
        };
        let working_tree_node_color = status_colors.warning;

        // Cheap Arc clones
        let commits = self.commits.clone();
        let graph_rows = self.graph_rows.clone();
        let selected_index = self.selected_index;
        let view: WeakEntity<GraphView> = cx.weak_entity();

        // Working tree state for the closure
        let wt_offset = self.working_tree_offset();
        let wt_staged_count = self.staged_count;
        let wt_unstaged_count = self.unstaged_count;
        let wt_combined_breakdown = self.cached_merge_breakdown.clone();
        let total_list_items = self.total_list_items();

        // Search state for the render closure — use pre-computed Arc for O(1) clone
        let filter_match_set = Arc::clone(&self.filter_match_set_arc);
        let current_match_index = if self.filter_matches.is_empty() {
            None
        } else {
            Some(self.filter_matches[self.current_match])
        };
        let has_search_query = self.show_search && !self.search_editor.read(cx).is_empty();

        let lane_width: f32 = 20.0;
        let graph_padding_left: f32 = 10.0;
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let graph_style = cx.global::<SettingsState>().settings().graph_style;
        let compact_mul = compactness.multiplier();
        let row_height = compactness.spacing(self.row_height);

        let graph_col_width =
            ((self.global_max_lane as f32 + 1.0) * lane_width + graph_padding_left).max(80.0);

        // Column visibility settings
        let show_author_column = self.show_author_column;
        let show_date_column = self.show_date_column;
        let show_full_hash = self.show_full_hash;
        let show_avatars = self.show_avatars;

        // Header row (not virtualized — always visible)
        let view_settings_toggle = cx.weak_entity();
        let mut header = div()
            .h_flex()
            .items_center()
            .w_full()
            .h(px(26.))
            .pl(px(11.))
            .pr(px(8.))
            .gap_1()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(border_color)
            .child(
                div()
                    .w(px(graph_col_width))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Graph")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            )
            .child(
                div().w(px(80.)).flex_shrink_0().child(
                    Label::new("Hash")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .weight(gpui::FontWeight::SEMIBOLD),
                ),
            )
            .child(
                div().flex_1().child(
                    Label::new("Message")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .weight(gpui::FontWeight::SEMIBOLD),
                ),
            );

        if show_author_column {
            header = header.child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        Icon::new(IconName::User)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Author")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            );
        }

        if show_date_column {
            header = header.child(
                div()
                    .w(px(100.))
                    .flex_shrink_0()
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        Icon::new(IconName::Clock)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Date")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            );
        }

        header = header.child(
            div()
                .id("settings-gear-btn")
                .flex_shrink_0()
                .w(px(22.))
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.bg(hover_bg))
                .active(|s| s.bg(active_bg))
                .on_mouse_down(
                    MouseButton::Left,
                    |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    },
                )
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                    view_settings_toggle
                        .update(cx, |this: &mut GraphView, cx| {
                            this.show_settings_popover = !this.show_settings_popover;
                            cx.notify();
                        })
                        .ok();
                })
                .child(
                    Icon::new(IconName::Settings)
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        // The virtualized list
        let list = uniform_list(
            "graph-commit-list",
            total_list_items,
            move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                range
                    .map(|i| {
                        // Working tree virtual row
                        if i < wt_offset {
                            return render_working_tree_row(WorkingTreeRowParams {
                                list_index: i,
                                selected: selected_index == Some(i),
                                staged_count: wt_staged_count,
                                unstaged_count: wt_unstaged_count,
                                combined_breakdown: wt_combined_breakdown.clone(),
                                row_height,
                                lane_width,
                                graph_padding_left,
                                graph_col_width,
                                working_tree_bg,
                                working_tree_border_color,
                                node_color: working_tree_node_color,
                                selected_bg,
                                hover_bg,
                                active_bg,
                                panel_bg,
                                selected_border,
                                view: view.clone(),
                                show_author_column,
                                show_date_column,
                                compact_mul,
                            });
                        }

                        let commit_idx = i - wt_offset;
                        let commit = &commits[commit_idx];
                        let graph_row = &graph_rows[commit_idx];
                        let selected = selected_index == Some(i);
                        let is_current_match = current_match_index == Some(commit_idx);
                        let is_any_match = has_search_query && filter_match_set.contains(&commit_idx);
                        let is_head_row = graph_row.is_head;
                        let is_merge_commit = graph_row.is_merge;

                        let row_base_bg = if is_head_row {
                            head_row_bg
                        } else {
                            panel_bg
                        };
                        let bg = if selected {
                            selected_bg
                        } else if is_current_match {
                            search_current_bg
                        } else if is_any_match {
                            search_match_bg
                        } else {
                            row_base_bg
                        };
                        let row_hover_bg = if selected { selected_bg } else { hover_bg };
                        let row_active_bg = if selected { selected_bg } else { active_bg };

                        let graph_width = graph_col_width;
                        let node_lane = graph_row.node_lane;
                        let node_x = node_lane as f32 * lane_width + lane_width / 2.0 + graph_padding_left;

                        let node_color = rgitui_theme::lane_color(graph_row.node_color);
                        let has_incoming = graph_row.has_incoming
                            || (commit_idx == 0 && wt_offset > 0);

                        // Author initials for avatar (extract small strings, not the whole CommitInfo)
                        let initials: SharedString = commit
                            .author
                            .name
                            .split_whitespace()
                            .take(2)
                            .filter_map(|w| w.chars().next())
                            .collect::<String>()
                            .to_uppercase()
                            .into();

                        // Ref badges — distinct styling per ref type
                        let mut ref_badges: Vec<Badge> = Vec::new();
                        for r in &commit.refs {
                            let badge = match r {
                                RefLabel::Head => Badge::new("HEAD").color(Color::Warning).bold(),
                                RefLabel::LocalBranch(name) => {
                                    Badge::new(name.clone()).color(Color::Success)
                                }
                                RefLabel::RemoteBranch(name) => {
                                    Badge::new(name.clone()).color(Color::Info).italic()
                                }
                                RefLabel::Tag(name) => {
                                    Badge::new(name.clone()).color(Color::Accent).prefix("tag")
                                }
                            };
                            ref_badges.push(badge);
                        }

                        let hash_display: SharedString = if show_full_hash {
                            format!("{}", commit.oid).into()
                        } else {
                            commit.short_id.clone().into()
                        };
                        let summary: SharedString = commit.summary.clone().into();
                        let author: SharedString = commit.author.name.clone().into();
                        let author_email = commit.author.email.clone();
                        let time_str: SharedString = format_relative_time(&commit.time).into();

                        // Clone only the edges vec for canvas closure (not the entire GraphRow)
                        let edges: Vec<GraphEdge> = graph_row.edges.clone();
                        let row_bg_for_canvas = bg;

                        let view_clone = view.clone();
                        let view_ctx_menu = view.clone();

                        let left_tab_color = if selected {
                            selected_border
                        } else if is_head_row {
                            gpui::Hsla {
                                a: 0.8,
                                ..accent_border
                            }
                        } else {
                            gpui::Hsla {
                                a: 0.4,
                                ..node_color
                            }
                        };

                        let mut row = div()
                            .id(ElementId::NamedInteger("commit-row".into(), i as u64))
                            .h_flex()
                            .items_center()
                            .h(px(row_height))
                            .w_full()
                            .bg(bg)
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| s.bg(row_hover_bg))
                            .active(move |s| s.bg(row_active_bg))
                            .on_click(
                                move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view_clone
                                        .update(cx, |this, cx| {
                                            this.dismiss_context_menu(cx);
                                            this.select_list_index(i, cx);
                                        })
                                        .ok();
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                move |event: &MouseDownEvent,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    view_ctx_menu
                                        .update(cx, |this, cx| {
                                            this.show_context_menu(commit_idx, event.position, cx);
                                        })
                                        .ok();
                                },
                            )
                            .child(
                                div()
                                    .w(px(3.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .bg(left_tab_color),
                            )
                            .child(div().w(px(5.)).flex_shrink_0());

                        // Graph column with canvas + avatar overlay
                        row = row.child(
                            div()
                                .relative()
                                .w(px(graph_width))
                                .flex_shrink_0()
                                .overflow_x_hidden()
                                .h_full()
                                .child(
                                    canvas(
                                        |_bounds: Bounds<Pixels>,
                                         _window: &mut Window,
                                         _cx: &mut App| {},
                                        move |bounds: Bounds<Pixels>,
                                              _: (),
                                              window: &mut Window,
                                              _cx: &mut App| {
                                            let origin = bounds.origin;
                                            let h = bounds.size.height;
                                            let mid_y = px(row_height / 2.0);
                                            let node_x_px = px(node_x);

                                            // 1. Approach segment: incoming line from row above → dot center.
                                            if has_incoming {
                                                let mut approach = PathBuilder::stroke(px(2.0));
                                                approach.move_to(point(
                                                    origin.x + node_x_px,
                                                    origin.y - px(2.0),
                                                ));
                                                approach.line_to(point(
                                                    origin.x + node_x_px,
                                                    origin.y + mid_y,
                                                ));
                                                if let Ok(built) = approach.build() {
                                                    window.paint_path(built, node_color);
                                                }
                                            }

                                            // 2. Draw all edges (pass-throughs + outgoing from dot).
                                            for edge in &edges {
                                                let from_x = px(
                                                    edge.from_lane as f32 * lane_width
                                                        + lane_width / 2.0
                                                        + graph_padding_left,
                                                );
                                                let to_x = px(
                                                    edge.to_lane as f32 * lane_width
                                                        + lane_width / 2.0
                                                        + graph_padding_left,
                                                );
                                                let color =
                                                    rgitui_theme::lane_color(edge.color_index);

                                                let start_y = if edge.from_lane == node_lane {
                                                    origin.y + mid_y
                                                } else {
                                                    origin.y - px(2.0)
                                                };
                                                let end_y = origin.y + h + px(2.0);

                                                let stroke_width = px(2.0);

                                                let edge_color = if edge.is_merge {
                                                    gpui::Hsla { a: 0.85, ..color }
                                                } else {
                                                    color
                                                };

                                                let mut path = PathBuilder::stroke(stroke_width);

                                                if from_x == to_x {
                                                    path.move_to(point(
                                                        origin.x + from_x,
                                                        start_y,
                                                    ));
                                                    path.line_to(point(
                                                        origin.x + to_x,
                                                        end_y,
                                                    ));
                                                } else if edge.from_lane != node_lane {
                                                    let fx = origin.x + from_x;
                                                    let tx = origin.x + to_x;
                                                    let horiz_y = origin.y + mid_y;

                                                    match graph_style {
                                                        GraphStyle::Curved => {
                                                            let ry = (horiz_y - start_y).max(px(1.0));
                                                            let rx = (from_x - to_x).abs().max(px(1.0));
                                                            let bend_y = horiz_y - ry;
                                                            let dir = if tx < fx { -1.0_f32 } else { 1.0_f32 };

                                                            path.move_to(point(fx, start_y));

                                                            let segments = 12_usize;
                                                            for s in 1..=segments {
                                                                let t = s as f32 / segments as f32;
                                                                let angle = t * std::f32::consts::FRAC_PI_2;
                                                                let px_x = fx + rx * dir * angle.sin();
                                                                let px_y = bend_y + ry * (1.0 - angle.cos());
                                                                path.line_to(point(px_x, px_y));
                                                            }
                                                        }
                                                        GraphStyle::Rails => {
                                                            path.move_to(point(fx, start_y));
                                                            path.line_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                        }
                                                        GraphStyle::Angular => {
                                                            path.move_to(point(fx, start_y));
                                                            path.line_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                        }
                                                    }
                                                } else {
                                                    let fx = origin.x + from_x;
                                                    let tx = origin.x + to_x;
                                                    let horiz_y = origin.y + mid_y;

                                                    match graph_style {
                                                        GraphStyle::Curved => {
                                                            let avail = end_y - horiz_y;
                                                            let r = avail.min(px(16.0)).max(px(1.0));
                                                            let dir = if tx < fx { -1.0_f32 } else { 1.0_f32 };

                                                            path.move_to(point(fx, horiz_y));
                                                            let horiz_end_x = tx - r * dir;
                                                            path.line_to(point(horiz_end_x, horiz_y));

                                                            let segments = 12_usize;
                                                            for s in 1..=segments {
                                                                let t = s as f32 / segments as f32;
                                                                let angle = t * std::f32::consts::FRAC_PI_2;
                                                                let arc_x = horiz_end_x + r * dir * angle.sin();
                                                                let arc_y = horiz_y + r * (1.0 - angle.cos());
                                                                path.line_to(point(arc_x, arc_y));
                                                            }

                                                            path.line_to(point(tx, end_y));
                                                        }
                                                        GraphStyle::Rails => {
                                                            path.move_to(point(fx, horiz_y));
                                                            let mid_point_y = horiz_y + (end_y - horiz_y) / 2.0;
                                                            path.line_to(point(fx, mid_point_y));
                                                            path.line_to(point(tx, mid_point_y));
                                                            path.line_to(point(tx, end_y));
                                                        }
                                                        GraphStyle::Angular => {
                                                            path.move_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                            path.line_to(point(tx, end_y));
                                                        }
                                                    }
                                                }

                                                if let Ok(built_path) = path.build() {
                                                    window.paint_path(built_path, edge_color);
                                                }
                                            }

                                            // Node dot: HEAD=larger filled, merge=filled+ring, normal=filled
                                            let dot_radius = if is_head_row {
                                                5.5_f32 * compact_mul
                                            } else if is_merge_commit {
                                                5.0_f32 * compact_mul
                                            } else {
                                                4.0_f32 * compact_mul
                                            };
                                            let cx_x = origin.x + node_x_px;
                                            let cy_y = origin.y + mid_y;
                                            let steps = 36_usize;

                                            // Background ring to occlude lines passing behind the dot
                                            let ring_r = 14.0_f32 * compact_mul;
                                            let mut ring = PathBuilder::fill();
                                            for s in 0..steps {
                                                let angle = (s as f32)
                                                    * std::f32::consts::TAU
                                                    / (steps as f32);
                                                let x = cx_x + px(ring_r * angle.cos());
                                                let y = cy_y + px(ring_r * angle.sin());
                                                if s == 0 {
                                                    ring.move_to(point(x, y));
                                                } else {
                                                    ring.line_to(point(x, y));
                                                }
                                            }
                                            ring.close();
                                            if let Ok(built_ring) = ring.build() {
                                                window
                                                    .paint_path(built_ring, row_bg_for_canvas);
                                            }

                                            // HEAD commit: glow ring + filled circle
                                            if is_head_row {
                                                let glow_r = dot_radius + 4.0 * compact_mul;
                                                let mut glow = PathBuilder::stroke(px(2.5));
                                                for s in 0..=steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x = cx_x + px(glow_r * angle.cos());
                                                    let y = cy_y + px(glow_r * angle.sin());
                                                    if s == 0 {
                                                        glow.move_to(point(x, y));
                                                    } else {
                                                        glow.line_to(point(x, y));
                                                    }
                                                }
                                                if let Ok(built_glow) = glow.build() {
                                                    let glow_color = gpui::Hsla {
                                                        a: 0.35,
                                                        ..node_color
                                                    };
                                                    window.paint_path(built_glow, glow_color);
                                                }

                                                // Filled circle for HEAD
                                                let mut head_fill = PathBuilder::fill();
                                                for s in 0..steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x = cx_x + px(dot_radius * angle.cos());
                                                    let y = cy_y + px(dot_radius * angle.sin());
                                                    if s == 0 {
                                                        head_fill.move_to(point(x, y));
                                                    } else {
                                                        head_fill.line_to(point(x, y));
                                                    }
                                                }
                                                head_fill.close();
                                                if let Ok(built_head) = head_fill.build() {
                                                    window.paint_path(built_head, node_color);
                                                }
                                            } else if is_merge_commit {
                                                // Merge commit: filled circle with outer ring
                                                let mut merge_fill = PathBuilder::fill();
                                                for s in 0..steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x = cx_x + px(dot_radius * angle.cos());
                                                    let y = cy_y + px(dot_radius * angle.sin());
                                                    if s == 0 {
                                                        merge_fill.move_to(point(x, y));
                                                    } else {
                                                        merge_fill.line_to(point(x, y));
                                                    }
                                                }
                                                merge_fill.close();
                                                if let Ok(built_merge_fill) = merge_fill.build() {
                                                    window.paint_path(built_merge_fill, node_color);
                                                }

                                                let outer_r = dot_radius + 2.5;
                                                let mut merge_ring =
                                                    PathBuilder::stroke(px(1.5));
                                                for s in 0..=steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x =
                                                        cx_x + px(outer_r * angle.cos());
                                                    let y =
                                                        cy_y + px(outer_r * angle.sin());
                                                    if s == 0 {
                                                        merge_ring.move_to(point(x, y));
                                                    } else {
                                                        merge_ring.line_to(point(x, y));
                                                    }
                                                }
                                                if let Ok(built_merge) = merge_ring.build() {
                                                    let merge_ring_color = gpui::Hsla {
                                                        a: 0.6,
                                                        ..node_color
                                                    };
                                                    window.paint_path(
                                                        built_merge,
                                                        merge_ring_color,
                                                    );
                                                }
                                            } else {
                                                // Normal commit: filled circle
                                                let mut normal_fill = PathBuilder::fill();
                                                for s in 0..steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x = cx_x + px(dot_radius * angle.cos());
                                                    let y = cy_y + px(dot_radius * angle.sin());
                                                    if s == 0 {
                                                        normal_fill.move_to(point(x, y));
                                                    } else {
                                                        normal_fill.line_to(point(x, y));
                                                    }
                                                }
                                                normal_fill.close();
                                                if let Ok(built_normal) = normal_fill.build() {
                                                    window.paint_path(built_normal, node_color);
                                                }
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                // Avatar overlay: resolved image or initials fallback
                                .when(show_avatars, |el| el.child({
                                    let avatar_url = cx
                                        .try_global::<AvatarCache>()
                                        .and_then(|cache| cache.avatar_url(&author_email))
                                        .map(|s| s.to_string());
                                    let initials_fallback = initials.clone();
                                    let fallback_color = node_color;

                                    let avatar_size = 24.0_f32 * compact_mul;
                                    let mut avatar_container = div()
                                        .absolute()
                                        .left(px(node_x - avatar_size / 2.0))
                                        .top(px((row_height - avatar_size) / 2.0))
                                        .w(px(avatar_size))
                                        .h(px(avatar_size))
                                        .rounded_full()
                                        .overflow_hidden()
                                        .bg(panel_bg)
                                        .flex()
                                        .items_center()
                                        .justify_center();

                                    if let Some(url) = avatar_url {
                                        let fb_initials = initials_fallback.clone();
                                        let fb_color = fallback_color;
                                        avatar_container = avatar_container.child(
                                            img(url)
                                                .rounded_full()
                                                .size_full()
                                                .object_fit(ObjectFit::Cover)
                                                .with_fallback(move || {
                                                    div()
                                                        .size_full()
                                                        .rounded_full()
                                                        .border_1()
                                                        .border_color(fb_color)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .text_color(fb_color)
                                                                .text_size(px(8.0))
                                                                .font_weight(
                                                                    gpui::FontWeight::BOLD,
                                                                )
                                                                .child(fb_initials.clone()),
                                                        )
                                                        .into_any_element()
                                                }),
                                        );
                                    } else {
                                        avatar_container = avatar_container
                                            .border_1()
                                            .border_color(fallback_color)
                                            .child(
                                                div()
                                                    .text_color(fallback_color)
                                                    .text_size(px(8.0))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .child(initials_fallback),
                                            );
                                    }

                                    avatar_container
                                })),
                        );

                        row = row.child(
                            div().w(px(80.)).flex_shrink_0().child(
                                Label::new(hash_display)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .weight(gpui::FontWeight::MEDIUM)
                                    .truncate(),
                            ),
                        );

                        {
                            let mut message_col = div()
                                .flex_1()
                                .min_w_0()
                                .h_flex()
                                .items_center()
                                .gap(px(4.))
                                .overflow_x_hidden();

                            for badge in ref_badges {
                                message_col = message_col.child(
                                    div().flex_shrink_0().child(badge),
                                );
                            }

                            let summary_label = if is_head_row {
                                Label::new(summary)
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate()
                            } else {
                                Label::new(summary).size(LabelSize::Small).truncate()
                            };
                            message_col = message_col.child(summary_label);

                            row = row.child(message_col);
                        }

                        // Author column (conditional)
                        if show_author_column {
                            row = row.child(
                                div().w(px(120.)).flex_shrink_0().px(px(4.)).child(
                                    Label::new(author)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                            );
                        }

                        // Date column (conditional)
                        if show_date_column {
                            row = row.child(
                                div().w(px(100.)).flex_shrink_0().pr(px(8.)).child(
                                    Label::new(time_str)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                            );
                        }

                        row.into_any_element()
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        let mut container = div()
            .id("graph-view")
            .track_focus(&self.graph_focus)
            .key_context("GraphView")
            .on_key_down(cx.listener(Self::handle_graph_key_down))
            .relative()
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(panel_bg)
            .on_mouse_down(MouseButton::Left, {
                let view_dismiss = cx.weak_entity();
                move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    view_dismiss
                        .update(cx, |this: &mut GraphView, cx| {
                            // Only dismiss if click is outside the context menu bounds.
                            // Menu dimensions: 200px wide, 220px tall, anchored at clamped_x/clamped_y.
                            let click_inside_menu = this.context_menu.as_ref().is_some_and(|cm| {
                                let menu_w: Pixels = px(200.);
                                let menu_h: Pixels = px(220.);
                                let x = event.position.x;
                                let y = event.position.y;
                                x >= cm.position.x
                                    && x < cm.position.x + menu_w
                                    && y >= cm.position.y
                                    && y < cm.position.y + menu_h
                            });
                            if !click_inside_menu {
                                this.dismiss_context_menu(cx);
                            }
                            if this.show_settings_popover {
                                this.show_settings_popover = false;
                                cx.notify();
                            }
                        })
                        .ok();
                }
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                // Focus the graph when clicked (for keyboard navigation)
                this.graph_focus.focus(window, cx);
                cx.notify();
            }))
            .child(header);

        // Search bar (shown when search is active)
        if self.show_search {
            let no_matches =
                !self.search_editor.read(cx).is_empty() && self.filter_matches.is_empty();
            let match_count_text: SharedString = if self.search_editor.read(cx).is_empty() {
                String::new().into()
            } else if no_matches {
                "No matches".into()
            } else {
                format!("{}/{}", self.current_match + 1, self.filter_matches.len()).into()
            };
            let match_count_color = if no_matches {
                Color::Warning
            } else {
                Color::Muted
            };

            let view_prev = cx.weak_entity();
            let view_next = cx.weak_entity();
            let has_matches = !self.filter_matches.is_empty();

            let search_bar = div()
                .id("search-bar-container")
                .on_click(|_: &ClickEvent, _, cx: &mut App| {
                    cx.stop_propagation();
                })
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx: &mut App| {
                    cx.stop_propagation();
                })
                .h_flex()
                .items_center()
                .w_full()
                .h(px(40.))
                .px_2()
                .gap_2()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(div().flex_1().child(self.search_editor.clone()))
                .child(
                    Label::new(match_count_text)
                        .size(LabelSize::XSmall)
                        .color(match_count_color),
                )
                .child(
                    div()
                        .id("search-prev")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .when(has_matches, |el| el.hover(move |s| s.bg(hover_bg)))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_prev
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.jump_to_prev_match(cx);
                                })
                                .ok();
                        })
                        .child(Icon::new(IconName::ChevronUp).size(IconSize::Small).color(
                            if has_matches {
                                Color::Default
                            } else {
                                Color::Muted
                            },
                        )),
                )
                .child(
                    div()
                        .id("search-next")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .when(has_matches, |el| el.hover(move |s| s.bg(hover_bg)))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_next
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.jump_to_next_match(cx);
                                })
                                .ok();
                        })
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(IconSize::Small)
                                .color(if has_matches {
                                    Color::Default
                                } else {
                                    Color::Muted
                                }),
                        ),
                );

            container = container.child(search_bar);
        }

        container = container.child(list);

        if !self.all_commits_loaded && !self.commits.is_empty() {
            let view_load = cx.weak_entity();
            container = container.child(
                div()
                    .id("load-more-row")
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .mt(px(4.))
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .bg(colors.surface_background)
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view_load
                            .update(cx, |_this, cx| {
                                cx.emit(GraphViewEvent::LoadMoreCommits);
                            })
                            .ok();
                    })
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Load more commits")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        // Context menu overlay
        if let Some(ref menu_state) = self.context_menu {
            if let Some(commit) = self.commits.get(menu_state.commit_index) {
                let oid = commit.oid;
                let sha = format!("{}", oid);
                let pos = menu_state.position;
                let weak = cx.weak_entity();
                let sha_clone = sha.clone();

                let menu_bg = colors.elevated_surface_background;
                let menu_border = colors.border;
                let menu_hover = colors.ghost_element_hover;
                let menu_active = colors.ghost_element_active;

                let menu_items: Vec<(&str, IconName)> = vec![
                    ("Cherry-pick commit", IconName::GitCommit),
                    ("Revert commit", IconName::Undo),
                    ("Checkout commit", IconName::Check),
                    ("Create branch here", IconName::GitBranch),
                    ("Create tag here", IconName::Tag),
                    ("Mark as good (bisect)", IconName::Check),
                    ("Mark as bad (bisect)", IconName::X),
                    ("Reset to here", IconName::Trash),
                    ("Copy SHA", IconName::Copy),
                ];

                // Clamp menu position to stay within window bounds.
                // Approximate menu dimensions: 200px wide, 220px tall.
                let menu_w = px(200.);
                let menu_h = px(220.);
                let win_bounds = window.bounds();
                let max_x = win_bounds.size.width - menu_w;
                let max_y = win_bounds.size.height - menu_h;
                let clamped_x = if pos.x > max_x { max_x } else { pos.x };
                let clamped_y = if pos.y > max_y { max_y } else { pos.y };

                let mut menu = div()
                    .id("graph-context-menu")
                    .absolute()
                    .left(clamped_x)
                    .top(clamped_y)
                    .v_flex()
                    .min_w(px(200.))
                    .py(px(3.))
                    .bg(menu_bg)
                    .border_1()
                    .border_color(menu_border)
                    .rounded(px(6.))
                    .elevation_3(cx)
                    // Prevent left-click on menu from dismissing via container handler
                    .on_mouse_down(
                        MouseButton::Left,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    // Prevent right-click on menu from opening another menu
                    .on_mouse_down(
                        MouseButton::Right,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    );

                for (idx, (label_text, icon_name)) in menu_items.iter().enumerate() {
                    let label: SharedString = (*label_text).into();
                    let icon = *icon_name;

                    // Add separator before bisect options, before destructive "Reset", and before "Copy SHA"
                    if idx == 5 || idx == 7 || idx == 8 {
                        menu = menu.child(
                            div()
                                .w_full()
                                .h(px(1.))
                                .my(px(2.))
                                .mx(px(8.))
                                .bg(colors.border_variant),
                        );
                    }

                    let mut item = div()
                        .id(ElementId::NamedInteger("ctx-action".into(), idx as u64))
                        .h_flex()
                        .w_full()
                        .h(px(26.))
                        .px(px(8.))
                        .mx(px(4.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(move |s| s.bg(menu_hover))
                        .active(move |s| s.bg(menu_active));

                    match idx {
                        0 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CherryPick(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        1 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::RevertCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        2 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CheckoutCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        3 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CreateBranchAtCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        4 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CreateTagAtCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        5 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::BisectGood(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        6 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::BisectBad(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        7 => {
                            let w = weak.clone();
                            let sha_for_reset = sha_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let sha_val = sha_for_reset.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::ResetToCommit(oid, sha_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        8 => {
                            let w = weak.clone();
                            let sha_for_click = sha_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let sha_val = sha_for_click.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CopyCommitSha(sha_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        _ => {}
                    }

                    // Destructive actions render in error color (Bad and Reset)
                    let item_color = if idx == 6 || idx == 7 {
                        Color::Error
                    } else {
                        Color::Muted
                    };
                    let label_color = if idx == 6 || idx == 7 {
                        Color::Error
                    } else {
                        Color::Default
                    };

                    item = item
                        .child(Icon::new(icon).size(IconSize::XSmall).color(item_color))
                        .child(Label::new(label).size(LabelSize::XSmall).color(label_color));

                    menu = menu.child(item);
                }

                container = container.child(menu);
            }
        }

        // Settings popover overlay
        if self.show_settings_popover {
            let popover_bg = colors.elevated_surface_background;
            let popover_border = colors.border;
            let popover_hover = colors.ghost_element_hover;

            let view_full_hash = cx.weak_entity();
            let view_author = cx.weak_entity();
            let view_date = cx.weak_entity();
            let view_avatars = cx.weak_entity();

            let full_hash_state = if self.show_full_hash {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let author_state = if self.show_author_column {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let date_state = if self.show_date_column {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let avatars_state = if self.show_avatars {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };

            let popover = div()
                .id("graph-settings-popover")
                .absolute()
                .right(px(8.))
                .top(px(28.))
                .v_flex()
                .min_w(px(180.))
                .py(px(4.))
                .bg(popover_bg)
                .border_1()
                .border_color(popover_border)
                .rounded(px(6.))
                .elevation_3(cx)
                .on_mouse_down(
                    MouseButton::Left,
                    |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    },
                )
                .child(
                    div().px(px(10.)).py(px(4.)).child(
                        Label::new("Display Settings")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .my(px(2.))
                        .bg(colors.border_variant),
                )
                .child(
                    div()
                        .id("toggle-full-hash")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_full_hash
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_full_hash = !this.show_full_hash;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-full-hash", full_hash_state))
                        .child(Label::new("Show full hash").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-author-col")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_author
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_author_column = !this.show_author_column;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-author-col", author_state))
                        .child(Label::new("Show author column").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-date-col")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_date
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_date_column = !this.show_date_column;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-date-col", date_state))
                        .child(Label::new("Show date column").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-avatars")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_avatars
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_avatars = !this.show_avatars;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-avatars", avatars_state))
                        .child(Label::new("Show avatars").size(LabelSize::XSmall)),
                );

            container = container.child(popover);
        }

        container.into_any_element()
    }
}

/// Parameters for rendering the virtual working tree row.
#[derive(Clone)]
struct WorkingTreeRowParams {
    list_index: usize,
    selected: bool,
    staged_count: usize,
    unstaged_count: usize,
    combined_breakdown: HashMap<FileChangeKind, usize>,
    row_height: f32,
    lane_width: f32,
    graph_padding_left: f32,
    graph_col_width: f32,
    working_tree_bg: gpui::Hsla,
    working_tree_border_color: gpui::Hsla,
    node_color: gpui::Hsla,
    selected_bg: gpui::Hsla,
    hover_bg: gpui::Hsla,
    active_bg: gpui::Hsla,
    panel_bg: gpui::Hsla,
    selected_border: gpui::Hsla,
    view: WeakEntity<GraphView>,
    show_author_column: bool,
    show_date_column: bool,
    compact_mul: f32,
}

/// Render the virtual "Working Tree" row that appears at the top of the graph.
fn render_working_tree_row(params: WorkingTreeRowParams) -> gpui::AnyElement {
    let WorkingTreeRowParams {
        list_index,
        selected,
        staged_count,
        unstaged_count,
        combined_breakdown,
        row_height,
        lane_width,
        graph_padding_left,
        graph_col_width,
        working_tree_bg,
        working_tree_border_color,
        node_color,
        selected_bg,
        hover_bg,
        active_bg,
        panel_bg,
        selected_border,
        view,
        show_author_column,
        show_date_column,
        compact_mul,
    } = params;
    let bg = if selected {
        selected_bg
    } else {
        working_tree_bg
    };
    let row_hover_bg = if selected { selected_bg } else { hover_bg };
    let row_active_bg = if selected { selected_bg } else { active_bg };
    let left_tab_color = if selected {
        selected_border
    } else {
        working_tree_border_color
    };

    let has_changes = staged_count > 0 || unstaged_count > 0;

    let added_count = combined_breakdown
        .get(&FileChangeKind::Added)
        .copied()
        .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Untracked)
            .copied()
            .unwrap_or(0);
    let modified_count = combined_breakdown
        .get(&FileChangeKind::Modified)
        .copied()
        .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Renamed)
            .copied()
            .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Copied)
            .copied()
            .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::TypeChange)
            .copied()
            .unwrap_or(0);
    let deleted_count = combined_breakdown
        .get(&FileChangeKind::Deleted)
        .copied()
        .unwrap_or(0);
    let conflicted_count = combined_breakdown
        .get(&FileChangeKind::Conflicted)
        .copied()
        .unwrap_or(0);

    let graph_width = graph_col_width;
    let node_x = lane_width / 2.0 + graph_padding_left;

    let view_click = view.clone();

    div()
        .id(ElementId::NamedInteger(
            "commit-row".into(),
            list_index as u64,
        ))
        .h_flex()
        .items_center()
        .h(px(row_height))
        .w_full()
        .bg(bg)
        .border_b_1()
        .border_color(gpui::Hsla {
            a: 0.15,
            ..working_tree_border_color
        })
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(row_hover_bg))
        .active(move |s| s.bg(row_active_bg))
        .on_click(
            move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                view_click
                    .update(cx, |this, cx| {
                        this.dismiss_context_menu(cx);
                        this.select_list_index(list_index, cx);
                    })
                    .ok();
            },
        )
        .child(div().w(px(3.)).h_full().flex_shrink_0().bg(left_tab_color))
        .child(div().w(px(5.)).flex_shrink_0())
        // Graph column with canvas (hollow circle node + connecting line down)
        .child(
            div()
                .relative()
                .w(px(graph_width))
                .flex_shrink_0()
                .overflow_x_hidden()
                .h_full()
                .child(
                    canvas(
                        |_bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut App| {},
                        move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _cx: &mut App| {
                            let origin = bounds.origin;
                            let h = bounds.size.height;
                            let mid_y = px(row_height / 2.0);
                            let node_x_px = px(node_x);
                            let cx_x = origin.x + node_x_px;
                            let cy_y = origin.y + mid_y;

                            // Vertical line from node center to bottom (connects to HEAD row below)
                            let mut line_down = PathBuilder::stroke(px(2.0));
                            line_down.move_to(point(cx_x, cy_y));
                            line_down.line_to(point(cx_x, origin.y + h + px(2.0)));
                            if let Ok(built) = line_down.build() {
                                window.paint_path(built, node_color);
                            }

                            // Background ring to occlude lines behind the dot
                            let ring_r = 13.0_f32 * compact_mul;
                            let steps = 32_usize;
                            let mut ring = PathBuilder::fill();
                            for s in 0..steps {
                                let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                                let x = cx_x + px(ring_r * angle.cos());
                                let y = cy_y + px(ring_r * angle.sin());
                                if s == 0 {
                                    ring.move_to(point(x, y));
                                } else {
                                    ring.line_to(point(x, y));
                                }
                            }
                            ring.close();
                            if let Ok(built_ring) = ring.build() {
                                window.paint_path(built_ring, panel_bg);
                            }

                            // Hollow circle (stroke only, no fill) to distinguish from commits
                            let dot_radius = 5.0_f32 * compact_mul;
                            let mut circle = PathBuilder::stroke(px(2.0));
                            for s in 0..=steps {
                                let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                                let x = cx_x + px(dot_radius * angle.cos());
                                let y = cy_y + px(dot_radius * angle.sin());
                                if s == 0 {
                                    circle.move_to(point(x, y));
                                } else {
                                    circle.line_to(point(x, y));
                                }
                            }
                            if let Ok(built_circle) = circle.build() {
                                window.paint_path(built_circle, node_color);
                            }

                            // Dashed outer ring to further distinguish
                            let outer_r = dot_radius + 3.0 * compact_mul;
                            let dash_count = 8_usize;
                            let arc_per_dash = std::f32::consts::TAU / (dash_count as f32 * 2.0);
                            for d in 0..dash_count {
                                let start_angle =
                                    d as f32 * std::f32::consts::TAU / dash_count as f32;
                                let end_angle = start_angle + arc_per_dash;
                                let mut dash = PathBuilder::stroke(px(1.5));
                                let sx = cx_x + px(outer_r * start_angle.cos());
                                let sy = cy_y + px(outer_r * start_angle.sin());
                                dash.move_to(point(sx, sy));
                                let ex = cx_x + px(outer_r * end_angle.cos());
                                let ey = cy_y + px(outer_r * end_angle.sin());
                                dash.line_to(point(ex, ey));
                                if let Ok(built_dash) = dash.build() {
                                    let dash_color = gpui::Hsla {
                                        a: 0.6,
                                        ..node_color
                                    };
                                    window.paint_path(built_dash, dash_color);
                                }
                            }
                        },
                    )
                    .size_full(),
                ),
        )
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .h_flex()
                .items_center()
                .gap(px(4.))
                .child(
                    Icon::new(IconName::Edit)
                        .size(IconSize::XSmall)
                        .color(Color::Warning),
                )
                .child(
                    Label::new("working")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning)
                        .weight(gpui::FontWeight::MEDIUM),
                ),
        )
        .child({
            let mut message_col = div()
                .flex_1()
                .min_w_0()
                .h_flex()
                .items_center()
                .gap(px(4.))
                .overflow_x_hidden();

            let badge_color = if has_changes {
                Color::Warning
            } else {
                Color::Muted
            };
            message_col = message_col.child(
                div()
                    .flex_shrink_0()
                    .child(Badge::new("Working Tree").color(badge_color).bold()),
            );

            if has_changes {
                if staged_count > 0 {
                    message_col = message_col.child(
                        div().flex_shrink_0().child(
                            Badge::new(SharedString::from(format!("{} staged", staged_count)))
                                .color(Color::Success),
                        ),
                    );
                }
                if unstaged_count > 0 {
                    message_col = message_col.child(
                        div().flex_shrink_0().child(
                            Badge::new(SharedString::from(format!("{} unstaged", unstaged_count)))
                                .color(Color::Warning),
                        ),
                    );
                }

                let mut indicators = div().h_flex().gap(px(4.)).flex_shrink_0();

                if added_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("+{added_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Success)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if modified_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("~{modified_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Accent)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if deleted_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("-{deleted_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if conflicted_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("!{conflicted_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }

                message_col = message_col.child(indicators);
            } else {
                message_col = message_col.child(
                    Label::new("Working Tree Clean")
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                );
            }

            message_col
        })
        .when(show_author_column, |el| {
            el.child(div().w(px(120.)).flex_shrink_0())
        })
        .when(show_date_column, |el| {
            el.child(div().w(px(100.)).flex_shrink_0())
        })
        .into_any_element()
}

/// Merge staged and unstaged file-kind breakdowns into a single combined map.
fn merge_breakdowns(
    staged: &HashMap<FileChangeKind, usize>,
    unstaged: &HashMap<FileChangeKind, usize>,
) -> HashMap<FileChangeKind, usize> {
    let mut combined = staged.clone();
    for (&kind, &count) in unstaged {
        *combined.entry(kind).or_insert(0) += count;
    }
    combined
}

/// Compute a breakdown of file change kinds from a list of `FileStatus` entries.
pub fn compute_breakdown(files: &[rgitui_git::FileStatus]) -> HashMap<FileChangeKind, usize> {
    let mut map = HashMap::new();
    for f in files {
        *map.entry(f.kind).or_insert(0) += 1;
    }
    map
}

fn format_relative_time(time: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*time);

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_weeks() < 4 {
        format!("{}w ago", duration.num_weeks())
    } else if duration.num_days() < 365 {
        format!("{}mo ago", duration.num_days() / 30)
    } else {
        format!("{}y ago", duration.num_days() / 365)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use rgitui_git::{FileChangeKind, FileStatus};
    use std::path::PathBuf;

    fn make_file_status(kind: FileChangeKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from("file.txt"),
            kind,
            old_path: None,
            additions: 0,
            deletions: 0,
        }
    }

    // --- compute_breakdown ---

    #[test]
    fn compute_breakdown_empty() {
        let result = compute_breakdown(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_breakdown_single_kind() {
        let files = vec![
            make_file_status(FileChangeKind::Modified),
            make_file_status(FileChangeKind::Modified),
        ];
        let result = compute_breakdown(&files);
        assert_eq!(result.get(&FileChangeKind::Modified), Some(&2));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn compute_breakdown_mixed_kinds() {
        let files = vec![
            make_file_status(FileChangeKind::Added),
            make_file_status(FileChangeKind::Modified),
            make_file_status(FileChangeKind::Deleted),
            make_file_status(FileChangeKind::Added),
        ];
        let result = compute_breakdown(&files);
        assert_eq!(result.get(&FileChangeKind::Added), Some(&2));
        assert_eq!(result.get(&FileChangeKind::Modified), Some(&1));
        assert_eq!(result.get(&FileChangeKind::Deleted), Some(&1));
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn compute_breakdown_all_kinds() {
        let kinds = [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
            FileChangeKind::Copied,
            FileChangeKind::TypeChange,
            FileChangeKind::Untracked,
            FileChangeKind::Conflicted,
        ];
        let files: Vec<FileStatus> = kinds.iter().copied().map(make_file_status).collect();
        let result = compute_breakdown(&files);
        assert_eq!(result.len(), 8);
        for kind in &kinds {
            assert_eq!(result.get(kind), Some(&1));
        }
    }

    // --- format_relative_time ---

    #[test]
    fn format_relative_time_just_now() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&now), "just now");
    }

    #[test]
    fn format_relative_time_minutes() {
        let t = Utc::now() - Duration::minutes(30);
        let result = format_relative_time(&t);
        assert_eq!(result, "30m ago");
    }

    #[test]
    fn format_relative_time_hours() {
        let t = Utc::now() - Duration::hours(5);
        let result = format_relative_time(&t);
        assert_eq!(result, "5h ago");
    }

    #[test]
    fn format_relative_time_days() {
        let t = Utc::now() - Duration::days(3);
        let result = format_relative_time(&t);
        assert_eq!(result, "3d ago");
    }

    #[test]
    fn format_relative_time_weeks() {
        let t = Utc::now() - Duration::weeks(2);
        let result = format_relative_time(&t);
        assert_eq!(result, "2w ago");
    }

    #[test]
    fn format_relative_time_months() {
        let t = Utc::now() - Duration::days(60);
        let result = format_relative_time(&t);
        assert_eq!(result, "2mo ago");
    }

    #[test]
    fn format_relative_time_years() {
        let t = Utc::now() - Duration::days(400);
        let result = format_relative_time(&t);
        assert_eq!(result, "1y ago");
    }
}
