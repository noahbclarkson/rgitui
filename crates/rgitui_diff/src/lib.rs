use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FontWeight,
    ListSizingBehavior, Render, SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{DiffLine, FileDiff};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize};

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
    HunkHeader {
        header: String,
        hunk_index: usize,
    },
    Line {
        old_num: Option<usize>,
        new_num: Option<usize>,
        content: String,
        kind: DisplayLineKind,
    },
}

/// A pre-computed row for side-by-side rendering.
#[derive(Clone)]
enum SideBySideRow {
    HunkHeader {
        header: String,
        hunk_index: usize,
    },
    Pair {
        left_num: Option<usize>,
        left_content: String,
        left_kind: SideBySideLineKind,
        right_num: Option<usize>,
        right_content: String,
        right_kind: SideBySideLineKind,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum SideBySideLineKind {
    Context,
    Addition,
    Deletion,
    Empty,
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
    sbs_rows: Arc<Vec<SideBySideRow>>,
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
            sbs_rows: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub fn set_diff(&mut self, diff: FileDiff, path: String, staged: bool, cx: &mut Context<Self>) {
        self.display_rows = Arc::new(Self::compute_display_rows(&diff));
        self.sbs_rows = Arc::new(Self::compute_sbs_rows(&diff));
        self.diff = Some(diff);
        self.file_path = Some(path);
        self.is_staged = staged;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.file_path = None;
        self.display_rows = Arc::new(Vec::new());
        self.sbs_rows = Arc::new(Vec::new());
        cx.notify();
    }

    pub fn toggle_display_mode(&mut self, cx: &mut Context<Self>) {
        self.display_mode = match self.display_mode {
            DiffDisplayMode::Unified => DiffDisplayMode::SideBySide,
            DiffDisplayMode::SideBySide => DiffDisplayMode::Unified,
        };
        if let Some(diff) = &self.diff {
            self.display_rows = Arc::new(Self::compute_display_rows(diff));
            self.sbs_rows = Arc::new(Self::compute_sbs_rows(diff));
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

    /// Count additions and deletions from display rows.
    fn count_changes(rows: &[DisplayRow]) -> (usize, usize) {
        let mut additions = 0usize;
        let mut deletions = 0usize;
        for row in rows {
            if let DisplayRow::Line { kind, .. } = row {
                match kind {
                    DisplayLineKind::Addition => additions += 1,
                    DisplayLineKind::Deletion => deletions += 1,
                    DisplayLineKind::Context => {}
                }
            }
        }
        (additions, deletions)
    }

    /// Choose an icon based on file extension.
    fn icon_for_path(path: &str) -> IconName {
        if let Some(ext) = path.rsplit('.').next() {
            match ext {
                "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "go" | "java" | "rb" | "sh"
                | "lua" | "zig" | "swift" | "kt" | "cs" | "ex" | "exs" | "hs" | "ml" => {
                    IconName::File
                }
                "toml" | "yaml" | "yml" | "json" | "xml" | "ini" | "conf" | "cfg" => {
                    IconName::Settings
                }
                "md" | "txt" | "rst" | "org" => IconName::File,
                "lock" => IconName::Pin,
                _ => IconName::File,
            }
        } else {
            IconName::File
        }
    }

    /// Compute side-by-side rows, pairing deletions with additions.
    fn compute_sbs_rows(diff: &FileDiff) -> Vec<SideBySideRow> {
        let mut rows = Vec::new();
        for (i, hunk) in diff.hunks.iter().enumerate() {
            rows.push(SideBySideRow::HunkHeader {
                header: hunk.header.trim().to_string(),
                hunk_index: i,
            });
            let mut old_line = hunk.old_start as usize;
            let mut new_line = hunk.new_start as usize;

            // Collect lines and pair deletions with additions
            let mut pending_dels: Vec<(usize, String)> = Vec::new();
            let mut pending_adds: Vec<(usize, String)> = Vec::new();

            let flush = |rows: &mut Vec<SideBySideRow>,
                         dels: &mut Vec<(usize, String)>,
                         adds: &mut Vec<(usize, String)>| {
                let max_len = dels.len().max(adds.len());
                for j in 0..max_len {
                    let (left_num, left_content, left_kind) = if let Some((num, text)) = dels.get(j)
                    {
                        (Some(*num), text.clone(), SideBySideLineKind::Deletion)
                    } else {
                        (None, String::new(), SideBySideLineKind::Empty)
                    };
                    let (right_num, right_content, right_kind) =
                        if let Some((num, text)) = adds.get(j) {
                            (Some(*num), text.clone(), SideBySideLineKind::Addition)
                        } else {
                            (None, String::new(), SideBySideLineKind::Empty)
                        };
                    rows.push(SideBySideRow::Pair {
                        left_num,
                        left_content,
                        left_kind,
                        right_num,
                        right_content,
                        right_kind,
                    });
                }
                dels.clear();
                adds.clear();
            };

            for line in &hunk.lines {
                match line {
                    DiffLine::Context(text) => {
                        flush(&mut rows, &mut pending_dels, &mut pending_adds);
                        rows.push(SideBySideRow::Pair {
                            left_num: Some(old_line),
                            left_content: text.clone(),
                            left_kind: SideBySideLineKind::Context,
                            right_num: Some(new_line),
                            right_content: text.clone(),
                            right_kind: SideBySideLineKind::Context,
                        });
                        old_line += 1;
                        new_line += 1;
                    }
                    DiffLine::Deletion(text) => {
                        // If we had pending adds without dels, flush first
                        if !pending_adds.is_empty() && pending_dels.is_empty() {
                            flush(&mut rows, &mut pending_dels, &mut pending_adds);
                        }
                        pending_dels.push((old_line, text.clone()));
                        old_line += 1;
                    }
                    DiffLine::Addition(text) => {
                        pending_adds.push((new_line, text.clone()));
                        new_line += 1;
                    }
                }
            }
            flush(&mut rows, &mut pending_dels, &mut pending_adds);
        }
        rows
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

        // ── Empty state ──────────────────────────────────────────────
        if self.diff.is_none() {
            return div()
                .id("diff-viewer")
                .size_full()
                .bg(colors.editor_background)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(48.))
                                .h(px(48.))
                                .rounded(px(24.))
                                .bg(colors.element_background)
                                .child(
                                    Icon::new(IconName::GitCommit)
                                        .size(IconSize::Large)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            Label::new("Select a file or commit to view changes")
                                .size(LabelSize::Default)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Click a file in the sidebar or a commit in the graph")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                )
                .into_any_element();
        }

        let display_rows = self.display_rows.clone();
        let sbs_rows = self.sbs_rows.clone();
        let is_staged = self.is_staged;
        let display_mode = self.display_mode;
        let view: WeakEntity<DiffViewer> = cx.weak_entity();

        // ── Pre-compute colors for the closure ──────────────────────
        let editor_bg = colors.editor_background;
        let surface_bg = colors.surface_background;
        let text_color = colors.text;
        let text_placeholder_color = colors.text_placeholder;
        let border_variant = colors.border_variant;
        let vc_added = colors.vc_added;
        let vc_deleted = colors.vc_deleted;

        // Line backgrounds: subtle tinted backgrounds for add/delete lines
        let added_line_bg = gpui::Hsla {
            a: 0.08,
            ..vc_added
        };
        let deleted_line_bg = gpui::Hsla {
            a: 0.08,
            ..vc_deleted
        };

        // Gutter accent: slightly stronger tint for line number columns on changed lines
        let added_gutter_bg = gpui::Hsla {
            a: 0.18,
            ..vc_added
        };
        let deleted_gutter_bg = gpui::Hsla {
            a: 0.18,
            ..vc_deleted
        };

        // Hunk header: accent-tinted background to distinguish from content
        let hunk_header_bg = gpui::Hsla {
            a: 0.06,
            ..colors.text_accent
        };

        // Neutral gutter background for context lines
        let gutter_bg = gpui::Hsla {
            a: 0.03,
            ..colors.text
        };

        let row_height = 22.0_f32;

        // ── Build the list based on display mode ─────────────────────
        let list = match display_mode {
            DiffDisplayMode::Unified => {
                let view = view.clone();
                uniform_list(
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
                                            .bg(hunk_header_bg)
                                            .border_b_1()
                                            .border_color(border_variant)
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .italic()
                                                    .child(header_text),
                                            )
                                            .child(div().flex_1())
                                            .child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "hunk-stage-{}",
                                                        idx
                                                    )),
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
                                        let (prefix, text_col, line_bg, gutter_accent) = match kind
                                        {
                                            DisplayLineKind::Context => {
                                                (" ", text_color, editor_bg, gutter_bg)
                                            }
                                            DisplayLineKind::Addition => {
                                                ("+", vc_added, added_line_bg, added_gutter_bg)
                                            }
                                            DisplayLineKind::Deletion => {
                                                ("-", vc_deleted, deleted_line_bg, deleted_gutter_bg)
                                            }
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
                                            .bg(line_bg)
                                            // Old line number gutter
                                            .child(
                                                div()
                                                    .w(px(44.))
                                                    .flex_shrink_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_end()
                                                    .bg(gutter_accent)
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .px_1()
                                                    .child(old_str),
                                            )
                                            // New line number gutter
                                            .child(
                                                div()
                                                    .w(px(44.))
                                                    .flex_shrink_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_end()
                                                    .bg(gutter_accent)
                                                    .border_r_1()
                                                    .border_color(border_variant)
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .px_1()
                                                    .child(new_str),
                                            )
                                            // Content area
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .pl_2()
                                                    .text_xs()
                                                    .text_color(text_col)
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
                .track_scroll(&self.scroll_handle)
            }
            DiffDisplayMode::SideBySide => {
                let view = view.clone();
                uniform_list(
                    "diff-lines-sbs",
                    sbs_rows.len(),
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let row = &sbs_rows[i];
                                match row {
                                    SideBySideRow::HunkHeader { header, hunk_index } => {
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
                                                "sbs-hunk-header".into(),
                                                i as u64,
                                            ))
                                            .h_flex()
                                            .h(px(row_height))
                                            .w_full()
                                            .px_2()
                                            .items_center()
                                            .bg(hunk_header_bg)
                                            .border_b_1()
                                            .border_color(border_variant)
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .italic()
                                                    .child(header_text),
                                            )
                                            .child(div().flex_1())
                                            .child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "sbs-hunk-stage-{}",
                                                        idx
                                                    )),
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
                                    SideBySideRow::Pair {
                                        left_num,
                                        left_content,
                                        left_kind,
                                        right_num,
                                        right_content,
                                        right_kind,
                                    } => {
                                        let (left_bg, left_gutter_bg, left_text_col) =
                                            match left_kind {
                                                SideBySideLineKind::Context => {
                                                    (editor_bg, gutter_bg, text_color)
                                                }
                                                SideBySideLineKind::Deletion => {
                                                    (deleted_line_bg, deleted_gutter_bg, vc_deleted)
                                                }
                                                SideBySideLineKind::Addition => {
                                                    (added_line_bg, added_gutter_bg, vc_added)
                                                }
                                                SideBySideLineKind::Empty => {
                                                    (editor_bg, gutter_bg, text_placeholder_color)
                                                }
                                            };
                                        let (right_bg, right_gutter_bg, right_text_col) =
                                            match right_kind {
                                                SideBySideLineKind::Context => {
                                                    (editor_bg, gutter_bg, text_color)
                                                }
                                                SideBySideLineKind::Addition => {
                                                    (added_line_bg, added_gutter_bg, vc_added)
                                                }
                                                SideBySideLineKind::Deletion => {
                                                    (deleted_line_bg, deleted_gutter_bg, vc_deleted)
                                                }
                                                SideBySideLineKind::Empty => {
                                                    (editor_bg, gutter_bg, text_placeholder_color)
                                                }
                                            };

                                        let left_num_str: SharedString = left_num
                                            .map(|n| format!("{:>4}", n))
                                            .unwrap_or_else(|| "    ".to_string())
                                            .into();
                                        let right_num_str: SharedString = right_num
                                            .map(|n| format!("{:>4}", n))
                                            .unwrap_or_else(|| "    ".to_string())
                                            .into();
                                        let left_text: SharedString =
                                            left_content.trim_end().to_string().into();
                                        let right_text: SharedString =
                                            right_content.trim_end().to_string().into();

                                        div()
                                            .h_flex()
                                            .h(px(row_height))
                                            .w_full()
                                            // ── Left half ──
                                            .child(
                                                div()
                                                    .h_flex()
                                                    .flex_1()
                                                    .h_full()
                                                    .bg(left_bg)
                                                    // Left gutter
                                                    .child(
                                                        div()
                                                            .w(px(44.))
                                                            .flex_shrink_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .justify_end()
                                                            .bg(left_gutter_bg)
                                                            .border_r_1()
                                                            .border_color(border_variant)
                                                            .text_xs()
                                                            .text_color(text_placeholder_color)
                                                            .px_1()
                                                            .child(left_num_str),
                                                    )
                                                    // Left content
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .pl_2()
                                                            .text_xs()
                                                            .text_color(left_text_col)
                                                            .whitespace_nowrap()
                                                            .child(left_text),
                                                    ),
                                            )
                                            // ── Center divider ──
                                            .child(
                                                div()
                                                    .w(px(1.))
                                                    .flex_shrink_0()
                                                    .h_full()
                                                    .bg(border_variant),
                                            )
                                            // ── Right half ──
                                            .child(
                                                div()
                                                    .h_flex()
                                                    .flex_1()
                                                    .h_full()
                                                    .bg(right_bg)
                                                    // Right gutter
                                                    .child(
                                                        div()
                                                            .w(px(44.))
                                                            .flex_shrink_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .justify_end()
                                                            .bg(right_gutter_bg)
                                                            .border_r_1()
                                                            .border_color(border_variant)
                                                            .text_xs()
                                                            .text_color(text_placeholder_color)
                                                            .px_1()
                                                            .child(right_num_str),
                                                    )
                                                    // Right content
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .pl_2()
                                                            .text_xs()
                                                            .text_color(right_text_col)
                                                            .whitespace_nowrap()
                                                            .child(right_text),
                                                    ),
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
                .track_scroll(&self.scroll_handle)
            }
        };

        // ── Build container ─────────────────────────────────────────
        let mut container = div().id("diff-viewer").v_flex().size_full().bg(editor_bg);

        // ── File header bar ─────────────────────────────────────────
        if let Some(path) = &self.file_path {
            let (additions, deletions) = Self::count_changes(&self.display_rows);
            let file_icon = Self::icon_for_path(path);
            let path_str: SharedString = path.clone().into();
            let mode_label = match self.display_mode {
                DiffDisplayMode::Unified => "Split",
                DiffDisplayMode::SideBySide => "Unified",
            };

            container = container.child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .gap_2()
                    .items_center()
                    .bg(surface_bg)
                    .border_b_1()
                    .border_color(border_variant)
                    // File icon (based on extension)
                    .child(
                        Icon::new(file_icon)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    // File path
                    .child(
                        Label::new(path_str)
                            .size(LabelSize::Small)
                            .weight(FontWeight::MEDIUM)
                            .truncate(),
                    )
                    // Spacer
                    .child(div().flex_1())
                    // Change stats: +X in green, -Y in red
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(vc_added)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(format!("+{}", additions))),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(vc_deleted)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(format!("-{}", deletions))),
                            ),
                    )
                    // Separator
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_placeholder_color)
                            .child("\u{00B7}"),
                    )
                    // Mode toggle button
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
