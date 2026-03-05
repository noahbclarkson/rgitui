use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter,
    ListSizingBehavior, Render, SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{DiffLine, FileDiff};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Label, LabelSize};

/// Events from the diff viewer.
#[derive(Debug, Clone)]
pub enum DiffViewerEvent {
    HunkStageRequested(usize),
    HunkUnstageRequested(usize),
}

/// Display mode for the diff viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffDisplayMode {
    #[default]
    Unified,
    SideBySide,
}

/// A pre-computed row for virtualized rendering.
#[derive(Clone)]
enum DisplayRow {
    HunkHeader { header: String, hunk_index: usize },
    Line {
        old_num: Option<usize>,
        new_num: Option<usize>,
        content: String,
        kind: DisplayLineKind,
    },
}

#[derive(Clone, Copy)]
enum DisplayLineKind {
    Context,
    Addition,
    Deletion,
}

/// A diff viewer panel that displays file diffs with syntax coloring.
pub struct DiffViewer {
    diff: Option<FileDiff>,
    display_mode: DiffDisplayMode,
    file_path: Option<String>,
    is_staged: bool,
    display_rows: Arc<Vec<DisplayRow>>,
    scroll_handle: UniformListScrollHandle,
}

impl EventEmitter<DiffViewerEvent> for DiffViewer {}

