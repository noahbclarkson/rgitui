use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent, Render, ScrollStrategy,
    SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::BlameLine;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

/// Events emitted by the blame view.
#[derive(Debug, Clone)]
pub enum BlameViewEvent {
    CommitSelected(String),
    Dismissed,
}

/// A blame viewer panel that shows per-line annotations for a file.
pub struct BlameView {
    lines: Arc<Vec<BlameLine>>,
    file_path: Option<String>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    selected_line: Option<usize>,
    highlighted_row: Option<usize>,
    /// Maps commit OID string to a color index for alternating hunk backgrounds.
    oid_color_indices: Arc<HashMap<String, usize>>,
}

impl EventEmitter<BlameViewEvent> for BlameView {}

impl BlameView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            lines: Arc::new(Vec::new()),
            file_path: None,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            selected_line: None,
            highlighted_row: None,
            oid_color_indices: Arc::new(HashMap::new()),
        }
    }

    /// Set blame data for display.
    pub fn set_blame(
        &mut self,
        lines: Vec<BlameLine>,
        file_path: String,
        cx: &mut Context<Self>,
    ) {
        self.oid_color_indices = Arc::new(Self::compute_oid_color_map(&lines));
        self.lines = Arc::new(lines);
        self.file_path = Some(file_path);
        self.highlighted_row = None;
        self.selected_line = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Clear blame data.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.lines = Arc::new(Vec::new());
        self.file_path = None;
        self.highlighted_row = None;
        self.selected_line = None;
        self.oid_color_indices = Arc::new(HashMap::new());
        cx.notify();
    }

    /// Whether the blame view has data to display.
    pub fn has_data(&self) -> bool {
        !self.lines.is_empty()
    }

    pub fn file_path(&self) -> Option<&str> {
        self.file_path.as_deref()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Build a mapping from commit OID to a color index for alternating hunk backgrounds.
    /// Assigns indices in order of first appearance.
    fn compute_oid_color_map(lines: &[BlameLine]) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        let mut next_idx = 0usize;
        for line in lines {
            let oid_str = line.entry.oid.to_string();
            if !map.contains_key(&oid_str) {
                map.insert(oid_str, next_idx);
                next_idx += 1;
            }
        }
        map
    }

    /// Format a timestamp as a human-readable relative time string.
    fn format_relative_time(timestamp: i64) -> String {
        let now = chrono::Utc::now().timestamp();
        let diff = now - timestamp;
        if diff < 0 {
            return "in the future".to_string();
        }
        let diff = diff as u64;
        match diff {
            0..=59 => "just now".to_string(),
            60..=3599 => {
                let mins = diff / 60;
                format!("{}m ago", mins)
            }
            3600..=86399 => {
                let hours = diff / 3600;
                format!("{}h ago", hours)
            }
            86400..=2591999 => {
                let days = diff / 86400;
                format!("{}d ago", days)
            }
            2592000..=31535999 => {
                let months = diff / 2592000;
                format!("{}mo ago", months)
            }
            _ => {
                let years = diff / 31536000;
                format!("{}y ago", years)
            }
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let line_count = self.lines.len();
        if line_count == 0 {
            return;
        }

        match key {
            "j" | "down" => {
                let next = self
                    .highlighted_row
                    .map(|r| (r + 1).min(line_count - 1))
                    .unwrap_or(0);
                self.highlighted_row = Some(next);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "k" | "up" => {
                let prev = self
                    .highlighted_row
                    .map(|r| r.saturating_sub(1))
                    .unwrap_or(0);
                self.highlighted_row = Some(prev);
                self.scroll_handle
                    .scroll_to_item(prev, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "enter" => {
                if let Some(row) = self.highlighted_row {
                    if let Some(line) = self.lines.get(row) {
                        let oid = line.entry.oid.to_string();
                        cx.emit(BlameViewEvent::CommitSelected(oid));
                    }
                }
                cx.stop_propagation();
            }
            "escape" => {
                cx.emit(BlameViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                self.highlighted_row = Some(0);
                self.scroll_handle
                    .scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for BlameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.lines.is_empty() {
            return div()
                .id("blame-view")
                .v_flex()
                .size_full()
                .bg(colors.editor_background)
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(26.))
                        .px(px(10.))
                        .gap(px(4.))
                        .items_center()
                        .bg(colors.toolbar_background)
                        .border_b_1()
                        .border_color(colors.border_variant)
                        .child(
                            Icon::new(IconName::Eye)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Blame")
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::SEMIBOLD)
                                .color(Color::Muted),
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
                                .gap(px(8.))
                                .items_center()
                                .px(px(24.))
                                .py(px(16.))
                                .rounded(px(8.))
                                .bg(gpui::Hsla {
                                    a: 0.03,
                                    ..colors.text
                                })
                                .child(
                                    Icon::new(IconName::Eye)
                                        .size(IconSize::Large)
                                        .color(Color::Placeholder),
                                )
                                .child(
                                    Label::new("No blame data")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new("Select a file and press 'b' to view blame")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Placeholder),
                                ),
                        ),
                )
                .into_any_element();
        }

        let lines = self.lines.clone();
        let oid_colors = self.oid_color_indices.clone();
        let line_count = lines.len();
        let view: WeakEntity<BlameView> = cx.weak_entity();

        let editor_bg = colors.editor_background;
        let text_color = colors.text;
        let text_placeholder_color = colors.text_placeholder;
        let border_variant = colors.border_variant;
        let text_accent = colors.text_accent;

        let gutter_bg = gpui::Hsla {
            a: 0.06,
            ..colors.text
        };

        let row_height = 20.0_f32;
        let highlighted_row = self.highlighted_row;
        let selected_line = self.selected_line;
        let highlight_bg = gpui::Hsla {
            a: 0.10,
            ..text_accent
        };
        let selected_bg = gpui::Hsla {
            a: 0.15,
            ..text_accent
        };
        let row_hover_bg = gpui::Hsla {
            a: 0.04,
            ..colors.text
        };

        // Two alternating subtle hunk backgrounds for visual grouping by commit
        let hunk_bg_even = gpui::Hsla {
            a: 0.02,
            ..text_accent
        };
        let hunk_bg_odd = gpui::Hsla {
            a: 0.05,
            ..text_accent
        };

        let file_path_display: SharedString = self
            .file_path
            .as_deref()
            .unwrap_or("Unknown file")
            .to_string()
            .into();

        let list = uniform_list(
            "blame-lines",
            line_count,
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let line = &lines[i];
                        let oid_str = line.entry.oid.to_string();
                        let color_idx = oid_colors.get(&oid_str).copied().unwrap_or(0);
                        let hunk_bg = if color_idx % 2 == 0 {
                            hunk_bg_even
                        } else {
                            hunk_bg_odd
                        };

                        let is_highlighted = highlighted_row == Some(i);
                        let is_selected = selected_line == Some(i);
                        let effective_bg = if is_selected {
                            selected_bg
                        } else if is_highlighted {
                            highlight_bg
                        } else {
                            hunk_bg
                        };

                        let is_first_in_hunk = if i == 0 {
                            true
                        } else {
                            lines[i - 1].entry.oid != line.entry.oid
                                || lines[i - 1].entry.start_line != line.entry.start_line
                        };

                        let author_display: SharedString = if is_first_in_hunk {
                            let name = &line.entry.author;
                            let truncated = if name.len() > 16 {
                                format!("{}...", &name[..14])
                            } else {
                                name.clone()
                            };
                            truncated.into()
                        } else {
                            SharedString::default()
                        };

                        let time_display: SharedString = if is_first_in_hunk {
                            BlameView::format_relative_time(line.entry.time.timestamp()).into()
                        } else {
                            SharedString::default()
                        };

                        let sha_display: SharedString = if is_first_in_hunk {
                            line.entry.short_id.clone().into()
                        } else {
                            SharedString::default()
                        };

                        let line_no_str: SharedString =
                            format!("{:>5}", line.line_no).into();
                        let content_text: SharedString =
                            line.content.clone().into();

                        let view_click = view.clone();
                        let view_commit = view.clone();
                        let commit_oid = oid_str.clone();

                        div()
                            .id(ElementId::NamedInteger(
                                "blame-line".into(),
                                i as u64,
                            ))
                            .h_flex()
                            .h(px(row_height))
                            .w_full()
                            .bg(effective_bg)
                            .hover(move |s| s.bg(row_hover_bg))
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.highlighted_row = Some(i);
                                            this.selected_line = Some(i);
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )
                            // Blame gutter: author
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .bg(gutter_bg)
                                    .text_xs()
                                    .text_color(text_placeholder_color)
                                    .pl(px(6.))
                                    .overflow_x_hidden()
                                    .child(author_display),
                            )
                            // Blame gutter: relative time
                            .child(
                                div()
                                    .w(px(64.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .bg(gutter_bg)
                                    .text_xs()
                                    .text_color(text_placeholder_color)
                                    .child(time_display),
                            )
                            // Blame gutter: short SHA (clickable)
                            .child(
                                div()
                                    .id(ElementId::NamedInteger(
                                        "blame-sha".into(),
                                        i as u64,
                                    ))
                                    .w(px(60.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .bg(gutter_bg)
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .text_xs()
                                    .text_color(text_accent)
                                    .cursor_pointer()
                                    .on_click({
                                        let commit_oid = commit_oid.clone();
                                        move |_: &ClickEvent,
                                              _: &mut Window,
                                              cx: &mut App| {
                                            let oid = commit_oid.clone();
                                            view_commit
                                                .update(cx, |_this, cx| {
                                                    cx.emit(
                                                        BlameViewEvent::CommitSelected(oid),
                                                    );
                                                })
                                                .ok();
                                        }
                                    })
                                    .child(sha_display),
                            )
                            // Line number
                            .child(
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .bg(gutter_bg)
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .text_xs()
                                    .text_color(text_placeholder_color)
                                    .px_1()
                                    .child(line_no_str),
                            )
                            // Content area
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .pl(px(8.))
                                    .text_xs()
                                    .text_color(text_color)
                                    .whitespace_nowrap()
                                    .child(content_text),
                            )
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        div()
            .id("blame-view")
            .track_focus(&self.focus_handle)
            .key_context("BlameView")
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(editor_bg)
            // Header bar
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(26.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(IconName::Eye)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("Blame")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(file_path_display)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new(SharedString::from(format!("{} lines", line_count)))
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
