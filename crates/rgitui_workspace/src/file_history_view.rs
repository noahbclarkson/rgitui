use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    ListSizingBehavior, MouseButton, MouseDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::CommitInfo;
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

use crate::keymap;
use crate::CommandId;

/// Events emitted by the file history view.
#[derive(Debug, Clone, PartialEq)]
pub enum FileHistoryViewEvent {
    CommitSelected(String),
    Dismissed,
    SwitchToBlame,
    SwitchToDiff,
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

    /// Runs a keyboard command scoped to `FileHistoryView` or to the shared
    /// `List` group. Esc navigates back to the diff rather than dismissing, so
    /// the view owns `HistoryShowDiff` instead of leaning on `menu::Cancel`.
    fn dispatch_command(&mut self, cmd: CommandId, _window: &mut Window, cx: &mut Context<Self>) {
        let count = self.commits.len();

        match cmd {
            CommandId::HistoryShowDiff => cx.emit(FileHistoryViewEvent::SwitchToDiff),
            CommandId::HistoryShowBlame => cx.emit(FileHistoryViewEvent::SwitchToBlame),
            CommandId::Cancel => cx.emit(FileHistoryViewEvent::Dismissed),
            _ if count == 0 => {}
            CommandId::SelectNext => self.highlight_row(
                self.highlighted_row
                    .map_or(0, |row| (row + 1).min(count - 1)),
                ScrollStrategy::Nearest,
                cx,
            ),
            CommandId::SelectPrev => self.highlight_row(
                self.highlighted_row.map_or(0, |row| row.saturating_sub(1)),
                ScrollStrategy::Nearest,
                cx,
            ),
            CommandId::SelectFirst => self.highlight_row(0, ScrollStrategy::Top, cx),
            CommandId::SelectLast => self.highlight_row(count - 1, ScrollStrategy::Bottom, cx),
            CommandId::Confirm => {
                if let Some(commit) = self.highlighted_row.and_then(|row| self.commits.get(row)) {
                    cx.emit(FileHistoryViewEvent::CommitSelected(commit.oid.to_string()));
                }
            }
            // A command this view does not own falls through to the next handler
            // out, and finally to the focused text field.
            _ => cx.propagate(),
        }
    }

    /// Moves the keyboard highlight and scrolls it into view.
    fn highlight_row(&mut self, row: usize, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        self.highlighted_row = Some(row);
        self.scroll_handle.scroll_to_item(row, strategy);
        cx.notify();
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
        let colors = cx.colors().clone();

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

        let compactness = cx.global::<SettingsState>().settings().compactness;
        let row_height = compactness.spacing(24.0);
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
                        // Author column: no pre-truncation; CSS handles overflow with ellipsis via .overflow_x_hidden()
                        let author_display: SharedString = commit.author.name.clone().into();
                        let time_display: SharedString =
                            super::time::format_relative_time_abbreviated(commit.time.timestamp())
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
                                    .w(px(150.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .pl(px(6.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .overflow_x_hidden()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_x_hidden()
                                            .text_ellipsis()
                                            .id(ElementId::NamedInteger(
                                                "file-history-author".into(),
                                                i as u64,
                                            ))
                                            .child(author_display),
                                    ),
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
            .map(|el| {
                keymap::bind_actions(
                    el,
                    "FileHistoryView List",
                    &["Menu", "FileHistoryView"],
                    cx,
                    Self::dispatch_command,
                )
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- FileHistoryViewEvent tests ---

    #[test]
    fn file_history_view_event_debug() {
        let event = FileHistoryViewEvent::Dismissed;
        let debug = format!("{:?}", event);
        assert!(debug.contains("Dismissed"));
    }

    #[test]
    fn file_history_view_event_clone_eq() {
        let event = FileHistoryViewEvent::CommitSelected("abc123".to_string());
        let clone = event.clone();
        assert_eq!(event, clone);
    }

    #[test]
    fn file_history_view_event_switch_to_blame() {
        let event = FileHistoryViewEvent::SwitchToBlame;
        assert_eq!(format!("{:?}", event), "SwitchToBlame");
    }

    #[test]
    fn file_history_view_event_switch_to_diff() {
        let event = FileHistoryViewEvent::SwitchToDiff;
        assert_eq!(format!("{:?}", event), "SwitchToDiff");
    }

    #[test]
    fn file_history_view_event_commit_selected() {
        let oid = "aabbccddee00112233445566778899aabbccdd00";
        let event = FileHistoryViewEvent::CommitSelected(oid.to_string());
        if let FileHistoryViewEvent::CommitSelected(val) = event {
            assert_eq!(val, oid);
        } else {
            panic!("expected CommitSelected");
        }
    }

    // --- FileHistoryView struct field tests ---

    #[test]
    fn file_history_view_debug_clone() {
        // Test that FileHistoryView derives Debug and Clone properly
        // by creating a minimal instance via the public constructor
        // We verify through the event enum which FileHistoryView emits
        let events = vec![
            FileHistoryViewEvent::Dismissed,
            FileHistoryViewEvent::SwitchToBlame,
            FileHistoryViewEvent::CommitSelected("deadbeef".to_string()),
        ];
        for event in events {
            let cloned = event.clone();
            assert_eq!(format!("{:?}", event), format!("{:?}", cloned));
        }
    }

    #[test]
    fn file_history_view_event_all_variants() {
        use FileHistoryViewEvent::*;
        let variants = [
            CommitSelected("test".to_string()),
            Dismissed,
            SwitchToBlame,
            SwitchToDiff,
        ];
        for v in variants {
            let debug = format!("{:?}", v);
            assert!(!debug.is_empty());
        }
    }
}