impl DiffViewer {
    pub fn new() -> Self {
        Self {
            diff: None,
            display_mode: DiffDisplayMode::Unified,
            file_path: None,
            is_staged: false,
            display_rows: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub fn set_diff(&mut self, diff: FileDiff, path: String, staged: bool, cx: &mut Context<Self>) {
        self.display_rows = Arc::new(Self::compute_display_rows(&diff));
        self.diff = Some(diff);
        self.file_path = Some(path);
        self.is_staged = staged;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.file_path = None;
        self.display_rows = Arc::new(Vec::new());
        cx.notify();
    }

    pub fn toggle_display_mode(&mut self, cx: &mut Context<Self>) {
        self.display_mode = match self.display_mode {
            DiffDisplayMode::Unified => DiffDisplayMode::SideBySide,
            DiffDisplayMode::SideBySide => DiffDisplayMode::Unified,
        };
        if let Some(diff) = &self.diff {
            self.display_rows = Arc::new(Self::compute_display_rows(diff));
        }
        cx.notify();
    }

    pub fn diff(&self) -> Option<&FileDiff> {
        self.diff.as_ref()
    }

    pub fn file_path(&self) -> Option<&str> {
        self.file_path.as_deref()
    }

    pub fn is_staged(&self) -> bool {
        self.is_staged
    }

    /// Flatten a FileDiff into a list of display rows for virtualization.
    fn compute_display_rows(diff: &FileDiff) -> Vec<DisplayRow> {
        let mut rows = Vec::new();
        for (i, hunk) in diff.hunks.iter().enumerate() {
            rows.push(DisplayRow::HunkHeader {
                header: hunk.header.trim().to_string(),
                hunk_index: i,
            });
            let mut old_line = hunk.old_start as usize;
            let mut new_line = hunk.new_start as usize;
            for line in &hunk.lines {
                match line {
                    DiffLine::Context(text) => {
                        rows.push(DisplayRow::Line {
                            old_num: Some(old_line),
                            new_num: Some(new_line),
                            content: text.clone(),
                            kind: DisplayLineKind::Context,
                        });
                        old_line += 1;
                        new_line += 1;
                    }
                    DiffLine::Addition(text) => {
                        rows.push(DisplayRow::Line {
                            old_num: None,
                            new_num: Some(new_line),
                            content: text.clone(),
                            kind: DisplayLineKind::Addition,
                        });
                        new_line += 1;
                    }
                    DiffLine::Deletion(text) => {
                        rows.push(DisplayRow::Line {
                            old_num: Some(old_line),
                            new_num: None,
                            content: text.clone(),
                            kind: DisplayLineKind::Deletion,
                        });
                        old_line += 1;
                    }
                }
            }
        }
        rows
    }
}

impl Render for DiffViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.diff.is_none() {
            return div()
                .id("diff-viewer")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(colors.editor_background)
                .child(
                    Label::new("Select a file to view diff")
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
                .into_any_element();
        }

        let diff = self.diff.as_ref().unwrap();
        let display_rows = self.display_rows.clone();
        let is_staged = self.is_staged;
        let view: WeakEntity<DiffViewer> = cx.weak_entity();

        // Extract colors for closure
        let editor_bg = colors.editor_background;
        let elevated_bg = colors.elevated_surface_background;
        let text_muted_color = colors.text_muted;
        let vc_added = colors.vc_added;
        let vc_deleted = colors.vc_deleted;
        let added_bg = gpui::Hsla { a: 0.1, ..vc_added };
        let deleted_bg = gpui::Hsla { a: 0.1, ..vc_deleted };

        let row_height = 22.0_f32;

        // Virtualized list of diff lines
        let list = uniform_list(
            "diff-lines",
            display_rows.len(),
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let row = &display_rows[i];
                        match row {
                            DisplayRow::HunkHeader { header, hunk_index } => {
                                let header_text: SharedString = header.clone().into();
                                let stage_label = if is_staged {
                                    "Unstage Hunk"
                                } else {
                                    "Stage Hunk"
                                };
                                let idx = *hunk_index;
                                let view_clone = view.clone();

                                div()
                                    .id(ElementId::NamedInteger(
                                        "hunk-header".into(),
                                        i as u64,
                                    ))
                                    .h_flex()
                                    .h(px(row_height))
                                    .w_full()
                                    .px_2()
                                    .items_center()
                                    .bg(elevated_bg)
                                    .child(
                                        Label::new(header_text)
                                            .color(Color::Muted)
                                            .size(LabelSize::XSmall),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("hunk-stage-{}", idx)),
                                            stage_label,
                                        )
                                        .size(ButtonSize::Compact)
                                        .style(ButtonStyle::Subtle)
                                        .on_click(
                                            move |_: &ClickEvent,
                                                  _: &mut Window,
                                                  cx: &mut App| {
                                                view_clone
                                                    .update(cx, |_this, cx| {
                                                        if is_staged {
                                                            cx.emit(
                                                                DiffViewerEvent::HunkUnstageRequested(
                                                                    idx,
                                                                ),
                                                            );
                                                        } else {
                                                            cx.emit(
                                                                DiffViewerEvent::HunkStageRequested(
                                                                    idx,
                                                                ),
                                                            );
                                                        }
                                                    })
                                                    .ok();
                                            },
                                        ),
                                    )
                                    .into_any_element()
                            }
                            DisplayRow::Line {
                                old_num,
                                new_num,
                                content,
                                kind,
                            } => {
                                let (prefix, text_color, bg) = match kind {
                                    DisplayLineKind::Context => {
                                        (" ", text_muted_color, editor_bg)
                                    }
                                    DisplayLineKind::Addition => ("+", vc_added, added_bg),
                                    DisplayLineKind::Deletion => ("-", vc_deleted, deleted_bg),
                                };
                                let old_str: SharedString = old_num
                                    .map(|n| format!("{:>4}", n))
                                    .unwrap_or_else(|| "    ".to_string())
                                    .into();
                                let new_str: SharedString = new_num
                                    .map(|n| format!("{:>4}", n))
                                    .unwrap_or_else(|| "    ".to_string())
                                    .into();
                                let content_text: SharedString =
                                    format!("{}{}", prefix, content.trim_end()).into();

                                div()
                                    .h_flex()
                                    .h(px(row_height))
                                    .w_full()
                                    .bg(bg)
                                    .child(
                                        div()
                                            .w(px(40.))
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(text_muted_color)
                                            .px_1()
                                            .child(old_str),
                                    )
                                    .child(
                                        div()
                                            .w(px(40.))
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(text_muted_color)
                                            .px_1()
                                            .child(new_str),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .pl_1()
                                            .text_xs()
                                            .text_color(text_color)
                                            .whitespace_nowrap()
                                            .child(content_text),
                                    )
                                    .into_any_element()
                            }
                        }
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        // File header
        let mut container = div()
            .id("diff-viewer")
            .v_flex()
            .size_full()
            .bg(editor_bg);

        if let Some(path) = &self.file_path {
            let stats: SharedString =
                format!("+{} -{}", diff.additions, diff.deletions).into();
            let path_str: SharedString = path.clone().into();
            let mode_label = match self.display_mode {
                DiffDisplayMode::Unified => "Split",
                DiffDisplayMode::SideBySide => "Unified",
            };
            container = container.child(
                div()
                    .h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(Label::new(path_str).size(LabelSize::Small))
                    .child(div().flex_1())
                    .child(
                        Label::new(stats)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Button::new("toggle-diff-mode", mode_label)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toggle_display_mode(cx);
                            })),
                    ),
            );
        }

        container = container.child(list);

        container.into_any_element()
    }
}
