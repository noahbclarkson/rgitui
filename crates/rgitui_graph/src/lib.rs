use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, point, px, uniform_list, App, Bounds, ClickEvent, Context, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, ListSizingBehavior, MouseButton,
    MouseDownEvent, ObjectFit, PathBuilder, Pixels, Point, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{compute_graph, CommitInfo, GraphEdge, GraphRow, RefLabel};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{AvatarCache, Badge, Icon, IconName, IconSize, Label, LabelSize};

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
    selected_index: Option<usize>,
    selected_oid: Option<git2::Oid>,
    row_height: f32,
    scroll_handle: UniformListScrollHandle,
    context_menu: Option<ContextMenuState>,
    show_search: bool,
    filter_matches: Vec<usize>,
    filter_match_set: HashSet<usize>,
    current_match: usize,
    search_editor: Entity<rgitui_ui::TextInput>,
    graph_focus: FocusHandle,
    all_commits_loaded: bool,
    cached_graph_hash: u64,
    search_debounce_task: Option<gpui::Task<()>>,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl GraphView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_editor = cx.new(|cx| {
            let mut ti = rgitui_ui::TextInput::new(cx);
            ti.set_placeholder("Search commits...");
            ti
        });

        cx.subscribe(&search_editor, |this: &mut Self, _, event: &rgitui_ui::TextInputEvent, cx| {
            match event {
                rgitui_ui::TextInputEvent::Changed(_) => {
                    this.schedule_search_filter(cx);
                }
                rgitui_ui::TextInputEvent::Submit => {
                    this.jump_to_next_match(cx);
                }
            }
        }).detach();

        Self {
            commits: Arc::new(Vec::new()),
            graph_rows: Arc::new(Vec::new()),
            selected_index: None,
            selected_oid: None,
            row_height: 32.0,
            scroll_handle: UniformListScrollHandle::new(),
            context_menu: None,
            show_search: false,
            filter_matches: Vec::new(),
            filter_match_set: HashSet::new(),
            current_match: 0,
            search_editor,
            graph_focus: cx.focus_handle(),
            all_commits_loaded: false,
            cached_graph_hash: 0,
            search_debounce_task: None,
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

    pub fn set_commits(&mut self, commits: Vec<CommitInfo>, cx: &mut Context<Self>) {
        // Compute a simple hash to detect if commits actually changed
        let new_hash = Self::compute_commits_hash(&commits);
        if new_hash == self.cached_graph_hash && !self.commits.is_empty() {
            // Commits haven't changed — skip recomputation
            return;
        }
        self.cached_graph_hash = new_hash;

        // Preserve selection by OID across refreshes
        let prev_selected_oid = self.selected_oid;

        self.graph_rows = Arc::new(compute_graph(&commits));
        self.commits = Arc::new(commits);

        // Restore selection if the previously selected commit still exists
        if let Some(prev_oid) = prev_selected_oid {
            if let Some(new_index) = self.commits.iter().position(|c| c.oid == prev_oid) {
                self.selected_index = Some(new_index);
                // Don't emit CommitSelected — the selection is unchanged
            } else {
                self.selected_index = None;
                self.selected_oid = None;
            }
        } else {
            self.selected_index = None;
        }

        if self.show_search && !self.search_editor.read(cx).is_empty() {
            self.update_search_filter(cx);
        }
        cx.notify();
    }

    /// Mark that all available commits have been loaded (disables "load more").
    pub fn set_all_loaded(&mut self, loaded: bool) {
        self.all_commits_loaded = loaded;
    }

    pub fn selected_commit(&self) -> Option<&CommitInfo> {
        self.selected_index.and_then(|i| self.commits.get(i))
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn commit_count(&self) -> usize {
        self.commits.len()
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
        if let Some(index) = self.commits.iter().position(|c| c.oid == oid) {
            self.select_index(index, cx);
            self.scroll_handle
                .scroll_to_item(index, ScrollStrategy::Top);
        }
    }

    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.commits.len() {
            self.selected_index = Some(index);
            if let Some(commit) = self.commits.get(index) {
                self.selected_oid = Some(commit.oid);
                cx.emit(GraphViewEvent::CommitSelected(commit.oid));
            }
            cx.notify();
        }
    }

    /// Toggle the search bar visibility. Clears query when hiding.
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if !self.show_search {
            self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Toggle search with window access (focuses the search input).
    pub fn toggle_search_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if self.show_search {
            self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
        } else {
            self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
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
    }

    /// Jump to the next search match, selecting and scrolling to it.
    fn jump_to_next_match(&mut self, cx: &mut Context<Self>) {
        if self.filter_matches.is_empty() {
            return;
        }
        self.current_match = (self.current_match + 1) % self.filter_matches.len();
        let index = self.filter_matches[self.current_match];
        self.select_index(index, cx);
        self.scroll_handle
            .scroll_to_item(index, ScrollStrategy::Top);
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
        let index = self.filter_matches[self.current_match];
        self.select_index(index, cx);
        self.scroll_handle
            .scroll_to_item(index, ScrollStrategy::Top);
    }

    /// Jump to first match after updating the search filter.
    fn jump_to_first_match(&mut self, cx: &mut Context<Self>) {
        if !self.filter_matches.is_empty() {
            self.current_match = 0;
            let index = self.filter_matches[0];
            self.select_index(index, cx);
            self.scroll_handle
                .scroll_to_item(index, ScrollStrategy::Top);
        }
    }

    /// Handle key events on the focused search input.
    fn handle_graph_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_editor.read(cx).focus_handle(cx).is_focused(window) {
            if event.keystroke.key.as_str() == "escape" {
                self.show_search = false;
                self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
                self.filter_matches.clear();
                self.filter_match_set.clear();
                self.current_match = 0;
                self.graph_focus.focus(window, cx);
                cx.notify();
            }
            return;
        }
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        match key {
            "j" | "down" if !ctrl => {
                // Move selection down
                let next = match self.selected_index {
                    Some(i) if i + 1 < self.commits.len() => i + 1,
                    None if !self.commits.is_empty() => 0,
                    _ => return,
                };
                self.select_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "k" | "up" if !ctrl => {
                // Move selection up
                let next = match self.selected_index {
                    Some(i) if i > 0 => i - 1,
                    None if !self.commits.is_empty() => 0,
                    _ => return,
                };
                self.select_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "g" if !ctrl && !keystroke.modifiers.shift => {
                // Jump to first commit
                if !self.commits.is_empty() {
                    self.select_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            }
            "g" if keystroke.modifiers.shift => {
                // Shift+G: jump to last commit (vim-style)
                if !self.commits.is_empty() {
                    let last = self.commits.len() - 1;
                    self.select_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            }
            "end" => {
                // Jump to last commit
                if !self.commits.is_empty() {
                    let last = self.commits.len() - 1;
                    self.select_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            }
            "home" => {
                if !self.commits.is_empty() {
                    self.select_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            }
            "/" if !ctrl => {
                self.show_search = true;
                self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
                cx.notify();
            }
            "escape" => {
                // Dismiss context menu or deselect
                if self.context_menu.is_some() {
                    self.dismiss_context_menu(cx);
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
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
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
        let selected_bg = colors.ghost_element_selected;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_selected;
        let panel_bg = colors.panel_background;
        let border_color = colors.border_variant;
        let accent_border = colors.text_accent;
        // Subtle row separator color (very faint border between rows)
        let row_separator = gpui::Hsla {
            a: 0.06,
            ..colors.border_variant
        };

        // Search highlight colors
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
            a: 0.12,
            ..colors.text_accent
        };

        // Cheap Arc clones
        let commits = self.commits.clone();
        let graph_rows = self.graph_rows.clone();
        let selected_index = self.selected_index;
        let view: WeakEntity<GraphView> = cx.weak_entity();

        // Search state for the render closure — use HashSet for O(1) lookup
        let filter_match_set: Arc<HashSet<usize>> = Arc::new(self.filter_match_set.clone());
        let current_match_index = if self.filter_matches.is_empty() {
            None
        } else {
            Some(self.filter_matches[self.current_match])
        };
        let has_search_query = self.show_search && !self.search_editor.read(cx).is_empty();

        let lane_width: f32 = 20.0;
        let row_height = self.row_height;

        // Header row (not virtualized — always visible)
        let header = div()
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
                    .w(px(80.))
                    .flex_shrink_0()
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
            )
            .child(
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
            )
            .child(
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

        // The virtualized list
        let list = uniform_list(
            "graph-commit-list",
            commits.len(),
            move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                range
                    .map(|i| {
                        let commit = commits[i].clone();
                        let graph_row = graph_rows[i].clone();
                        let selected = selected_index == Some(i);
                        let is_current_match = current_match_index == Some(i);
                        let is_any_match = has_search_query && filter_match_set.contains(&i);
                        let is_head_row = graph_row.is_head;
                        let is_merge_commit = graph_row.is_merge;

                        // Row background priority: selected > search current > search match > head > default
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
                        // Don't show hover effect on already-selected rows
                        let row_hover_bg = if selected { selected_bg } else { hover_bg };
                        let row_active_bg = if selected { selected_bg } else { active_bg };

                        let max_lane = graph_row.lane_count.max(1);
                        let graph_width = (max_lane as f32 * lane_width).max(80.0);
                        let node_lane = graph_row.node_lane;
                        let node_x = node_lane as f32 * lane_width + lane_width / 2.0;

                        let node_color = rgitui_theme::lane_color(graph_row.node_color);
                        let has_incoming = graph_row.has_incoming;

                        // Author initials for avatar
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

                        let short_id: SharedString = commit.short_id.clone().into();
                        let summary: SharedString = commit.summary.clone().into();
                        let author: SharedString = commit.author.name.clone().into();
                        let time_str: SharedString = format_relative_time(&commit.time).into();

                        // Clone data for canvas closure
                        let edges: Vec<GraphEdge> = graph_row.edges.clone();
                        let row_bg_for_canvas = bg;

                        let view_clone = view.clone();
                        let view_ctx_menu = view.clone();

                        // Left edge indicator: accent border when selected, lane color otherwise
                        let left_tab_color = if selected {
                            accent_border
                        } else if is_head_row {
                            // HEAD row gets a brighter accent tab
                            gpui::Hsla {
                                a: 0.8,
                                ..accent_border
                            }
                        } else {
                            gpui::Hsla {
                                a: 0.5,
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
                            .border_b_1()
                            .border_color(row_separator)
                            .cursor_pointer()
                            .hover(move |s| s.bg(row_hover_bg))
                            .active(move |s| s.bg(row_active_bg))
                            .on_click(
                                move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view_clone
                                        .update(cx, |this, cx| {
                                            this.dismiss_context_menu(cx);
                                            this.select_index(i, cx);
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
                                            this.show_context_menu(i, event.position, cx);
                                        })
                                        .ok();
                                },
                            )
                            // Color tab on left edge — accent for selected, lane color otherwise
                            .child(
                                div()
                                    .w(px(3.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .bg(left_tab_color),
                            )
                            // Gap between color tab and graph
                            .child(div().w(px(5.)).flex_shrink_0());

                        // Graph column with canvas + avatar overlay
                        row = row.child(
                            div()
                                .relative()
                                .w(px(graph_width))
                                .flex_shrink_0()
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
                                                    origin.y - px(1.0),
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
                                                        + lane_width / 2.0,
                                                );
                                                let to_x = px(
                                                    edge.to_lane as f32 * lane_width
                                                        + lane_width / 2.0,
                                                );
                                                let color =
                                                    rgitui_theme::lane_color(edge.color_index);

                                                let start_y = if edge.from_lane == node_lane {
                                                    origin.y + mid_y
                                                } else {
                                                    origin.y - px(1.0)
                                                };
                                                let end_y = origin.y + h + px(1.0);

                                                let stroke_width =
                                                    if edge.is_merge { px(1.0) } else { px(2.0) };

                                                // Merge edges use dashed appearance via slightly
                                                // transparent color
                                                let edge_color = if edge.is_merge {
                                                    gpui::Hsla { a: 0.7, ..color }
                                                } else {
                                                    color
                                                };

                                                let mut path = PathBuilder::stroke(stroke_width);

                                                if from_x == to_x {
                                                    // Straight vertical line
                                                    path.move_to(point(
                                                        origin.x + from_x,
                                                        start_y,
                                                    ));
                                                    path.line_to(point(
                                                        origin.x + to_x,
                                                        end_y,
                                                    ));
                                                } else {
                                                    // Smooth S-curve with cubic bezier
                                                    let span = end_y - start_y;
                                                    let ctrl_y1 = start_y + span * 0.4;
                                                    let ctrl_y2 = start_y + span * 0.6;
                                                    path.move_to(point(
                                                        origin.x + from_x,
                                                        start_y,
                                                    ));
                                                    path.cubic_bezier_to(
                                                        point(origin.x + to_x, end_y),
                                                        point(origin.x + from_x, ctrl_y1),
                                                        point(origin.x + to_x, ctrl_y2),
                                                    );
                                                }

                                                if let Ok(built_path) = path.build() {
                                                    window.paint_path(built_path, edge_color);
                                                }
                                            }

                                            // 3. Draw commit dot with background ring.
                                            let dot_radius = if is_merge_commit {
                                                5.5_f32
                                            } else {
                                                4.5_f32
                                            };
                                            let cx_x = origin.x + node_x_px;
                                            let cy_y = origin.y + mid_y;
                                            let steps = 32_usize;

                                            // Background ring to occlude lines passing behind the dot
                                            let ring_r = 13.0_f32;
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

                                            // For HEAD commit, draw a subtle glow ring
                                            if is_head_row {
                                                let glow_r = dot_radius + 4.0;
                                                let mut glow = PathBuilder::stroke(px(2.0));
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
                                                        a: 0.3,
                                                        ..node_color
                                                    };
                                                    window.paint_path(built_glow, glow_color);
                                                }
                                            }

                                            // For merge commits, draw a slightly larger outer ring
                                            if is_merge_commit {
                                                let merge_r = dot_radius + 1.5;
                                                let mut merge_ring =
                                                    PathBuilder::stroke(px(1.5));
                                                for s in 0..=steps {
                                                    let angle = (s as f32)
                                                        * std::f32::consts::TAU
                                                        / (steps as f32);
                                                    let x =
                                                        cx_x + px(merge_r * angle.cos());
                                                    let y =
                                                        cy_y + px(merge_r * angle.sin());
                                                    if s == 0 {
                                                        merge_ring.move_to(point(x, y));
                                                    } else {
                                                        merge_ring.line_to(point(x, y));
                                                    }
                                                }
                                                if let Ok(built_merge) = merge_ring.build() {
                                                    let merge_ring_color = gpui::Hsla {
                                                        a: 0.5,
                                                        ..node_color
                                                    };
                                                    window.paint_path(
                                                        built_merge,
                                                        merge_ring_color,
                                                    );
                                                }
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                // Avatar overlay: resolved image or initials fallback
                                .child({
                                    let avatar_url = cx
                                        .try_global::<AvatarCache>()
                                        .and_then(|cache| cache.avatar_url(&commit.author.email))
                                        .map(|s| s.to_string());
                                    let initials_fallback = initials.clone();
                                    let fallback_color = node_color;

                                    let avatar_size = 24.0_f32;
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
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .text_color(fb_color)
                                                                .text_xs()
                                                                .font_weight(
                                                                    gpui::FontWeight::BOLD,
                                                                )
                                                                .child(fb_initials.clone()),
                                                        )
                                                        .into_any_element()
                                                }),
                                        );
                                    } else {
                                        avatar_container = avatar_container.child(
                                            div()
                                                .text_color(fallback_color)
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(initials_fallback),
                                        );
                                    }

                                    avatar_container
                                }),
                        );

                        // Hash column
                        row = row.child(
                            div().w(px(80.)).flex_shrink_0().child(
                                Label::new(short_id)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Accent)
                                    .weight(gpui::FontWeight::MEDIUM),
                            ),
                        );

                        // Message column — contains ref badges (inline) + summary text
                        {
                            let mut message_col = div()
                                .flex_1()
                                .min_w_0()
                                .h_flex()
                                .items_center()
                                .gap(px(5.))
                                .overflow_x_hidden();

                            // Ref badges inline before the summary
                            if !ref_badges.is_empty() {
                                let mut badges_container = div()
                                    .h_flex()
                                    .gap(px(3.))
                                    .flex_shrink_0()
                                    .max_w(px(300.))
                                    .overflow_x_hidden();
                                for badge in ref_badges {
                                    badges_container = badges_container.child(badge);
                                }
                                message_col = message_col.child(badges_container);
                            }

                            // Summary text — HEAD commits get slightly bolder text
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

                        // Author column
                        row = row.child(
                            div().w(px(120.)).flex_shrink_0().px(px(4.)).child(
                                Label::new(author)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                        );

                        // Date column (relative time)
                        row = row.child(
                            div().w(px(100.)).flex_shrink_0().pr(px(8.)).child(
                                Label::new(time_str)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                        );

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
            .on_key_down(cx.listener(Self::handle_graph_key_down))
            .relative()
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(panel_bg)
            .on_mouse_down(MouseButton::Left, {
                let view_dismiss = cx.weak_entity();
                move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    view_dismiss
                        .update(cx, |this: &mut GraphView, cx| {
                            this.dismiss_context_menu(cx);
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
            let no_matches = !self.search_editor.read(cx).is_empty() && self.filter_matches.is_empty();
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
                        .hover(move |s| {
                            s.bg(if has_matches {
                                hover_bg
                            } else {
                                gpui::Hsla {
                                    h: 0.0,
                                    s: 0.0,
                                    l: 0.0,
                                    a: 0.0,
                                }
                            })
                        })
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_prev
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.jump_to_prev_match(cx);
                                })
                                .ok();
                        })
                        .child(
                            Icon::new(IconName::ChevronUp)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("search-next")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .hover(move |s| {
                            s.bg(if has_matches {
                                active_bg
                            } else {
                                gpui::Hsla {
                                    h: 0.0,
                                    s: 0.0,
                                    l: 0.0,
                                    a: 0.0,
                                }
                            })
                        })
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
                                .color(Color::Muted),
                        ),
                );

            container = container.child(search_bar);
        }

        container = container.child(list);

        // "Load more" footer when there are more commits available
        if !self.all_commits_loaded && !self.commits.is_empty() {
            let view_load = cx.weak_entity();
            container = container.child(
                div()
                    .id("load-more-row")
                    .h_flex()
                    .w_full()
                    .h(px(32.))
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .bg(colors.surface_background)
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .cursor_pointer()
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

                    // Add separator before destructive "Reset" and before "Copy SHA"
                    if idx == 5 || idx == 6 {
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
                        6 => {
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

                    // Destructive actions render in error color
                    let item_color = if idx == 5 { Color::Error } else { Color::Muted };
                    let label_color = if idx == 5 {
                        Color::Error
                    } else {
                        Color::Default
                    };

                    item = item
                        .child(Icon::new(icon).size(IconSize::XSmall).color(item_color))
                        .child(
                            Label::new(label)
                                .size(LabelSize::XSmall)
                                .color(label_color),
                        );

                    menu = menu.child(item);
                }

                container = container.child(menu);
            }
        }

        container.into_any_element()
    }
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
