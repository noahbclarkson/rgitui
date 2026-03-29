use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};



use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, ClipboardItem, Context, ElementId, EventEmitter,
    FocusHandle, FontStyle, FontWeight, HighlightStyle, KeyDownEvent, ListSizingBehavior,
    MouseButton, MouseDownEvent, Render, ScrollStrategy, SharedString, StyledText,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{DiffLine, FileDiff};
use rgitui_theme::{ActiveTheme, Appearance, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle as SyntectFontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

#[derive(Debug, Clone)]
pub enum DiffViewerEvent {
    HunkStageRequested(usize),
    HunkUnstageRequested(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffDisplayMode {
    #[default]
    Unified,
    SideBySide,
}

#[derive(Clone)]
enum DisplayRow {
    HunkHeader {
        header: String,
        context_name: String,
        hunk_index: usize,
    },
    Line {
        old_num: Option<usize>,
        new_num: Option<usize>,
        styled: StyledLine,
        kind: DisplayLineKind,
    },
}

#[derive(Clone)]
enum SideBySideRow {
    HunkHeader {
        header: String,
        context_name: String,
        hunk_index: usize,
    },
    Pair {
        left_num: Option<usize>,
        left_styled: StyledLine,
        left_kind: SideBySideLineKind,
        right_num: Option<usize>,
        right_styled: StyledLine,
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

#[derive(Clone, Default)]
struct StyledLine {
    text: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

impl StyledLine {
    fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            highlights: Vec::new(),
        }
    }

    /// Merge word-level highlight spans into the existing highlights list.
    /// `deletion_spans` are byte ranges of deleted words (highlighted red).
    /// `addition_spans` are byte ranges of inserted words (highlighted green).
    /// Word-level spans are appended after syntax spans; GPUI renders later
    /// highlights on top of earlier ones, so word-level takes visual priority.
    fn apply_word_highlights(
        &mut self,
        deletion_spans: Vec<Range<usize>>,
        addition_spans: Vec<Range<usize>>,
        deleted_word_bg: HighlightStyle,
        added_word_bg: HighlightStyle,
    ) {
        for span in deletion_spans {
            self.highlights.push((span, deleted_word_bg));
        }
        for span in addition_spans {
            self.highlights.push((span, added_word_bg));
        }
    }
}

struct SyntaxAssets {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

enum SyntaxLineHighlighter {
    Plain,
    Syntect {
        syntax_set: &'static SyntaxSet,
        highlighter: Box<HighlightLines<'static>>,
    },
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
    focus_handle: FocusHandle,
    highlighted_row: Option<usize>,
    selected_lines: Option<Range<usize>>,
    selection_anchor: Option<usize>,
    /// Tracks the current theme appearance to drive syntax highlighting.
    current_appearance: Appearance,
    /// Top item index to restore after a display mode switch.
    pending_scroll_top: Option<usize>,
}

impl EventEmitter<DiffViewerEvent> for DiffViewer {}

impl DiffViewer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            diff: None,
            display_mode: DiffDisplayMode::Unified,
            file_path: None,
            is_staged: false,
            display_rows: Arc::new(Vec::new()),
            sbs_rows: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            highlighted_row: None,
            selected_lines: None,
            selection_anchor: None,
            current_appearance: Appearance::Dark,
            pending_scroll_top: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn set_diff(&mut self, diff: FileDiff, path: String, staged: bool, cx: &mut Context<Self>) {
        let appearance = cx.theme().appearance;
        let colors = cx.colors();
        let added_word_bg = HighlightStyle {
            background_color: Some(gpui::Hsla {
                a: 0.25,
                ..colors.vc_added
            }),
            ..Default::default()
        };
        let deleted_word_bg = HighlightStyle {
            background_color: Some(gpui::Hsla {
                a: 0.25,
                ..colors.vc_deleted
            }),
            ..Default::default()
        };
        self.current_appearance = appearance;
        self.display_rows = Arc::new(Self::compute_display_rows(
            &diff,
            &path,
            appearance,
            added_word_bg,
            deleted_word_bg,
        ));
        self.sbs_rows = Arc::new(Self::compute_sbs_rows(&diff, &path, appearance));
        self.diff = Some(diff);
        self.file_path = Some(path);
        self.is_staged = staged;
        self.highlighted_row = None;
        self.selected_lines = None;
        self.selection_anchor = None;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.file_path = None;
        self.display_rows = Arc::new(Vec::new());
        self.sbs_rows = Arc::new(Vec::new());
        self.highlighted_row = None;
        self.selected_lines = None;
        self.selection_anchor = None;
        cx.notify();
    }

    fn handle_line_click(&mut self, row_ix: usize, shift: bool, cx: &mut Context<Self>) {
        if shift {
            if let Some(anchor) = self.selection_anchor {
                let start = anchor.min(row_ix);
                let end = anchor.max(row_ix) + 1;
                self.selected_lines = Some(start..end);
            } else {
                self.selection_anchor = Some(row_ix);
                self.selected_lines = Some(row_ix..row_ix + 1);
            }
        } else {
            self.selection_anchor = Some(row_ix);
            self.selected_lines = Some(row_ix..row_ix + 1);
        }
        cx.notify();
    }

    fn copy_selected_lines(&self, cx: &mut Context<Self>) {
        let range = match &self.selected_lines {
            Some(r) => r.clone(),
            None => return,
        };

        let mut text = String::new();
        match self.display_mode {
            DiffDisplayMode::Unified => {
                for i in range {
                    if i < self.display_rows.len() {
                        match &self.display_rows[i] {
                            DisplayRow::HunkHeader { header, .. } => {
                                text.push_str(header);
                                text.push('\n');
                            }
                            DisplayRow::Line { styled, .. } => {
                                text.push_str(styled.text.as_ref());
                                text.push('\n');
                            }
                        }
                    }
                }
            }
            DiffDisplayMode::SideBySide => {
                for i in range {
                    if i < self.sbs_rows.len() {
                        match &self.sbs_rows[i] {
                            SideBySideRow::HunkHeader { header, .. } => {
                                text.push_str(header);
                                text.push('\n');
                            }
                            SideBySideRow::Pair {
                                left_styled,
                                left_kind,
                                right_styled,
                                right_kind,
                                ..
                            } => {
                                if *left_kind != SideBySideLineKind::Empty {
                                    text.push_str(left_styled.text.as_ref());
                                }
                                if *left_kind != SideBySideLineKind::Empty
                                    && *right_kind != SideBySideLineKind::Empty
                                    && *left_kind != SideBySideLineKind::Context
                                {
                                    text.push('\t');
                                }
                                if *right_kind != SideBySideLineKind::Empty
                                    && *right_kind != SideBySideLineKind::Context
                                {
                                    text.push_str(right_styled.text.as_ref());
                                }
                                text.push('\n');
                            }
                        }
                    }
                }
            }
        }

        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn row_count(&self) -> usize {
        match self.display_mode {
            DiffDisplayMode::Unified => self.display_rows.len(),
            DiffDisplayMode::SideBySide => self.sbs_rows.len(),
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let ctrl = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let row_count = self.row_count();

        if row_count == 0 {
            return;
        }

        match key {
            "j" | "down" if !ctrl => {
                let next = match self.highlighted_row {
                    Some(i) if i + 1 < row_count => i + 1,
                    None => 0,
                    Some(i) => i,
                };
                self.highlighted_row = Some(next);
                self.scroll_handle.scroll_to_item(next, ScrollStrategy::Top);
                cx.notify();
            }
            "k" | "up" if !ctrl => {
                let next = match self.highlighted_row {
                    Some(i) if i > 0 => i - 1,
                    None if row_count > 0 => 0,
                    Some(i) => i,
                    None => 0,
                };
                self.highlighted_row = Some(next);
                self.scroll_handle.scroll_to_item(next, ScrollStrategy::Top);
                cx.notify();
            }
            "g" if !ctrl && !event.keystroke.modifiers.shift => {
                self.highlighted_row = Some(0);
                self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
            "g" if event.keystroke.modifiers.shift => {
                let last = row_count.saturating_sub(1);
                self.highlighted_row = Some(last);
                self.scroll_handle.scroll_to_item(last, ScrollStrategy::Top);
                cx.notify();
            }
            "]" if !ctrl => {
                // Jump to next hunk header after current position
                let start = self.highlighted_row.map(|r| r + 1).unwrap_or(0);
                let next = match self.display_mode {
                    DiffDisplayMode::Unified => (start..row_count)
                        .find(|&i| matches!(self.display_rows[i], DisplayRow::HunkHeader { .. })),
                    DiffDisplayMode::SideBySide => (start..row_count)
                        .find(|&i| matches!(self.sbs_rows[i], SideBySideRow::HunkHeader { .. })),
                }
                // Wrap around
                .or_else(|| match self.display_mode {
                    DiffDisplayMode::Unified => (0..start)
                        .find(|&i| matches!(self.display_rows[i], DisplayRow::HunkHeader { .. })),
                    DiffDisplayMode::SideBySide => (0..start)
                        .find(|&i| matches!(self.sbs_rows[i], SideBySideRow::HunkHeader { .. })),
                });
                if let Some(pos) = next {
                    self.highlighted_row = Some(pos);
                    self.scroll_handle.scroll_to_item(pos, ScrollStrategy::Top);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "[" if !ctrl => {
                // Jump to previous hunk header before current position
                let end = self.highlighted_row.unwrap_or(row_count);
                let prev = match self.display_mode {
                    DiffDisplayMode::Unified => (0..end)
                        .rev()
                        .find(|&i| matches!(self.display_rows[i], DisplayRow::HunkHeader { .. })),
                    DiffDisplayMode::SideBySide => (0..end)
                        .rev()
                        .find(|&i| matches!(self.sbs_rows[i], SideBySideRow::HunkHeader { .. })),
                }
                // Wrap around
                .or_else(|| match self.display_mode {
                    DiffDisplayMode::Unified => (end..row_count)
                        .rev()
                        .find(|&i| matches!(self.display_rows[i], DisplayRow::HunkHeader { .. })),
                    DiffDisplayMode::SideBySide => (end..row_count)
                        .rev()
                        .find(|&i| matches!(self.sbs_rows[i], SideBySideRow::HunkHeader { .. })),
                });
                if let Some(pos) = prev {
                    self.highlighted_row = Some(pos);
                    self.scroll_handle.scroll_to_item(pos, ScrollStrategy::Top);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "d" if !ctrl => {
                self.toggle_display_mode(cx);
            }
            "c" if ctrl => {
                self.copy_selected_lines(cx);
            }
            "a" if ctrl && row_count > 0 => {
                self.selection_anchor = Some(0);
                self.selected_lines = Some(0..row_count);
                cx.notify();
            }
            "s" | "S" if !event.keystroke.modifiers.alt && !ctrl => {
                // s: stage hunks under selection (or cursor hunk if no selection)
                if !self.is_staged {
                    let hunks = if let Some(sel) = &self.selected_lines {
                        self.hunks_under_selection(sel.clone())
                    } else {
                        self.current_hunk_index()
                            .map(|i| vec![i])
                            .unwrap_or_default()
                    };
                    for idx in hunks {
                        cx.emit(DiffViewerEvent::HunkStageRequested(idx));
                    }
                }
                cx.stop_propagation();
            }
            "u" | "U" if !event.keystroke.modifiers.alt && !ctrl => {
                // u: unstage hunks under selection (or cursor hunk if no selection)
                if self.is_staged {
                    let hunks = if let Some(sel) = &self.selected_lines {
                        self.hunks_under_selection(sel.clone())
                    } else {
                        self.current_hunk_index()
                            .map(|i| vec![i])
                            .unwrap_or_default()
                    };
                    for idx in hunks {
                        cx.emit(DiffViewerEvent::HunkUnstageRequested(idx));
                    }
                }
                cx.stop_propagation();
            }
            "s" | "S" if event.keystroke.modifiers.alt && !ctrl => {
                // Alt+S: stage the current hunk
                if !self.is_staged {
                    if let Some(idx) = self.current_hunk_index() {
                        cx.emit(DiffViewerEvent::HunkStageRequested(idx));
                    }
                }
                cx.stop_propagation();
            }
            "u" | "U" if event.keystroke.modifiers.alt && !ctrl => {
                // Alt+U: unstage the current hunk
                if self.is_staged {
                    if let Some(idx) = self.current_hunk_index() {
                        cx.emit(DiffViewerEvent::HunkUnstageRequested(idx));
                    }
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// Returns the hunk index at or before the currently highlighted row.
    /// If the highlighted row itself is a hunk header, returns its index.
    /// Otherwise searches backwards to find the nearest preceding hunk.
    fn current_hunk_index(&self) -> Option<usize> {
        let pos = self.highlighted_row?;
        match self.display_mode {
            DiffDisplayMode::Unified => {
                // Check current position first
                if let DisplayRow::HunkHeader { hunk_index, .. } = &self.display_rows[pos] {
                    return Some(*hunk_index);
                }
                // Search backwards for nearest hunk header
                (0..pos).rev().find_map(|i| {
                    if let DisplayRow::HunkHeader { hunk_index, .. } = &self.display_rows[i] {
                        Some(*hunk_index)
                    } else {
                        None
                    }
                })
            }
            DiffDisplayMode::SideBySide => {
                if let SideBySideRow::HunkHeader { hunk_index, .. } = &self.sbs_rows[pos] {
                    return Some(*hunk_index);
                }
                (0..pos).rev().find_map(|i| {
                    if let SideBySideRow::HunkHeader { hunk_index, .. } = &self.sbs_rows[i] {
                        Some(*hunk_index)
                    } else {
                        None
                    }
                })
            }
        }
    }

    /// Returns all unique hunk indices that have at least one row within `range`.
    /// Works by tracking the current hunk as we iterate through display rows,
    /// emitting the hunk index each time we encounter a hunk header within the range.
    fn hunks_under_selection(&self, range: Range<usize>) -> Vec<usize> {
        let mut hunks = Vec::new();
        let mut seen = HashSet::new();
        let start = range.start;
        let end = range.end.min(self.row_count());

        match &self.display_mode {
            DiffDisplayMode::Unified => {
                let rows = &self.display_rows;
                let mut current_hunk: Option<usize> = None;
                for i in start..end {
                    if let DisplayRow::HunkHeader { hunk_index, .. } = &rows[i] {
                        current_hunk = Some(*hunk_index);
                    } else if let Some(h) = current_hunk {
                        if seen.insert(h) {
                            hunks.push(h);
                        }
                    }
                }
            }
            DiffDisplayMode::SideBySide => {
                let rows = &self.sbs_rows;
                let mut current_hunk: Option<usize> = None;
                for i in start..end {
                    if let SideBySideRow::HunkHeader { hunk_index, .. } = &rows[i] {
                        current_hunk = Some(*hunk_index);
                    } else if let Some(h) = current_hunk {
                        if seen.insert(h) {
                            hunks.push(h);
                        }
                    }
                }
            }
        }
        hunks
    }

    pub fn toggle_display_mode(&mut self, cx: &mut Context<Self>) {
        // Capture the current top item so we can restore scroll position after the switch.
        let top_item = self.scroll_handle.0.borrow().base_handle.top_item();
        self.pending_scroll_top = Some(top_item);
        self.display_mode = match self.display_mode {
            DiffDisplayMode::Unified => DiffDisplayMode::SideBySide,
            DiffDisplayMode::SideBySide => DiffDisplayMode::Unified,
        };
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

    fn icon_for_path(path: &str) -> IconName {
        if let Some(ext) = path.rsplit('.').next() {
            match ext {
                "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "c"
                | "cpp" | "h" | "hpp" | "go" | "java" | "rb" | "sh" | "lua" | "zig" | "swift"
                | "kt" | "kts" | "cs" | "fs" | "ex" | "exs" | "hs" | "ml" | "elm" | "dart"
                | "r" | "scala" | "clj" | "erl" | "v" | "odin" | "proto" | "sql" | "vue"
                | "svelte" | "astro" | "php" | "pl" | "d" => IconName::File,
                "toml" | "yaml" | "yml" | "json" | "jsonc" | "json5" | "xml" | "ini" | "conf"
                | "cfg" | "env" | "hcl" | "tf" | "graphql" | "gql" | "prisma" => IconName::Settings,
                "css" | "scss" | "sass" | "less" | "styl" | "pcss" => IconName::File,
                "html" | "htm" | "hbs" | "ejs" | "njk" => IconName::File,
                "md" | "txt" | "rst" | "org" => IconName::File,
                "lock" => IconName::Pin,
                _ => IconName::File,
            }
        } else {
            IconName::File
        }
    }

    /// Extract the function/context name from a hunk header.
    /// Hunk headers look like `@@ -10,5 +10,7 @@ fn some_function(...)`
    /// This returns the part after the closing `@@`.
    fn extract_context_name(header: &str) -> String {
        if let Some(pos) = header.find("@@") {
            let after_first = &header[pos + 2..];
            if let Some(pos2) = after_first.find("@@") {
                let context = after_first[pos2 + 2..].trim();
                if !context.is_empty() {
                    return context.to_string();
                }
            }
        }
        String::new()
    }

    /// Extract the line range portion from a hunk header (the `@@ -x,y +x,y @@` part).
    fn extract_line_range(header: &str) -> String {
        if let Some(start) = header.find("@@") {
            let after_first = &header[start..];
            if let Some(end) = after_first[2..].find("@@") {
                return after_first[..end + 4].to_string();
            }
        }
        header.to_string()
    }

    fn syntax_assets() -> &'static SyntaxAssets {
        static ASSETS: OnceLock<SyntaxAssets> = OnceLock::new();
        ASSETS.get_or_init(|| SyntaxAssets {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        })
    }

    fn syntax_theme(appearance: Appearance) -> &'static Theme {
        let assets = Self::syntax_assets();
        let preferred_name = match appearance {
            Appearance::Dark => "base16-ocean.dark",
            Appearance::Light => "base16-ocean.light",
        };
        assets
            .theme_set
            .themes
            .get(preferred_name)
            .or_else(|| {
                // Graceful fallback: for light pick any non-dark theme, else any
                if appearance == Appearance::Light {
                    assets
                        .theme_set
                        .themes
                        .iter()
                        .find(|(k, _)| k.contains("light") || k.contains("Light"))
                        .map(|(_, v)| v)
                } else {
                    None
                }
            })
            .or_else(|| assets.theme_set.themes.values().next())
            .expect("syntect theme set should contain at least one theme")
    }

    fn syntax_for_path(path: &str) -> Option<&'static SyntaxReference> {
        let assets = Self::syntax_assets();
        assets
            .syntax_set
            .find_syntax_for_file(Path::new(path))
            .ok()
            .flatten()
            .or_else(|| {
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| assets.syntax_set.find_syntax_by_token(name))
            })
            .or_else(|| {
                // Fallback: map common extensions that syntect's defaults don't cover
                // to the closest available syntax grammar.
                let ext = Path::new(path).extension()?.to_str()?;
                let fallback_name = match ext {
                    // TypeScript / JSX / TSX → JavaScript
                    "ts" | "tsx" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => "JavaScript",
                    // Markup variants → HTML
                    "vue" | "svelte" | "astro" | "hbs" | "ejs" | "njk" | "liquid" => "HTML",
                    // Style variants → CSS
                    "scss" | "sass" | "less" | "styl" | "pcss" | "postcss" => "CSS",
                    // Config / data → JSON or YAML
                    "jsonc" | "json5" | "geojson" | "webmanifest" | "eslintrc" | "prettierrc"
                    | "babelrc" => "JSON",
                    "toml" | "ini" | "cfg" | "conf" | "env" | "properties" | "editorconfig" => {
                        // TOML/INI are closest to YAML in structure for basic highlighting
                        "YAML"
                    }
                    // Shell variants → Bash
                    "ps1" | "psm1" | "psd1" | "nu" => "Bourne Again Shell (bash)",
                    // Docker / container
                    "dockerfile" => "Makefile",
                    // Systems languages → C/C++
                    "zig" | "odin" | "v" => "C",
                    "kt" | "kts" => "Java",
                    "swift" => "Objective-C",
                    "dart" => "Java",
                    // Functional languages
                    "ex" | "exs" | "eex" | "heex" | "leex" => "Ruby",
                    "elm" | "hs" | "purs" => "Haskell",
                    "fs" | "fsx" | "fsi" => "C#",
                    "ml" | "mli" | "re" | "rei" => "OCaml",
                    // Other
                    "proto" | "protobuf" => "C",
                    "graphql" | "gql" => "JavaScript",
                    "tf" | "hcl" => "JSON",
                    "cmake" => "Makefile",
                    "r" | "rmd" => "R",
                    "sql" | "pgsql" | "mysql" | "tsql" => "SQL",
                    _ => return None,
                };
                assets.syntax_set.find_syntax_by_name(fallback_name)
            })
            .or_else(|| {
                // Final fallback: match known filenames without extensions
                let filename = Path::new(path).file_name()?.to_str()?;
                let fallback_name = match filename {
                    "Dockerfile" | "Containerfile" | "Justfile" | "Brewfile" => "Makefile",
                    ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zprofile" | ".env"
                    | ".envrc" => "Bourne Again Shell (bash)",
                    ".gitignore" | ".dockerignore" | ".prettierignore" | ".eslintignore" => {
                        "Bourne Again Shell (bash)"
                    }
                    "tsconfig.json" | "package.json" | "composer.json" | ".swcrc" => "JSON",
                    "CMakeLists.txt" => "Makefile",
                    _ => return None,
                };
                assets.syntax_set.find_syntax_by_name(fallback_name)
            })
    }

    fn syntax_line_highlighter(path: &str, appearance: Appearance) -> SyntaxLineHighlighter {
        let assets = Self::syntax_assets();
        let Some(syntax) = Self::syntax_for_path(path) else {
            return SyntaxLineHighlighter::Plain;
        };

        SyntaxLineHighlighter::Syntect {
            syntax_set: &assets.syntax_set,
            highlighter: Box::new(HighlightLines::new(syntax, Self::syntax_theme(appearance))),
        }
    }

    fn syntect_style_to_highlight(style: syntect::highlighting::Style) -> HighlightStyle {
        let foreground = style.foreground;
        let mut highlight = HighlightStyle {
            color: Some(rgitui_theme::hex_to_hsla(&format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                foreground.r, foreground.g, foreground.b, foreground.a
            ))),
            ..Default::default()
        };

        if style.font_style.contains(SyntectFontStyle::BOLD) {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if style.font_style.contains(SyntectFontStyle::ITALIC) {
            highlight.font_style = Some(FontStyle::Italic);
        }

        highlight
    }

    fn highlight_text(text: &str, highlighter: &mut SyntaxLineHighlighter) -> StyledLine {
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return StyledLine::plain(trimmed.to_string());
        }

        match highlighter {
            SyntaxLineHighlighter::Plain => StyledLine::plain(trimmed.to_string()),
            SyntaxLineHighlighter::Syntect {
                syntax_set,
                highlighter,
            } => {
                let Ok(ranges) = highlighter.highlight_line(trimmed, syntax_set) else {
                    return StyledLine::plain(trimmed.to_string());
                };

                let mut highlights = Vec::new();
                let mut cursor = 0usize;
                for (style, segment) in ranges.into_iter() {
                    let segment: &str = segment;
                    let len = segment.len();
                    if len == 0 {
                        continue;
                    }
                    highlights.push((
                        cursor..cursor + len,
                        Self::syntect_style_to_highlight(style),
                    ));
                    cursor += len;
                }

                StyledLine {
                    text: trimmed.to_string().into(),
                    highlights,
                }
            }
        }
    }

    fn render_styled_text(
        window: &Window,
        text: &StyledLine,
        default_color: gpui::Hsla,
    ) -> gpui::AnyElement {
        let mut text_style = window.text_style();
        text_style.color = default_color;

        if text.highlights.is_empty() {
            div()
                .child(StyledText::new(text.text.clone()))
                .into_any_element()
        } else {
            div()
                .child(
                    StyledText::new(text.text.clone())
                        .with_default_highlights(&text_style, text.highlights.clone()),
                )
                .into_any_element()
        }
    }

    fn compute_sbs_rows(diff: &FileDiff, path: &str, appearance: Appearance) -> Vec<SideBySideRow> {
        let mut rows = Vec::new();
        for (i, hunk) in diff.hunks.iter().enumerate() {
            let header_str = hunk.header.trim().to_string();
            rows.push(SideBySideRow::HunkHeader {
                context_name: Self::extract_context_name(&header_str),
                header: header_str,
                hunk_index: i,
            });
            let mut old_line = hunk.old_start as usize;
            let mut new_line = hunk.new_start as usize;
            let mut highlighter = Self::syntax_line_highlighter(path, appearance);

            let mut pending_dels: Vec<(usize, StyledLine)> = Vec::new();
            let mut pending_adds: Vec<(usize, StyledLine)> = Vec::new();

            let flush = |rows: &mut Vec<SideBySideRow>,
                         dels: &mut Vec<(usize, StyledLine)>,
                         adds: &mut Vec<(usize, StyledLine)>| {
                let max_len = dels.len().max(adds.len());
                for j in 0..max_len {
                    let (left_num, left_styled, left_kind) =
                        if let Some((num, styled)) = dels.get(j) {
                            (Some(*num), styled.clone(), SideBySideLineKind::Deletion)
                        } else {
                            (None, StyledLine::plain(""), SideBySideLineKind::Empty)
                        };
                    let (right_num, right_styled, right_kind) =
                        if let Some((num, styled)) = adds.get(j) {
                            (Some(*num), styled.clone(), SideBySideLineKind::Addition)
                        } else {
                            (None, StyledLine::plain(""), SideBySideLineKind::Empty)
                        };
                    rows.push(SideBySideRow::Pair {
                        left_num,
                        left_styled,
                        left_kind,
                        right_num,
                        right_styled,
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
                        let styled = Self::highlight_text(text, &mut highlighter);
                        rows.push(SideBySideRow::Pair {
                            left_num: Some(old_line),
                            left_styled: styled.clone(),
                            left_kind: SideBySideLineKind::Context,
                            right_num: Some(new_line),
                            right_styled: styled,
                            right_kind: SideBySideLineKind::Context,
                        });
                        old_line += 1;
                        new_line += 1;
                    }
                    DiffLine::Deletion(text) => {
                        if !pending_adds.is_empty() && pending_dels.is_empty() {
                            flush(&mut rows, &mut pending_dels, &mut pending_adds);
                        }
                        pending_dels.push((old_line, Self::highlight_text(text, &mut highlighter)));
                        old_line += 1;
                    }
                    DiffLine::Addition(text) => {
                        pending_adds.push((new_line, Self::highlight_text(text, &mut highlighter)));
                        new_line += 1;
                    }
                }
            }
            flush(&mut rows, &mut pending_dels, &mut pending_adds);
        }
        rows
    }

    fn compute_display_rows(
        diff: &FileDiff,
        path: &str,
        appearance: Appearance,
        added_word_bg: HighlightStyle,
        deleted_word_bg: HighlightStyle,
    ) -> Vec<DisplayRow> {
        let mut rows = Vec::new();
        for (i, hunk) in diff.hunks.iter().enumerate() {
            let header_str = hunk.header.trim().to_string();
            rows.push(DisplayRow::HunkHeader {
                context_name: Self::extract_context_name(&header_str),
                header: header_str,
                hunk_index: i,
            });
            let mut old_line = hunk.old_start as usize;
            let mut new_line = hunk.new_start as usize;
            let mut highlighter = Self::syntax_line_highlighter(path, appearance);

            // Buffer for pending deletions: (old_line_num, text, styled_line)
            let mut pending_dels: Vec<(usize, String, StyledLine)> = Vec::new();

            let flush_dels =
                |rows: &mut Vec<DisplayRow>, dels: &mut Vec<(usize, String, StyledLine)>| {
                    for (ln, _text, styled) in dels.drain(..) {
                        rows.push(DisplayRow::Line {
                            old_num: Some(ln),
                            new_num: None,
                            styled,
                            kind: DisplayLineKind::Deletion,
                        });
                    }
                };

            for line in &hunk.lines {
                match line {
                    DiffLine::Context(text) => {
                        flush_dels(&mut rows, &mut pending_dels);
                        rows.push(DisplayRow::Line {
                            old_num: Some(old_line),
                            new_num: Some(new_line),
                            styled: Self::highlight_text(text, &mut highlighter),
                            kind: DisplayLineKind::Context,
                        });
                        old_line += 1;
                        new_line += 1;
                    }
                    DiffLine::Deletion(text) => {
                        pending_dels.push((
                            old_line,
                            text.clone(),
                            Self::highlight_text(text, &mut highlighter),
                        ));
                        old_line += 1;
                    }
                    DiffLine::Addition(text) => {
                        let add_styled = Self::highlight_text(text, &mut highlighter);
                        if let Some((del_line, del_text, mut del_styled)) = pending_dels.pop() {
                            // Pair deletion + addition → compute word-level highlights
                            let (del_spans, add_spans) = Self::compute_word_diff(&del_text, text);
                            del_styled.apply_word_highlights(
                                del_spans,
                                Vec::new(),
                                deleted_word_bg,
                                added_word_bg,
                            );
                            let mut add_styled = add_styled;
                            add_styled.apply_word_highlights(
                                Vec::new(),
                                add_spans,
                                deleted_word_bg,
                                added_word_bg,
                            );
                            rows.push(DisplayRow::Line {
                                old_num: Some(del_line),
                                new_num: None,
                                styled: del_styled,
                                kind: DisplayLineKind::Deletion,
                            });
                            rows.push(DisplayRow::Line {
                                old_num: None,
                                new_num: Some(new_line),
                                styled: add_styled,
                                kind: DisplayLineKind::Addition,
                            });
                        } else {
                            // Unpaired addition
                            rows.push(DisplayRow::Line {
                                old_num: None,
                                new_num: Some(new_line),
                                styled: add_styled,
                                kind: DisplayLineKind::Addition,
                            });
                        }
                        new_line += 1;
                    }
                }
            }
            flush_dels(&mut rows, &mut pending_dels);
        }
        rows
    }

    /// DISABLED: word-level diff is temporarily disabled while investigating
    /// STATUS_STACK_BUFFER_OVERRUN on Windows (crash on commit click, all interactions).
    /// Returns empty spans until the root cause is identified and fixed.
    #[allow(dead_code)]
    pub(crate) fn compute_word_diff(
        _old_text: &str,
        _new_text: &str,
    ) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
        (Vec::new(), Vec::new())
    }

    /// Split `text` into words, returning each word's byte-offset range.
    #[allow(dead_code)]
    pub(crate) fn split_word_ranges(text: &str) -> Vec<Range<usize>> {
        let mut result = Vec::new();
        let mut word_start: Option<usize> = None;

        for (i, ch) in text.char_indices() {
            if ch.is_whitespace() {
                if let Some(start) = word_start.take() {
                    result.push(start..i);
                }
            } else if word_start.is_none() {
                word_start = Some(i);
            }
        }
        if let Some(start) = word_start {
            result.push(start..text.len());
        }
        result
    }
}

impl Render for DiffViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        // Re-render with syntax highlighting if the theme appearance changed while a diff
        // was open. Without this, switching themes (e.g. Dark → Light) leaves the stored
        // rows with the old syntect palette until the next `set_diff` call.
        if self.diff.is_some() {
            let appearance = cx.theme().appearance;
            if appearance != self.current_appearance {
                let added_word_bg = HighlightStyle {
                    background_color: Some(gpui::Hsla {
                        a: 0.25,
                        ..colors.vc_added
                    }),
                    ..Default::default()
                };
                let deleted_word_bg = HighlightStyle {
                    background_color: Some(gpui::Hsla {
                        a: 0.25,
                        ..colors.vc_deleted
                    }),
                    ..Default::default()
                };
                self.current_appearance = appearance;
                if let Some(ref diff) = self.diff {
                    if let Some(ref path) = self.file_path {
                        self.display_rows = Arc::new(Self::compute_display_rows(
                            diff,
                            path,
                            appearance,
                            added_word_bg,
                            deleted_word_bg,
                        ));
                        self.sbs_rows = Arc::new(Self::compute_sbs_rows(diff, path, appearance));
                    }
                }
            }
        }

        if self.diff.is_none() {
            return div()
                .id("diff-viewer")
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
                            Icon::new(IconName::File)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Diff")
                                .size(LabelSize::XSmall)
                                .weight(FontWeight::SEMIBOLD)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    div().flex_1().flex().items_center().justify_center().child(
                        div()
                            .v_flex()
                            .gap(px(12.))
                            .items_center()
                            .px(px(32.))
                            .py(px(24.))
                            .rounded(px(8.))
                            .bg(colors.element_background)
                            .child(
                                Icon::new(IconName::File)
                                    .size(IconSize::Large)
                                    .color(Color::Placeholder),
                            )
                            .child(
                                Label::new("Select a file to view changes")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("Click a file in the sidebar or detail panel")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                            ),
                    ),
                )
                .into_any_element();
        }

        let display_rows = self.display_rows.clone();
        let sbs_rows = self.sbs_rows.clone();
        let is_staged = self.is_staged;
        let display_mode = self.display_mode;
        let view: WeakEntity<DiffViewer> = cx.weak_entity();

        let editor_bg = colors.editor_background;
        let text_color = colors.text;
        let text_muted = colors.text_muted;
        let text_placeholder_color = colors.text_placeholder;
        let border_variant = colors.border_variant;
        let vc_added = colors.vc_added;
        let vc_deleted = colors.vc_deleted;
        let element_bg = colors.element_background;

        let added_line_bg = gpui::Hsla {
            a: 0.12,
            ..vc_added
        };
        let deleted_line_bg = gpui::Hsla {
            a: 0.12,
            ..vc_deleted
        };

        let added_gutter_bg = gpui::Hsla {
            a: 0.20,
            ..vc_added
        };
        let deleted_gutter_bg = gpui::Hsla {
            a: 0.20,
            ..vc_deleted
        };

        let empty_fill_bg = gpui::Hsla {
            a: 0.04,
            ..text_color
        };

        let gutter_bg = gpui::Hsla {
            a: 0.04,
            ..text_color
        };

        let compactness = cx
            .global::<rgitui_settings::SettingsState>()
            .settings()
            .compactness;
        let row_height = compactness.spacing(20.0);
        let hunk_header_height = row_height;

        let highlighted_row = self.highlighted_row;
        let highlight_bg = gpui::Hsla {
            a: 0.10,
            ..colors.text_accent
        };
        let row_hover_bg = gpui::Hsla {
            a: 0.04,
            ..text_color
        };
        let selection_bg = gpui::Hsla {
            a: 0.15,
            ..colors.text_accent
        };
        let selected_lines = self.selected_lines.clone();

        // Restore scroll position after a display mode switch. The deferred scroll is
        // applied by GPUI's layout pass before the list content is painted, so the
        // user sees the correct scroll position immediately.
        if let Some(top_ix) = self.pending_scroll_top.take() {
            let target_ix = top_ix.min(self.row_count().saturating_sub(1));
            self.scroll_handle
                .scroll_to_item(target_ix, ScrollStrategy::Top);
        }

        let list = match display_mode {
            DiffDisplayMode::Unified => {
                let view = view.clone();
                uniform_list(
                    "diff-lines",
                    display_rows.len(),
                    move |range: Range<usize>, window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let row = &display_rows[i];
                                match row {
                                    DisplayRow::HunkHeader {
                                        header,
                                        context_name,
                                        hunk_index,
                                    } => {
                                        let line_range: SharedString =
                                            Self::extract_line_range(header).into();
                                        let ctx_name: SharedString = context_name.clone().into();
                                        let has_context = !context_name.is_empty();
                                        let stage_label = if is_staged {
                                            "Unstage Hunk"
                                        } else {
                                            "Stage Hunk"
                                        };
                                        let idx = *hunk_index;
                                        let view_clone = view.clone();
                                        let view_hunk = view.clone();
                                        let is_hunk_selected = selected_lines
                                            .as_ref()
                                            .is_some_and(|r| r.contains(&i));
                                        let hunk_bg = if is_hunk_selected {
                                            selection_bg
                                        } else {
                                            element_bg
                                        };

                                        let mut hunk_row = div()
                                            .id(ElementId::NamedInteger(
                                                "hunk-header".into(),
                                                i as u64,
                                            ))
                                            .h_flex()
                                            .h(px(hunk_header_height))
                                            .w_full()
                                            .px(px(8.))
                                            .py(px(4.))
                                            .items_center()
                                            .gap(px(8.))
                                            .bg(hunk_bg)
                                            .border_t_1()
                                            .border_b_1()
                                            .border_color(border_variant)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |event: &MouseDownEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    let shift = event.modifiers.shift;
                                                    view_hunk
                                                        .update(cx, |this, cx| {
                                                            this.handle_line_click(i, shift, cx);
                                                        })
                                                        .ok();
                                                },
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_family("monospace")
                                                    .text_color(text_muted)
                                                    .child(line_range),
                                            );

                                        if has_context {
                                            hunk_row = hunk_row.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .italic()
                                                    .child(ctx_name),
                                            );
                                        }

                                        hunk_row
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
                                        styled,
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
                                        let prefix_str: SharedString = prefix.into();
                                        let is_highlighted = highlighted_row == Some(i);
                                        let is_selected = selected_lines
                                            .as_ref()
                                            .is_some_and(|r| r.contains(&i));
                                        let effective_bg = if is_selected {
                                            selection_bg
                                        } else if is_highlighted {
                                            highlight_bg
                                        } else {
                                            line_bg
                                        };

                                        let view_line = view.clone();
                                        div()
                                            .id(ElementId::NamedInteger(
                                                "diff-line".into(),
                                                i as u64,
                                            ))
                                            .h_flex()
                                            .h(px(row_height))
                                            .w_full()
                                            .bg(effective_bg)
                                            .hover(move |s| s.bg(row_hover_bg))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |event: &MouseDownEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    let shift = event.modifiers.shift;
                                                    view_line
                                                        .update(cx, |this, cx| {
                                                            this.handle_line_click(i, shift, cx);
                                                        })
                                                        .ok();
                                                },
                                            )
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
                                                    .font_family("monospace")
                                                    .text_color(text_muted)
                                                    .pr(px(4.))
                                                    .child(old_str),
                                            )
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
                                                    .font_family("monospace")
                                                    .text_color(text_muted)
                                                    .pr(px(4.))
                                                    .child(new_str),
                                            )
                                            .child(
                                                div()
                                                    .w(px(18.))
                                                    .flex_shrink_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_xs()
                                                    .font_family("monospace")
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(text_col)
                                                    .child(prefix_str),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .pl(px(6.))
                                                    .text_xs()
                                                    .font_family("monospace")
                                                    .text_color(text_col)
                                                    .whitespace_nowrap()
                                                    .child(Self::render_styled_text(window, styled, text_col)),
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
                    move |range: Range<usize>, window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let row = &sbs_rows[i];
                                match row {
                                    SideBySideRow::HunkHeader {
                                        header,
                                        context_name,
                                        hunk_index,
                                    } => {
                                        let line_range: SharedString =
                                            Self::extract_line_range(header).into();
                                        let ctx_name: SharedString = context_name.clone().into();
                                        let has_context = !context_name.is_empty();
                                        let stage_label = if is_staged {
                                            "Unstage Hunk"
                                        } else {
                                            "Stage Hunk"
                                        };
                                        let idx = *hunk_index;
                                        let view_clone = view.clone();
                                        let view_sbs_hunk = view.clone();
                                        let is_sbs_hunk_selected = selected_lines
                                            .as_ref()
                                            .is_some_and(|r| r.contains(&i));
                                        let sbs_hunk_bg = if is_sbs_hunk_selected {
                                            selection_bg
                                        } else {
                                            element_bg
                                        };

                                        let mut hunk_row = div()
                                            .id(ElementId::NamedInteger(
                                                "sbs-hunk-header".into(),
                                                i as u64,
                                            ))
                                            .h_flex()
                                            .h(px(hunk_header_height))
                                            .w_full()
                                            .px(px(8.))
                                            .py(px(4.))
                                            .items_center()
                                            .gap(px(8.))
                                            .bg(sbs_hunk_bg)
                                            .border_t_1()
                                            .border_b_1()
                                            .border_color(border_variant)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |event: &MouseDownEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    let shift = event.modifiers.shift;
                                                    view_sbs_hunk
                                                        .update(cx, |this, cx| {
                                                            this.handle_line_click(i, shift, cx);
                                                        })
                                                        .ok();
                                                },
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_family("monospace")
                                                    .text_color(text_muted)
                                                    .child(line_range),
                                            );

                                        if has_context {
                                            hunk_row = hunk_row.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(text_placeholder_color)
                                                    .italic()
                                                    .child(ctx_name),
                                            );
                                        }

                                        hunk_row
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
                                        left_styled,
                                        left_kind,
                                        right_num,
                                        right_styled,
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
                                                    (empty_fill_bg, gutter_bg, text_placeholder_color)
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
                                                    (empty_fill_bg, gutter_bg, text_placeholder_color)
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
                                        let is_highlighted = highlighted_row == Some(i);
                                        let is_sbs_selected = selected_lines
                                            .as_ref()
                                            .is_some_and(|r| r.contains(&i));
                                        let effective_left_bg = if is_sbs_selected {
                                            selection_bg
                                        } else if is_highlighted {
                                            highlight_bg
                                        } else {
                                            left_bg
                                        };
                                        let effective_right_bg = if is_sbs_selected {
                                            selection_bg
                                        } else if is_highlighted {
                                            highlight_bg
                                        } else {
                                            right_bg
                                        };

                                        let view_sbs_line = view.clone();
                                        div()
                                            .id(ElementId::NamedInteger(
                                                "sbs-line".into(),
                                                i as u64,
                                            ))
                                            .h_flex()
                                            .h(px(row_height))
                                            .w_full()
                                            .hover(move |s| s.bg(row_hover_bg))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |event: &MouseDownEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    let shift = event.modifiers.shift;
                                                    view_sbs_line
                                                        .update(cx, |this, cx| {
                                                            this.handle_line_click(i, shift, cx);
                                                        })
                                                        .ok();
                                                },
                                            )
                                            .child(
                                                div()
                                                    .h_flex()
                                                    .flex_1()
                                                    .h_full()
                                                    .overflow_x_hidden()
                                                    .bg(effective_left_bg)
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
                                                            .font_family("monospace")
                                                            .text_color(text_muted)
                                                            .pr(px(4.))
                                                            .child(left_num_str),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .pl(px(6.))
                                                            .text_xs()
                                                            .font_family("monospace")
                                                            .text_color(left_text_col)
                                                            .whitespace_nowrap()
                                                            .text_ellipsis()
                                                            .child(Self::render_styled_text(window, left_styled, left_text_col)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .w(px(2.))
                                                    .flex_shrink_0()
                                                    .h_full()
                                                    .bg(border_variant),
                                            )
                                            .child(
                                                div()
                                                    .h_flex()
                                                    .flex_1()
                                                    .h_full()
                                                    .overflow_x_hidden()
                                                    .bg(effective_right_bg)
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
                                                            .font_family("monospace")
                                                            .text_color(text_muted)
                                                            .pr(px(4.))
                                                            .child(right_num_str),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .h_full()
                                                            .flex()
                                                            .items_center()
                                                            .pl(px(6.))
                                                            .text_xs()
                                                            .font_family("monospace")
                                                            .text_color(right_text_col)
                                                            .whitespace_nowrap()
                                                            .text_ellipsis()
                                                            .child(Self::render_styled_text(window, right_styled, right_text_col)),
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

        let mut container = div()
            .id("diff-viewer")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .v_flex()
            .size_full()
            .bg(editor_bg);

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
                    .h(px(26.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(border_variant)
                    .child(
                        Icon::new(file_icon)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(path_str)
                            .size(LabelSize::Small)
                            .weight(FontWeight::MEDIUM)
                            .truncate(),
                    )
                    .child(
                        Badge::new(if is_staged { "Staged" } else { "Unstaged" }).color(
                            if is_staged {
                                Color::Added
                            } else {
                                Color::Modified
                            },
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .h_flex()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(vc_added)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(format!("+{}", additions))),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(vc_deleted)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(format!("-{}", deletions))),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_placeholder_color)
                            .child("\u{00B7}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_for_path_uses_expected_fallback_icons() {
        assert_eq!(DiffViewer::icon_for_path("src/main.rs"), IconName::File);
        assert_eq!(
            DiffViewer::icon_for_path("config/settings.jsonc"),
            IconName::Settings
        );
        assert_eq!(DiffViewer::icon_for_path("Cargo.lock"), IconName::Pin);
    }

    #[test]
    fn syntax_for_path_maps_common_extension_fallbacks() {
        let tsx = DiffViewer::syntax_for_path("src/app.tsx")
            .expect("tsx fallback should resolve")
            .name
            .as_str();
        let jsonc = DiffViewer::syntax_for_path("config/biome.jsonc")
            .expect("jsonc fallback should resolve")
            .name
            .as_str();
        let env = DiffViewer::syntax_for_path(".env")
            .expect("dot env fallback should resolve")
            .name
            .as_str();
        let sql = DiffViewer::syntax_for_path("queries/migrate.sql")
            .expect("sql fallback should resolve")
            .name
            .as_str();

        assert_eq!(tsx, "JavaScript");
        assert_eq!(jsonc, "JSON");
        assert_eq!(env, "Bourne Again Shell (bash)");
        assert_eq!(sql, "SQL");
    }

    #[test]
    fn icon_for_path_covers_common_extensions() {
        // Code files
        assert_eq!(DiffViewer::icon_for_path("main.rs"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("server.go"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("script.py"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("app.js"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("types.ts"), IconName::File);
        // Config files
        assert_eq!(DiffViewer::icon_for_path("Cargo.toml"), IconName::Settings);
        assert_eq!(DiffViewer::icon_for_path("config.yaml"), IconName::Settings);
        assert_eq!(DiffViewer::icon_for_path("deploy.yml"), IconName::Settings);
        assert_eq!(
            DiffViewer::icon_for_path("settings.json"),
            IconName::Settings
        );
        // Lock files
        assert_eq!(DiffViewer::icon_for_path("yarn.lock"), IconName::Pin);
        // package-lock.json has extension .json → Settings, not Pin
        assert_eq!(
            DiffViewer::icon_for_path("package-lock.json"),
            IconName::Settings
        );
        // Other
        assert_eq!(DiffViewer::icon_for_path("styles.css"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("index.html"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path("README.md"), IconName::File);
        // No extension → File
        assert_eq!(DiffViewer::icon_for_path("Makefile"), IconName::File);
        assert_eq!(DiffViewer::icon_for_path(".gitignore"), IconName::File);
    }

    #[test]
    fn extract_context_name_parses_hunk_header() {
        // Normal hunk with function name
        let header = "@@ -1,5 +1,7 @@ fn main()";
        assert_eq!(DiffViewer::extract_context_name(header), "fn main()");

        // Multi-line Rust function signature
        let header = "@@ -10,5 +10,7 @@ pub fn process_items<T>(items: Vec<T>) where T: Clone";
        assert_eq!(
            DiffViewer::extract_context_name(header),
            "pub fn process_items<T>(items: Vec<T>) where T: Clone"
        );

        // Empty context after @@ (no function name)
        let header = "@@ -0,0 +1,4 @@";
        assert_eq!(DiffViewer::extract_context_name(header), "");

        // No second @@
        let header = "@@ -1,3 +1,4";
        assert_eq!(DiffViewer::extract_context_name(header), "");

        // Not a hunk header at all
        let header = "just some text";
        assert_eq!(DiffViewer::extract_context_name(header), "");

        // Class method
        let header = "@@ -5,10 +5,12 @@ impl<T> MyStruct<T> {";
        assert_eq!(
            DiffViewer::extract_context_name(header),
            "impl<T> MyStruct<T> {"
        );
    }

    #[test]
    fn extract_line_range_parses_hunk_header() {
        // Normal multi-line hunk
        let header = "@@ -1,5 +1,7 @@ fn main()";
        assert_eq!(DiffViewer::extract_line_range(header), "@@ -1,5 +1,7 @@");

        // Single-line hunk (deletion only)
        let header = "@@ -3,1 +3,0 @@";
        assert_eq!(DiffViewer::extract_line_range(header), "@@ -3,1 +3,0 @@");

        // Single-line hunk (addition only)
        let header = "@@ -0,0 +1,1 @@";
        assert_eq!(DiffViewer::extract_line_range(header), "@@ -0,0 +1,1 @@");

        // Non-hunk string returns as-is
        let header = "no markers here";
        assert_eq!(DiffViewer::extract_line_range(header), "no markers here");
    }

    #[test]
    fn syntax_for_path_covers_rust_and_config_extensions() {
        let rs = DiffViewer::syntax_for_path("src/lib.rs")
            .expect("rust file should resolve")
            .name
            .as_str();
        assert_eq!(rs, "Rust");

        let toml = DiffViewer::syntax_for_path("workspace.toml")
            .expect("toml file should resolve")
            .name
            .as_str();
        // Fallback maps .toml → YAML (closest in structure for basic highlighting)
        assert_eq!(toml, "YAML");

        let yaml = DiffViewer::syntax_for_path("ci.yml")
            .expect("yaml file should resolve")
            .name
            .as_str();
        assert_eq!(yaml, "YAML");

        let css = DiffViewer::syntax_for_path("style.css")
            .expect("css file should resolve")
            .name
            .as_str();
        assert_eq!(css, "CSS");
    }

    // --- Word-level diff tests ---

    #[test]
    fn split_word_ranges_simple() {
        let words = DiffViewer::split_word_ranges("hello world");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], 0..5); // "hello"
        assert_eq!(words[1], 6..11); // "world"
    }

    #[test]
    fn split_word_ranges_single_word() {
        let words = DiffViewer::split_word_ranges("hello");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0..5);
    }

    #[test]
    fn split_word_ranges_empty() {
        assert!(DiffViewer::split_word_ranges("").is_empty());
    }

    #[test]
    fn split_word_ranges_only_spaces() {
        // All-whitespace input yields no word ranges
        assert!(DiffViewer::split_word_ranges("   ").is_empty());
    }

    #[test]
    fn split_word_ranges_leading_trailing_spaces() {
        // Leading/trailing spaces are trimmed; only "foo" is a word
        let words = DiffViewer::split_word_ranges("  foo  ");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 2..5);
    }

    #[test]
    fn split_word_ranges_multiple_spaces_between_words() {
        // Multiple spaces between words: each gap is a separator
        let words = DiffViewer::split_word_ranges("foo  bar");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], 0..3); // "foo"
        assert_eq!(words[1], 5..8); // "bar" (indices skip the two spaces)
    }

    #[test]
    fn split_word_ranges_mixed_whitespace() {
        let words = DiffViewer::split_word_ranges("foo\tbar\nbaz");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0], 0..3); // "foo"
        assert_eq!(words[1], 4..7); // "bar" (\t at index 3)
        assert_eq!(words[2], 8..11); // "baz" (\n at index 7)
    }

    #[test]
    fn split_word_ranges_with_unicode() {
        // Non-ASCII chars treated as word characters
        let words = DiffViewer::split_word_ranges("日本語 English");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], 0..9); // "日本語" = 3 chars × 3 bytes = 9 bytes
        assert_eq!(words[1], 10..17); // " English" — space at 9, then 7 chars
    }

    // #[test] // DISABLED — compute_word_diff is disabled while investigating
    // fn compute_word_diff_unchanged() { ... }
    // WORD-LEVEL DIFF TESTS DISABLED — see compute_word_diff stub above
}
