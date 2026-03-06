use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, canvas, div, img, point, px, Bounds, ClickEvent, Context, ElementId, EventEmitter,
    ListSizingBehavior, ObjectFit, PathBuilder, Pixels, Render, SharedString,
    UniformListScrollHandle, WeakEntity, Window, uniform_list,
};
use rgitui_git::{CommitInfo, GraphEdge, GraphRow, RefLabel, compute_graph};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{AvatarCache, Badge, Label, LabelSize};

/// Events emitted by the graph view.
#[derive(Debug, Clone)]
pub enum GraphViewEvent {
    CommitSelected(git2::Oid),
}

/// The commit graph panel.
pub struct GraphView {
    commits: Arc<Vec<CommitInfo>>,
    graph_rows: Arc<Vec<GraphRow>>,
    selected_index: Option<usize>,
    row_height: f32,
    scroll_handle: UniformListScrollHandle,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl GraphView {
    pub fn new() -> Self {
        Self {
            commits: Arc::new(Vec::new()),
            graph_rows: Arc::new(Vec::new()),
            selected_index: None,
            row_height: 34.0,
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub fn set_commits(&mut self, commits: Vec<CommitInfo>, cx: &mut Context<Self>) {
        self.graph_rows = Arc::new(compute_graph(&commits));
        self.commits = Arc::new(commits);
        self.selected_index = None;
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

    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.commits.len() {
            self.selected_index = Some(index);
            if let Some(commit) = self.commits.get(index) {
                cx.emit(GraphViewEvent::CommitSelected(commit.oid));
            }
            cx.notify();
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
        let selected_bg = colors.ghost_element_selected;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;
        let panel_bg = colors.panel_background;
        let surface_bg = colors.surface_background;
        let border_color = colors.border_variant;
        // Cheap Arc clones
        let commits = self.commits.clone();
        let graph_rows = self.graph_rows.clone();
        let selected_index = self.selected_index;
        let view: WeakEntity<GraphView> = cx.weak_entity();

        let lane_width: f32 = 24.0;
        let row_height = self.row_height;

        // Header row (not virtualized — always visible)
        let header = div()
            .h_flex()
            .w_full()
            .h(px(30.))
            .px_1()
            .gap_1()
            .bg(surface_bg)
            .border_b_1()
            .border_color(border_color)
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(Label::new("Graph").size(LabelSize::XSmall).color(Color::Muted)),
            )
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(Label::new("Hash").size(LabelSize::XSmall).color(Color::Muted)),
            )
            .child(
                div()
                    .flex_1()
                    .child(Label::new("Message").size(LabelSize::XSmall).color(Color::Muted)),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .child(Label::new("Author").size(LabelSize::XSmall).color(Color::Muted)),
            )
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(Label::new("Date").size(LabelSize::XSmall).color(Color::Muted)),
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
                    let bg = if selected { selected_bg } else { panel_bg };

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

                    // Ref badges
                    let mut ref_badges: Vec<Badge> = Vec::new();
                    for r in &commit.refs {
                        let (text, color) = match r {
                            RefLabel::Head => ("HEAD".to_string(), Color::Accent),
                            RefLabel::LocalBranch(name) => (name.clone(), Color::Success),
                            RefLabel::RemoteBranch(name) => (name.clone(), Color::Info),
                            RefLabel::Tag(name) => (name.clone(), Color::Warning),
                        };
                        ref_badges.push(Badge::new(text).color(color));
                    }

                    let short_id: SharedString = commit.short_id.clone().into();
                    let summary: SharedString = commit.summary.clone().into();
                    let author: SharedString = commit.author.name.clone().into();
                    let time_str: SharedString = format_relative_time(&commit.time).into();

                    // Clone data for canvas closure
                    let edges: Vec<GraphEdge> = graph_row.edges.clone();

                    let view_clone = view.clone();

                    // Branch color tab: thin colored strip on left edge
                    let lane_tab_color = gpui::Hsla { a: 0.5, ..node_color };

                    let mut row = div()
                        .id(ElementId::NamedInteger("commit-row".into(), i as u64))
                        .h_flex()
                        .h(px(row_height))
                        .w_full()
                        .bg(bg)
                        .hover(move |s| s.bg(hover_bg))
                        .active(move |s| s.bg(active_bg))
                        .on_click(move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            view_clone.update(cx, |this, cx| {
                                this.select_index(i, cx);
                            }).ok();
                        })
                        // Color tab on left edge
                        .child(
                            div()
                                .w(px(3.))
                                .h_full()
                                .flex_shrink_0()
                                .bg(lane_tab_color),
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
                                                // Smooth S-curve: symmetric bezier with inflection at midpoint
                                                let half_y = start_y + (end_y - start_y) / 2.0;
                                                path.move_to(point(origin.x + from_x, start_y));
                                                path.cubic_bezier_to(
                                                    point(origin.x + to_x, end_y),
                                                    point(origin.x + from_x, half_y),
                                                    point(origin.x + to_x, half_y),
                                                );
                                            }

                                            if let Ok(built_path) = path.build() {
                                                window.paint_path(built_path, color);
                                            }
                                        }

                                        // 3. Draw commit dot with background ring (occludes passing lines).
                                        let r = 11.0_f32;
                                        let cx_x = origin.x + node_x_px;
                                        let cy_y = origin.y + mid_y;
                                        let steps = 48_usize;

                                        // Background ring to occlude lines passing behind the dot
                                        let ring_r = r + 2.0;
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
                                            window.paint_path(built_ring, panel_bg);
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

                                let mut avatar_container = div()
                                    .absolute()
                                    .left(px(node_x - 11.0))
                                    .top(px((row_height - 22.0) / 2.0))
                                    .w(px(22.))
                                    .h(px(22.))
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

                    // Hash
                    row = row.child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .child(
                                Label::new(short_id)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    );

                    // Ref badges
                    if !ref_badges.is_empty() {
                        let mut badges_container = div().h_flex().gap(px(4.)).flex_shrink_0().mr(px(4.));
                        for badge in ref_badges {
                            badges_container = badges_container.child(badge);
                        }
                        row = row.child(badges_container);
                    }

                    // Summary
                    row = row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Label::new(summary).size(LabelSize::Small).truncate()),
                    );

                    // Author
                    row = row.child(
                        div().w(px(120.)).flex_shrink_0().child(
                            Label::new(author)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                    );

                    // Time
                    row = row.child(
                        div().w(px(80.)).flex_shrink_0().child(
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

        div()
            .id("graph-view")
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(panel_bg)
            .child(header)
            .child(list)
            .into_any_element()
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
