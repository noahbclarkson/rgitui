use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, Render,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::BlameLine;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

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
    hovered_oid: Option<String>,
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
            hovered_oid: None,
            oid_color_indices: Arc::new(HashMap::new()),
        }
    }

    pub fn set_blame(&mut self, lines: Vec<BlameLine>, file_path: String, cx: &mut Context<Self>) {
        self.oid_color_indices = Arc::new(Self::compute_oid_color_map(&lines));
        self.lines = Arc::new(lines);
        self.file_path = Some(file_path);
        self.highlighted_row = None;
        self.selected_line = None;
        self.hovered_oid = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.lines = Arc::new(Vec::new());
        self.file_path = None;
        self.highlighted_row = None;
        self.selected_line = None;
        self.hovered_oid = None;
        self.oid_color_indices = Arc::new(HashMap::new());
        cx.notify();
    }

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

    pub(crate) fn compute_oid_color_map(lines: &[BlameLine]) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        let mut next_idx = 0usize;
        for line in lines {
            let oid_str = line.entry.oid.to_string();
            if let std::collections::hash_map::Entry::Vacant(e) = map.entry(oid_str) {
                e.insert(next_idx);
                next_idx += 1;
            }
        }
        map
    }

    pub(crate) fn format_relative_time(timestamp: i64) -> String {
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
        let modifiers = &event.keystroke.modifiers;
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
                if modifiers.shift {
                    // G (Shift+G) — jump to last line
                    let last = line_count - 1;
                    self.highlighted_row = Some(last);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Bottom);
                } else {
                    // g — jump to first line
                    self.highlighted_row = Some(0);
                    self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                }
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();

        div()
            .id("blame-view")
            .v_flex()
            .size_full()
            .bg(colors.editor_background)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
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
                div().flex_1().flex().items_center().justify_center().child(
                    div()
                        .v_flex()
                        .gap(px(8.))
                        .items_center()
                        .px(px(24.))
                        .py(px(16.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_background)
                        .child(
                            Icon::new(IconName::File)
                                .size(IconSize::Large)
                                .color(Color::Placeholder),
                        )
                        .child(
                            Label::new("Select a file to view blame")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Press 'b' on a file to see line-by-line attribution")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for BlameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.lines.is_empty() {
            return self.render_empty_state(cx);
        }

        let lines = self.lines.clone();
        let oid_colors = self.oid_color_indices.clone();
        let line_count = lines.len();
        let view: WeakEntity<BlameView> = cx.weak_entity();

        let editor_bg = colors.editor_background;
        let text_color = colors.text;
        let text_muted = colors.text_muted;
        let border_variant = colors.border_variant;
        let text_accent = colors.text_accent;
        let surface_bg = colors.surface_background;
        let ghost_hover = colors.ghost_element_hover;

        let row_height = 20.0_f32;
        let highlighted_row = self.highlighted_row;
        let selected_line = self.selected_line;
        let hovered_oid = self.hovered_oid.clone();

        let selected_bg = colors.ghost_element_selected;
        let highlight_bg = colors.ghost_element_active;

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
                            editor_bg
                        } else {
                            surface_bg
                        };

                        let is_highlighted = highlighted_row == Some(i);
                        let is_selected = selected_line == Some(i);
                        let is_oid_hovered = hovered_oid.as_deref() == Some(oid_str.as_str());
                        let effective_bg = if is_selected {
                            selected_bg
                        } else if is_highlighted {
                            highlight_bg
                        } else if is_oid_hovered {
                            ghost_hover
                        } else {
                            hunk_bg
                        };

                        let is_first_in_hunk = if i == 0 {
                            true
                        } else {
                            lines[i - 1].entry.oid != line.entry.oid
                                || lines[i - 1].entry.start_line != line.entry.start_line
                        };

                        let has_boundary = i > 0 && lines[i - 1].entry.oid != line.entry.oid;

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

                        let line_no_str: SharedString = format!("{:>5}", line.line_no).into();
                        let content_text: SharedString = line.content.clone().into();

                        let tooltip_text: SharedString = format!(
                            "{} ({}) - {} <{}>",
                            line.entry.short_id,
                            BlameView::format_relative_time(line.entry.time.timestamp()),
                            line.entry.author,
                            line.entry.email,
                        )
                        .into();

                        let view_click = view.clone();
                        let view_commit = view.clone();
                        let view_hover = view.clone();
                        let commit_oid = oid_str.clone();
                        let hover_oid = oid_str.clone();

                        div()
                            .id(ElementId::NamedInteger("blame-line".into(), i as u64))
                            .h_flex()
                            .h(px(row_height))
                            .w_full()
                            .bg(effective_bg)
                            .when(has_boundary, |el| {
                                el.border_t_1().border_color(border_variant)
                            })
                            .tooltip(Tooltip::text(tooltip_text))
                            .on_mouse_move({
                                let hover_oid = hover_oid.clone();
                                move |_: &MouseMoveEvent, _: &mut Window, cx: &mut App| {
                                    let oid = hover_oid.clone();
                                    view_hover
                                        .update(cx, |this, cx| {
                                            if this.hovered_oid.as_deref() != Some(oid.as_str()) {
                                                this.hovered_oid = Some(oid);
                                                cx.notify();
                                            }
                                        })
                                        .ok();
                                }
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.highlighted_row = Some(i);
                                            this.selected_line = Some(i);
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .pl(px(6.))
                                    .overflow_x_hidden()
                                    .child(author_display),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(time_display),
                            )
                            .child(
                                div()
                                    .id(ElementId::NamedInteger("blame-sha".into(), i as u64))
                                    .w(px(60.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .text_xs()
                                    .text_color(text_accent)
                                    .cursor_pointer()
                                    .on_click({
                                        let commit_oid = commit_oid.clone();
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let oid = commit_oid.clone();
                                            view_commit
                                                .update(cx, |_this, cx| {
                                                    cx.emit(BlameViewEvent::CommitSelected(oid));
                                                })
                                                .ok();
                                        }
                                    })
                                    .child(sha_display),
                            )
                            .child(
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .text_xs()
                                    .text_color(text_muted)
                                    .px_1()
                                    .child(line_no_str),
                            )
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
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use git2::Oid;
    use rgitui_git::BlameEntry;

    fn make_blame_line(oid: &str, line_no: usize, content: &str) -> BlameLine {
        BlameLine {
            line_no,
            content: content.to_string(),
            entry: BlameEntry {
                oid: Oid::from_str(oid).unwrap(),
                short_id: oid[..7].to_string(),
                author: "Test Author".to_string(),
                email: "test@example.com".to_string(),
                time: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                start_line: line_no,
                line_count: 1,
            },
        }
    }

    // --- compute_oid_color_map tests ---

    #[test]
    fn test_compute_oid_color_map_empty() {
        let lines: &[BlameLine] = &[];
        let result = BlameView::compute_oid_color_map(lines);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compute_oid_color_map_single_author() {
        let lines = vec![
            make_blame_line("aabbccddee00112233445566778899aabbccdd00", 1, "line 1"),
            make_blame_line("aabbccddee00112233445566778899aabbccdd00", 2, "line 2"),
            make_blame_line("aabbccddee00112233445566778899aabbccdd00", 3, "line 3"),
        ];
        let result = BlameView::compute_oid_color_map(&lines);
        assert_eq!(result.len(), 1);
        assert_eq!(
            *result
                .get("aabbccddee00112233445566778899aabbccdd00")
                .unwrap(),
            0
        );
    }

    #[test]
    fn test_compute_oid_color_map_multiple_authors() {
        let lines = vec![
            make_blame_line("aaaa000000000000000000000000000000000000", 1, "line 1"),
            make_blame_line("bbbb000000000000000000000000000000000000", 2, "line 2"),
            make_blame_line("aaaa000000000000000000000000000000000000", 3, "line 3"),
        ];
        let result = BlameView::compute_oid_color_map(&lines);
        assert_eq!(result.len(), 2);
        // aaaa appears first → index 0
        assert_eq!(
            *result
                .get("aaaa000000000000000000000000000000000000")
                .unwrap(),
            0
        );
        // bbbb appears second → index 1
        assert_eq!(
            *result
                .get("bbbb000000000000000000000000000000000000")
                .unwrap(),
            1
        );
    }

    #[test]
    fn test_compute_oid_color_map_three_authors() {
        let lines = vec![
            make_blame_line("cccc000000000000000000000000000000000000", 1, "line 1"),
            make_blame_line("dddd000000000000000000000000000000000000", 2, "line 2"),
            make_blame_line("eeee000000000000000000000000000000000000", 3, "line 3"),
        ];
        let result = BlameView::compute_oid_color_map(&lines);
        assert_eq!(result.len(), 3);
        assert_eq!(
            *result
                .get("cccc000000000000000000000000000000000000")
                .unwrap(),
            0
        );
        assert_eq!(
            *result
                .get("dddd000000000000000000000000000000000000")
                .unwrap(),
            1
        );
        assert_eq!(
            *result
                .get("eeee000000000000000000000000000000000000")
                .unwrap(),
            2
        );
    }

    // --- format_relative_time tests (blame_view abbreviated variant) ---

    #[test]
    fn test_format_relative_time_future() {
        let future = chrono::Utc::now().timestamp() + 3600;
        assert_eq!(BlameView::format_relative_time(future), "in the future");
    }

    #[test]
    fn test_format_relative_time_just_now() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(BlameView::format_relative_time(now), "just now");
    }

    #[test]
    fn test_format_relative_time_seconds() {
        let past = chrono::Utc::now().timestamp() - 30;
        assert_eq!(BlameView::format_relative_time(past), "just now");
    }

    #[test]
    fn test_format_relative_time_minutes() {
        let past = chrono::Utc::now().timestamp() - 300; // 5m ago
        assert_eq!(BlameView::format_relative_time(past), "5m ago");
    }

    #[test]
    fn test_format_relative_time_single_minute() {
        let past = chrono::Utc::now().timestamp() - 60;
        assert_eq!(BlameView::format_relative_time(past), "1m ago");
    }

    #[test]
    fn test_format_relative_time_hours() {
        let past = chrono::Utc::now().timestamp() - 7200; // 2h ago
        assert_eq!(BlameView::format_relative_time(past), "2h ago");
    }

    #[test]
    fn test_format_relative_time_single_hour() {
        let past = chrono::Utc::now().timestamp() - 3600;
        assert_eq!(BlameView::format_relative_time(past), "1h ago");
    }

    #[test]
    fn test_format_relative_time_days() {
        let past = chrono::Utc::now().timestamp() - 172800; // 2d ago
        assert_eq!(BlameView::format_relative_time(past), "2d ago");
    }

    #[test]
    fn test_format_relative_time_single_day() {
        let past = chrono::Utc::now().timestamp() - 86400;
        assert_eq!(BlameView::format_relative_time(past), "1d ago");
    }

    #[test]
    fn test_format_relative_time_months() {
        let past = chrono::Utc::now().timestamp() - 5184000; // ~2mo ago
        assert_eq!(BlameView::format_relative_time(past), "2mo ago");
    }

    #[test]
    fn test_format_relative_time_single_month() {
        let past = chrono::Utc::now().timestamp() - 2592000;
        assert_eq!(BlameView::format_relative_time(past), "1mo ago");
    }

    #[test]
    fn test_format_relative_time_years() {
        let past = chrono::Utc::now().timestamp() - 63072000; // ~2y ago
        assert_eq!(BlameView::format_relative_time(past), "2y ago");
    }

    #[test]
    fn test_format_relative_time_single_year() {
        let past = chrono::Utc::now().timestamp() - 31536000;
        assert_eq!(BlameView::format_relative_time(past), "1y ago");
    }
}
