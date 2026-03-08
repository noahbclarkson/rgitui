use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, canvas, div, img, point, px, Bounds, ClickEvent, Context, ElementId, EventEmitter,
    FocusHandle, KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent, ObjectFit,
    PathBuilder, Pixels, Point, Render, ScrollStrategy, SharedString, UniformListScrollHandle,
    WeakEntity, Window, uniform_list,
};
use rgitui_git::{CommitInfo, GraphEdge, GraphRow, RefLabel, compute_graph};
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
    row_height: f32,
    scroll_handle: UniformListScrollHandle,
    context_menu: Option<ContextMenuState>,
    /// Current search query text.
    search_query: String,
    /// Whether the search bar is visible.
    show_search: bool,
    /// Indices of commits that match the current search query.
    filter_matches: Vec<usize>,
    /// Index into `filter_matches` for the current highlighted match.
    current_match: usize,
    /// Focus handle for the search input.
    search_focus: FocusHandle,
    /// Cursor position within the search query (byte offset).
    search_cursor_pos: usize,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl GraphView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            commits: Arc::new(Vec::new()),
            graph_rows: Arc::new(Vec::new()),
            selected_index: None,
            row_height: 38.0,
            scroll_handle: UniformListScrollHandle::new(),
            context_menu: None,
            search_query: String::new(),
            show_search: false,
            filter_matches: Vec::new(),
            current_match: 0,
            search_focus: cx.focus_handle(),
            search_cursor_pos: 0,
        }
    }

    pub fn set_commits(&mut self, commits: Vec<CommitInfo>, cx: &mut Context<Self>) {
        self.graph_rows = Arc::new(compute_graph(&commits));
        self.commits = Arc::new(commits);
        self.selected_index = None;
        if self.show_search && !self.search_query.is_empty() {
            self.update_search_filter();
        }
        cx.notify();
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
                cx.emit(GraphViewEvent::CommitSelected(commit.oid));
            }
            cx.notify();
        }
    }

    /// Toggle the search bar visibility. Clears query when hiding.
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if !self.show_search {
            self.search_query.clear();
            self.search_cursor_pos = 0;
            self.filter_matches.clear();
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Toggle search with window access (focuses the search input).
    pub fn toggle_search_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if self.show_search {
            self.search_focus.focus(window, cx);
        } else {
            self.search_query.clear();
            self.search_cursor_pos = 0;
            self.filter_matches.clear();
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Returns whether the search bar is currently visible.
    pub fn is_search_visible(&self) -> bool {
        self.show_search
    }

    /// Get the byte index of the start of the character before `pos`.
    fn prev_char_boundary(s: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut p = pos - 1;
        while p > 0 && !s.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// Get the byte index of the start of the character after `pos`.
    fn next_char_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() {
            return s.len();
        }
        let mut p = pos + 1;
        while p < s.len() && !s.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    /// Update the filter_matches list based on the current search_query.
    /// Matches case-insensitively against commit message, author name, author email, and short SHA.
    fn update_search_filter(&mut self) {
        self.filter_matches.clear();
        self.current_match = 0;

        if self.search_query.is_empty() {
            return;
        }

        let query = self.search_query.to_lowercase();
        for (i, commit) in self.commits.iter().enumerate() {
            if commit.summary.to_lowercase().contains(&query)
                || commit.message.to_lowercase().contains(&query)
                || commit.author.name.to_lowercase().contains(&query)
                || commit.author.email.to_lowercase().contains(&query)
                || commit.short_id.to_lowercase().contains(&query)
            {
                self.filter_matches.push(i);
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
        self.scroll_handle.scroll_to_item(index, ScrollStrategy::Top);
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
        self.scroll_handle.scroll_to_item(index, ScrollStrategy::Top);
    }

    /// Jump to first match after updating the search filter.
    fn jump_to_first_match(&mut self, cx: &mut Context<Self>) {
        if !self.filter_matches.is_empty() {
            self.current_match = 0;
            let index = self.filter_matches[0];
            self.select_index(index, cx);
            self.scroll_handle.scroll_to_item(index, ScrollStrategy::Top);
        }
    }

    /// Handle key events on the focused search input.
    fn handle_search_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        // Ctrl shortcuts
        if ctrl {
            match key {
                "a" => {
                    self.search_cursor_pos = self.search_query.len();
                    cx.notify();
                    return;
                }
                "v" => {
                    if let Some(clipboard) = cx.read_from_clipboard() {
                        if let Some(text) = clipboard.text() {
                            // Only take the first line for search
                            let line = text.lines().next().unwrap_or("");
                            self.search_query.insert_str(self.search_cursor_pos, line);
                            self.search_cursor_pos += line.len();
                            self.update_search_filter();
                            self.jump_to_first_match(cx);
                            cx.notify();
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        match key {
            "escape" => {
                self.show_search = false;
                self.search_query.clear();
                self.search_cursor_pos = 0;
                self.filter_matches.clear();
                self.current_match = 0;
                cx.notify();
            }
            "enter" => {
                if keystroke.modifiers.shift {
                    self.jump_to_prev_match(cx);
                } else {
                    self.jump_to_next_match(cx);
                }
            }
            "backspace" => {
                if self.search_cursor_pos > 0 {
                    let prev = Self::prev_char_boundary(&self.search_query, self.search_cursor_pos);
                    self.search_query.drain(prev..self.search_cursor_pos);
                    self.search_cursor_pos = prev;
                    self.update_search_filter();
                    self.jump_to_first_match(cx);
                    cx.notify();
                }
            }
            "delete" => {
                if self.search_cursor_pos < self.search_query.len() {
                    let next = Self::next_char_boundary(&self.search_query, self.search_cursor_pos);
                    self.search_query.drain(self.search_cursor_pos..next);
                    self.update_search_filter();
                    self.jump_to_first_match(cx);
                    cx.notify();
                }
            }
            "left" => {
                if self.search_cursor_pos > 0 {
                    self.search_cursor_pos = Self::prev_char_boundary(&self.search_query, self.search_cursor_pos);
                    cx.notify();
                }
            }
            "right" => {
                if self.search_cursor_pos < self.search_query.len() {
                    self.search_cursor_pos = Self::next_char_boundary(&self.search_query, self.search_cursor_pos);
                    cx.notify();
                }
            }
            "home" => {
                self.search_cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.search_cursor_pos = self.search_query.len();
                cx.notify();
            }
            _ => {
                // Insert printable characters
                if !ctrl {
                    if let Some(key_char) = &keystroke.key_char {
                        self.search_query.insert_str(self.search_cursor_pos, key_char);
                        self.search_cursor_pos += key_char.len();
                        self.update_search_filter();
                        self.jump_to_first_match(cx);
                        cx.notify();
                    } else if key.len() == 1 {
                        let ch = key.chars().next().unwrap();
                        if ch.is_ascii_graphic() || ch == ' ' {
                            self.search_query.insert(self.search_cursor_pos, ch);
                            self.search_cursor_pos += ch.len_utf8();
                            self.update_search_filter();
                            self.jump_to_first_match(cx);
                            cx.notify();
                        }
                    }
                }
            }
        }
    }
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.commits.is_empty() {
            return div()
                .id("graph-view")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(colors.panel_background)
                .child(
                    Label::new("No commits to display")
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
                .into_any_element();
        }

        // Extract colors before closure (can't call cx inside uniform_list closure)
        let selected_bg = colors.element_selected;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;
        let panel_bg = colors.panel_background;
        let surface_bg = colors.surface_background;
        let border_color = colors.border_variant;
        let accent_border = colors.text_accent;
        // Subtle zebra-stripe: derive from surface bg with tiny alpha shift
        let zebra_bg = gpui::Hsla {
            l: (surface_bg.l - 0.02).max(0.0),
            a: 0.5,
            ..surface_bg
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

        // Cheap Arc clones
        let commits = self.commits.clone();
        let graph_rows = self.graph_rows.clone();
        let selected_index = self.selected_index;
        let view: WeakEntity<GraphView> = cx.weak_entity();

        // Search state for the render closure
        let filter_matches: Arc<Vec<usize>> = Arc::new(self.filter_matches.clone());
        let current_match_index = if self.filter_matches.is_empty() {
            None
        } else {
            Some(self.filter_matches[self.current_match])
        };
        let _show_search = self.show_search;
        let has_search_query = self.show_search && !self.search_query.is_empty();

        let lane_width: f32 = 24.0;
        let row_height = self.row_height;

        // Header row (not virtualized — always visible)
        let header = div()
            .h_flex()
            .items_center()
            .w_full()
            .h(px(28.))
            .pl(px(12.))
            .pr(px(8.))
            .gap_1()
            .bg(surface_bg)
            .border_b_1()
            .border_color(border_color)
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(Icon::new(IconName::GitCommit).size(IconSize::XSmall).color(Color::Muted))
                    .child(
                        Label::new("Graph")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            )
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(
                        Label::new("Hash")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .child(
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
                    .child(Icon::new(IconName::User).size(IconSize::XSmall).color(Color::Muted))
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
                    .child(Icon::new(IconName::Clock).size(IconSize::XSmall).color(Color::Muted))
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
                range.map(|i| {
                    let commit = commits[i].clone();
                    let graph_row = graph_rows[i].clone();
                    let selected = selected_index == Some(i);
                    let is_current_match = current_match_index == Some(i);
                    let is_any_match = has_search_query && filter_matches.contains(&i);
                    // Alternating row background for zebra-stripe effect
                    let row_base_bg = if i % 2 == 1 { zebra_bg } else { panel_bg };
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
                            RefLabel::Head => Badge::new("HEAD")
                                .color(Color::Accent)
                                .bold(),
                            RefLabel::LocalBranch(name) => Badge::new(name.clone())
                                .color(Color::Success),
                            RefLabel::RemoteBranch(name) => Badge::new(name.clone())
                                .color(Color::Info)
                                .italic(),
                            RefLabel::Tag(name) => Badge::new(name.clone())
                                .color(Color::Warning)
                                .prefix("tag"),
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
                    } else {
                        gpui::Hsla { a: 0.5, ..node_color }
                    };

                    let mut row = div()
                        .id(ElementId::NamedInteger("commit-row".into(), i as u64))
                        .h_flex()
                        .items_center()
                        .h(px(row_height))
                        .w_full()
                        .bg(bg)
                        .border_l_2()
                        .border_color(if selected { accent_border } else { gpui::Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 } })
                        .cursor_pointer()
                        .hover(move |s| s.bg(row_hover_bg))
                        .active(move |s| s.bg(row_active_bg))
                        .on_click(move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            view_clone.update(cx, |this, cx| {
                                this.dismiss_context_menu(cx);
                                this.select_index(i, cx);
                            }).ok();
                        })
                        .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
                            view_ctx_menu.update(cx, |this, cx| {
                                this.show_context_menu(i, event.position, cx);
                            }).ok();
                        })
                        // Color tab on left edge (after the border)
                        .child(
                            div()
                                .w(px(3.))
                                .h_full()
                                .flex_shrink_0()
                                .bg(left_tab_color),
                        )
                        // Gap between color tab and graph
                        .child(div().w(px(4.)).flex_shrink_0());

                    // Graph column with canvas + avatar overlay
                    row = row.child(
                        div()
                            .relative()
                            .w(px(graph_width))
                            .flex_shrink_0()
                            .h_full()
                            .child(
                                canvas(
                                    |_bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut App| {},
                                    move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _cx: &mut App| {
                                        let origin = bounds.origin;
                                        let h = bounds.size.height;
                                        let mid_y = px(row_height / 2.0);
                                        let node_x_px = px(node_x);

                                        // 1. Approach segment: incoming line from row above → dot center.
                                        //    Only drawn if this commit was expected (not a branch tip).
                                        if has_incoming {
                                            let mut approach = PathBuilder::stroke(px(2.5));
                                            approach.move_to(point(origin.x + node_x_px, origin.y));
                                            approach.line_to(point(origin.x + node_x_px, origin.y + mid_y));
                                            if let Ok(built) = approach.build() {
                                                window.paint_path(built, node_color);
                                            }
                                        }

                                        // 2. Draw all edges (pass-throughs + outgoing from dot).
                                        for edge in &edges {
                                            let from_x = px(edge.from_lane as f32 * lane_width + lane_width / 2.0);
                                            let to_x = px(edge.to_lane as f32 * lane_width + lane_width / 2.0);
                                            let color = rgitui_theme::lane_color(edge.color_index);

                                            // Edges departing from the commit dot start at mid_y;
                                            // pass-through edges span the full row height.
                                            let start_y = if edge.from_lane == node_lane {
                                                origin.y + mid_y
                                            } else {
                                                origin.y
                                            };
                                            let end_y = origin.y + h;

                                            let stroke_width = if edge.is_merge { px(1.5) } else { px(2.5) };
                                            let mut path = PathBuilder::stroke(stroke_width);

                                            if from_x == to_x {
                                                // Straight vertical line
                                                path.move_to(point(origin.x + from_x, start_y));
                                                path.line_to(point(origin.x + to_x, end_y));
                                            } else {
                                                // Smooth S-curve: control points at 40%/60% of
                                                // vertical span, keeping source/dest X coords.
                                                // This departs vertically, transitions smoothly,
                                                // and arrives vertically — a natural branch/merge curve.
                                                let span = end_y - start_y;
                                                let ctrl_y1 = start_y + span * 0.4;
                                                let ctrl_y2 = start_y + span * 0.6;
                                                path.move_to(point(origin.x + from_x, start_y));
                                                path.cubic_bezier_to(
                                                    point(origin.x + to_x, end_y),
                                                    point(origin.x + from_x, ctrl_y1),
                                                    point(origin.x + to_x, ctrl_y2),
                                                );
                                            }

                                            if let Ok(built_path) = path.build() {
                                                window.paint_path(built_path, color);
                                            }
                                        }

                                        // 3. Draw commit dot with background ring (occludes passing lines).
                                        let r = 14.0_f32;
                                        let cx_x = origin.x + node_x_px;
                                        let cy_y = origin.y + mid_y;
                                        let steps = 32_usize;

                                        // Background ring to occlude lines passing behind the dot
                                        // Use the actual row background color so it blends correctly
                                        let ring_r = r + 3.0;
                                        let mut ring = PathBuilder::fill();
                                        for s in 0..steps {
                                            let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                                            let x = cx_x + px(ring_r * angle.cos());
                                            let y = cy_y + px(ring_r * angle.sin());
                                            if s == 0 { ring.move_to(point(x, y)); }
                                            else { ring.line_to(point(x, y)); }
                                        }
                                        ring.close();
                                        if let Ok(built_ring) = ring.build() {
                                            window.paint_path(built_ring, row_bg_for_canvas);
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

                                let avatar_size = 28.0_f32;
                                let mut avatar_container = div()
                                    .absolute()
                                    .left(px(node_x - avatar_size / 2.0))
                                    .top(px((row_height - avatar_size) / 2.0))
                                    .w(px(avatar_size))
                                    .h(px(avatar_size))
                                    .rounded_full()
                                    .bg(panel_bg)
                                    .border_2()
                                    .border_color(node_color)
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
                                                            .font_weight(gpui::FontWeight::BOLD)
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
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .child(
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

                        // Summary text
                        message_col = message_col.child(
                            Label::new(summary)
                                .size(LabelSize::Small)
                                .truncate(),
                        );

                        row = row.child(message_col);
                    }

                    // Author column
                    row = row.child(
                        div()
                            .w(px(120.))
                            .flex_shrink_0()
                            .px(px(4.))
                            .child(
                                Label::new(author)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    );

                    // Date column (relative time)
                    row = row.child(
                        div()
                            .w(px(100.))
                            .flex_shrink_0()
                            .pr(px(8.))
                            .child(
                                Label::new(time_str)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    );

                    row.into_any_element()
                }).collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        let mut container = div()
            .id("graph-view")
            .relative()
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(panel_bg)
            .on_mouse_down(MouseButton::Left, {
                let view_dismiss = cx.weak_entity();
                move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    view_dismiss.update(cx, |this: &mut GraphView, cx| {
                        this.dismiss_context_menu(cx);
                    }).ok();
                }
            })
            .child(header);

        // Search bar (shown when search is active)
        if self.show_search {
            let is_search_focused = self.search_focus.is_focused(_window);
            let query_display: SharedString = if self.search_query.is_empty() {
                if is_search_focused {
                    "|".into()
                } else {
                    "Type to search...".into()
                }
            } else if is_search_focused {
                let mut display = self.search_query.clone();
                let pos = self.search_cursor_pos.min(display.len());
                display.insert(pos, '|');
                display.into()
            } else {
                self.search_query.clone().into()
            };
            let match_count_text: SharedString = if self.search_query.is_empty() {
                String::new().into()
            } else if self.filter_matches.is_empty() {
                "No matches".into()
            } else {
                format!(
                    "{}/{}",
                    self.current_match + 1,
                    self.filter_matches.len()
                )
                .into()
            };
            let query_color = if self.search_query.is_empty() && !is_search_focused {
                Color::Placeholder
            } else {
                Color::Default
            };

            let view_prev = cx.weak_entity();
            let view_next = cx.weak_entity();
            let has_matches = !self.filter_matches.is_empty();

            let search_bar = div()
                .id("search-bar-input")
                .track_focus(&self.search_focus)
                .on_key_down(cx.listener(Self::handle_search_key_down))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.search_focus.focus(window, cx);
                    cx.notify();
                }))
                .h_flex()
                .items_center()
                .w_full()
                .h(px(32.))
                .px_2()
                .gap_2()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_focused)
                .cursor_text()
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .flex_1()
                        .child(
                            Label::new(query_display)
                                .size(LabelSize::Small)
                                .color(query_color),
                        ),
                )
                .child(
                    Label::new(match_count_text)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .id("search-prev")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .hover(move |s| s.bg(if has_matches { hover_bg } else { gpui::Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 } }))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_prev.update(cx, |this: &mut GraphView, cx| {
                                this.jump_to_prev_match(cx);
                            }).ok();
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
                        .hover(move |s| s.bg(if has_matches { active_bg } else { gpui::Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 } }))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_next.update(cx, |this: &mut GraphView, cx| {
                                this.jump_to_next_match(cx);
                            }).ok();
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
                    ("Copy SHA", IconName::Copy),
                ];

                let mut menu = div()
                    .id("graph-context-menu")
                    .absolute()
                    .left(pos.x)
                    .top(pos.y)
                    .v_flex()
                    .min_w(px(200.))
                    .py(px(4.))
                    .bg(menu_bg)
                    .border_1()
                    .border_color(menu_border)
                    .rounded_md()
                    .elevation_3(cx)
                    // Prevent left-click on menu from dismissing via container handler
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    // Prevent right-click on menu from opening another menu
                    .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    });

                for (idx, (label_text, icon_name)) in menu_items.iter().enumerate() {
                    let label: SharedString = (*label_text).into();
                    let icon = *icon_name;

                    // Add separator before "Copy SHA" (last item)
                    if idx == 4 {
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
                        .h(px(30.))
                        .px(px(8.))
                        .mx(px(4.))
                        .gap(px(8.))
                        .items_center()
                        .cursor_pointer()
                        .rounded_sm()
                        .hover(move |s| s.bg(menu_hover))
                        .active(move |s| s.bg(menu_active));

                    match idx {
                        0 => {
                            let w = weak.clone();
                            item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                w.update(cx, |this: &mut GraphView, cx| {
                                    this.context_menu = None;
                                    cx.emit(GraphViewEvent::CherryPick(oid));
                                    cx.notify();
                                }).ok();
                            });
                        }
                        1 => {
                            let w = weak.clone();
                            item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                w.update(cx, |this: &mut GraphView, cx| {
                                    this.context_menu = None;
                                    cx.emit(GraphViewEvent::RevertCommit(oid));
                                    cx.notify();
                                }).ok();
                            });
                        }
                        2 => {
                            let w = weak.clone();
                            item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                w.update(cx, |this: &mut GraphView, cx| {
                                    this.context_menu = None;
                                    cx.emit(GraphViewEvent::CheckoutCommit(oid));
                                    cx.notify();
                                }).ok();
                            });
                        }
                        3 => {
                            let w = weak.clone();
                            item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                w.update(cx, |this: &mut GraphView, cx| {
                                    this.context_menu = None;
                                    cx.emit(GraphViewEvent::CreateBranchAtCommit(oid));
                                    cx.notify();
                                }).ok();
                            });
                        }
                        4 => {
                            let w = weak.clone();
                            let sha_for_click = sha_clone.clone();
                            item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                let sha_val = sha_for_click.clone();
                                w.update(cx, |this: &mut GraphView, cx| {
                                    this.context_menu = None;
                                    cx.emit(GraphViewEvent::CopyCommitSha(sha_val));
                                    cx.notify();
                                }).ok();
                            });
                        }
                        _ => {}
                    }

                    item = item
                        .child(
                            Icon::new(icon)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(label)
                                .size(LabelSize::Small)
                                .color(Color::Default),
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
