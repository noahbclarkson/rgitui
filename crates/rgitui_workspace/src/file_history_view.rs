use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent, Render, ScrollStrategy,
    SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::CommitInfo;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

/// Events emitted by the file history view.
#[derive(Debug, Clone)]
pub enum FileHistoryViewEvent {
    CommitSelected(String),
    Dismissed,
}

/// A file history viewer panel that shows commits touching a specific file.
pub struct FileHistoryView {
    commits: Arc<Vec<CommitInfo>>,
    file_path: Option<String>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    selected_row: Option<usize>,
    highlighted_row: Option<usize>,
}

impl EventEmitter<FileHistoryViewEvent> for FileHistoryView {}

impl FileHistoryView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            commits: Arc::new(Vec::new()),
            file_path: None,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            selected_row: None,
            highlighted_row: None,
        }
    }

    pub fn set_history(
        &mut self,
        commits: Vec<CommitInfo>,
        file_path: String,
        cx: &mut Context<Self>,
    ) {
        self.commits = Arc::new(commits);
        self.file_path = Some(file_path);
        self.highlighted_row = None;
        self.selected_row = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.commits = Arc::new(Vec::new());
        self.file_path = None;
        self.highlighted_row = None;
        self.selected_row = None;
        cx.notify();
    }

    pub fn has_data(&self) -> bool {
        !self.commits.is_empty()
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
        let modifiers = &event.keystroke.modifiers;
        let count = self.commits.len();
        if count == 0 {
            return;
        }

        match key {
            "j" | "down" => {
                let next = self
                    .highlighted_row
                    .map(|r| (r + 1).min(count - 1))
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
                    if let Some(commit) = self.commits.get(row) {
                        let oid = commit.oid.to_string();
                        cx.emit(FileHistoryViewEvent::CommitSelected(oid));
                    }
                }
                cx.stop_propagation();
            }
            "escape" => {
                cx.emit(FileHistoryViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                if modifiers.shift {
                    // G (Shift+G) — jump to last commit
                    let last = count - 1;
                    self.highlighted_row = Some(last);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Bottom);
                } else {
                    // g — jump to first commit
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
            .id("file-history-view")
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
                        Icon::new(IconName::Clock)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("File History")
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
                            Label::new("Select a file to view history")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Press 'h' on a file to see commits")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for FileHistoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.commits.is_empty() {
            return self.render_empty_state(cx);
        }

        let commits = self.commits.clone();
        let count = commits.len();
        let view: WeakEntity<FileHistoryView> = cx.weak_entity();

        let editor_bg = colors.editor_background;
        let text_color = colors.text;
        let text_muted = colors.text_muted;
        let border_variant = colors.border_variant;
        let text_accent = colors.text_accent;
        let _ghost_hover = colors.ghost_element_hover;

        let row_height = 24.0_f32;
        let highlighted_row = self.highlighted_row;
        let selected_row = self.selected_row;

        let selected_bg = colors.ghost_element_selected;
        let highlight_bg = colors.ghost_element_active;

        let file_path_display: SharedString = self
            .file_path
            .as_deref()
            .unwrap_or("Unknown file")
            .to_string()
            .into();

        let list = uniform_list(
            "file-history-commits",
            count,
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let commit = &commits[i];

                        let is_highlighted = highlighted_row == Some(i);
                        let is_selected = selected_row == Some(i);
                        let effective_bg = if is_selected {
                            selected_bg
                        } else if is_highlighted {
                            highlight_bg
                        } else {
                            editor_bg
                        };

                        let sha_display: SharedString = commit.short_id.clone().into();
                        let summary_display: SharedString = commit.summary.clone().into();
                        let author_display: SharedString = commit.author.name.clone().into();
                        let time_display: SharedString =
                            Self::format_relative_time(commit.time.timestamp()).into();

                        let tooltip_text: SharedString = format!(
                            "{}\n{}\n{} <{}>",
                            commit.short_id,
                            commit.summary,
                            commit.author.name,
                            commit.author.email,
                        )
                        .into();

                        let view_click = view.clone();
                        let view_commit = view.clone();
                        let commit_oid = commit.oid.to_string();

                        div()
                            .id(ElementId::NamedInteger(
                                "file-history-commit".into(),
                                i as u64,
                            ))
                            .h_flex()
                            .h(px(row_height))
                            .w_full()
                            .bg(effective_bg)
                            .border_b_1()
                            .border_color(border_variant)
                            .tooltip(Tooltip::text(tooltip_text))
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.highlighted_row = Some(i);
                                            this.selected_row = Some(i);
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )
                            .child(
                                div()
                                    .id(ElementId::NamedInteger(
                                        "file-history-sha".into(),
                                        i as u64,
                                    ))
                                    .w(px(70.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .pl(px(8.))
                                    .text_xs()
                                    .text_color(text_accent)
                                    .cursor_pointer()
                                    .on_click({
                                        let commit_oid = commit_oid.clone();
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let oid = commit_oid.clone();
                                            view_commit
                                                .update(cx, |_this, cx| {
                                                    cx.emit(FileHistoryViewEvent::CommitSelected(
                                                        oid,
                                                    ));
                                                })
                                                .ok();
                                        }
                                    })
                                    .child(sha_display),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_color)
                                    .overflow_x_hidden()
                                    .child(summary_display),
                            )
                            .child(
                                div()
                                    .w(px(100.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .overflow_x_hidden()
                                    .child(author_display),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(time_display),
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
            .id("file-history-view")
            .track_focus(&self.focus_handle)
            .key_context("FileHistoryView")
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
                        Icon::new(IconName::Clock)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("File History")
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
                        Label::new(SharedString::from(format!("{} commits", count)))
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
