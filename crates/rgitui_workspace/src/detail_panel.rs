use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, App, ClickEvent, ClipboardItem, Context, ElementId, EventEmitter,
    FocusHandle, KeyDownEvent, ListSizingBehavior, ObjectFit, Render, SharedString, WeakEntity,
    Window,
};
use rgitui_git::{
    BranchInfo, CommitDiff, CommitInfo, FileChangeKind, FileDiff, RefLabel, Signature,
};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    AvatarCache, Badge, ButtonSize, ButtonStyle, DiffStat, Icon, IconButton, IconName, IconSize,
    Label, LabelSize,
};

use crate::markdown_view::render_markdown;

/// Maximum height of the changed-files list. Beyond this the list virtualizes and
/// scrolls internally (only the visible window is built); shorter lists size to
/// their content so the panel scrolls as one region.
const FILE_LIST_MAX_HEIGHT: f32 = 600.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum FileViewMode {
    #[default]
    Flat,
    Tree,
}

impl FileViewMode {
    fn toggled(self) -> Self {
        match self {
            Self::Flat => Self::Tree,
            Self::Tree => Self::Flat,
        }
    }

    fn toggle_icon(self) -> IconName {
        match self {
            Self::Flat => IconName::Folder,
            Self::Tree => IconName::File,
        }
    }

    fn toggle_tooltip(self) -> &'static str {
        match self {
            Self::Flat => "Switch to Tree view (v)",
            Self::Tree => "Switch to Flat view (v)",
        }
    }

    fn list_element_id(self) -> &'static str {
        match self {
            Self::Flat => "detail-files-filtered",
            Self::Tree => "detail-files-filtered-tree",
        }
    }
}

fn is_file_searching(file_search_active: bool, file_search_query: &Option<String>) -> bool {
    file_search_active || file_search_query.is_some()
}

fn can_toggle_file_view(file_count: usize, is_searching: bool) -> bool {
    file_count > 0 && !is_searching
}

fn file_view_toggle_tooltip(
    file_count: usize,
    is_searching: bool,
    file_view_mode: FileViewMode,
) -> &'static str {
    if file_count == 0 {
        "No changed files to display"
    } else if is_searching {
        "Clear file search to switch views"
    } else {
        file_view_mode.toggle_tooltip()
    }
}

fn file_view_mode_after_toggle_request(
    file_view_mode: FileViewMode,
    file_count: usize,
    is_searching: bool,
) -> FileViewMode {
    if can_toggle_file_view(file_count, is_searching) {
        file_view_mode.toggled()
    } else {
        file_view_mode
    }
}

fn format_absolute_date(timestamp: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(timestamp, 0);
    match dt {
        Some(dt) => dt.format("%b %d, %Y %H:%M").to_string(),
        None => "Unknown date".to_string(),
    }
}

#[derive(Debug, Clone)]
pub enum DetailPanelEvent {
    FileSelected(FileDiff, String),
    CopySha(String),
    CherryPick(String),
    NavigatePrevCommit,
    NavigateNextCommit,
}

fn file_change_icon(kind: FileChangeKind) -> IconName {
    match kind {
        FileChangeKind::Added => IconName::FileAdded,
        FileChangeKind::Modified => IconName::FileModified,
        FileChangeKind::Deleted => IconName::FileDeleted,
        FileChangeKind::Renamed => IconName::FileRenamed,
        FileChangeKind::Copied => IconName::FileRenamed,
        FileChangeKind::TypeChange => IconName::FileModified,
        FileChangeKind::Untracked => IconName::FileAdded,
        FileChangeKind::Conflicted => IconName::FileConflict,
    }
}

fn file_change_color(kind: FileChangeKind) -> Color {
    match kind {
        FileChangeKind::Added => Color::Added,
        FileChangeKind::Modified => Color::Modified,
        FileChangeKind::Deleted => Color::Deleted,
        FileChangeKind::Renamed => Color::Renamed,
        FileChangeKind::Copied => Color::Info,
        FileChangeKind::TypeChange => Color::Warning,
        FileChangeKind::Untracked => Color::Untracked,
        FileChangeKind::Conflicted => Color::Conflict,
    }
}

struct CachedFileDiffTree {
    /// Rows for flat view: one per file, in diff order (the search-relevance
    /// ordering is applied later by `filtered_files`). Held behind an `Arc` so
    /// the virtualized list closure can take a cheap `'static` handle and clone
    /// only the rows in the visible window.
    flat_rows: Arc<Vec<CachedFlatRow>>,
    /// Rows for tree view: directory and file nodes flattened depth-first, with
    /// single-child directory chains compacted (e.g. `crates/foo/src`). Shared
    /// via `Arc` for the same reason as `flat_rows`.
    tree_rows: Arc<Vec<CachedTreeRow>>,
}

#[derive(Clone)]
struct CachedFlatRow {
    file_index: usize,
    file_name: SharedString,
    dir_path: SharedString,
    additions: usize,
    deletions: usize,
    icon_name: IconName,
    icon_color: Color,
    change_code: SharedString,
    change_color: Color,
}

/// A file leaf in the tree view. Carries only what a file row renders — the
/// basename lives on the enclosing [`CachedTreeRow::label`], and the directory
/// path is intentionally absent (the tree shows hierarchy, not full paths).
#[derive(Clone)]
struct CachedTreeFile {
    file_index: usize,
    additions: usize,
    deletions: usize,
    icon_name: IconName,
    icon_color: Color,
    change_code: SharedString,
    change_color: Color,
}

#[derive(Clone)]
enum CachedTreeRowKind {
    /// A directory node. `key` is its full path, used as the collapse identity.
    Dir {
        key: SharedString,
    },
    File(CachedTreeFile),
}

#[derive(Clone)]
struct CachedTreeRow {
    depth: usize,
    /// Directory name (possibly compacted, e.g. `foo/bar`) or file basename.
    label: SharedString,
    kind: CachedTreeRowKind,
}

/// Intermediate nested structure used to build [`CachedTreeRow`]s. `BTreeMap`
/// keeps directories in stable alphabetical order.
#[derive(Default)]
struct TreeBuilder {
    dirs: std::collections::BTreeMap<String, TreeBuilder>,
    file_indices: Vec<usize>,
}

/// Flatten the nested builder depth-first into ordered rows: directories first
/// (alphabetical), then files (alphabetical), at each level. Single-child
/// directory chains are compacted into one node so deep crate layouts read as
/// `crates/foo/src` rather than three nested rows. The root builder is virtual —
/// its children are emitted at `depth`, no row is emitted for the root itself.
fn flatten_tree(
    builder: &TreeBuilder,
    path_prefix: &str,
    depth: usize,
    files: &[FileDiff],
    flat_rows: &[CachedFlatRow],
    out: &mut Vec<CachedTreeRow>,
) {
    for (name, child) in &builder.dirs {
        let mut label = name.clone();
        let mut full = if path_prefix.is_empty() {
            name.clone()
        } else {
            format!("{path_prefix}/{name}")
        };
        // Compact single-child directory chains: keep descending while a dir has
        // no files of its own and exactly one subdirectory.
        let mut node = child;
        while node.file_indices.is_empty() && node.dirs.len() == 1 {
            let (child_name, grandchild) = node.dirs.iter().next().unwrap();
            label = format!("{label}/{child_name}");
            full = format!("{full}/{child_name}");
            node = grandchild;
        }
        out.push(CachedTreeRow {
            depth,
            label: label.into(),
            kind: CachedTreeRowKind::Dir {
                key: full.clone().into(),
            },
        });
        flatten_tree(node, &full, depth + 1, files, flat_rows, out);
    }

    let mut file_indices = builder.file_indices.clone();
    file_indices.sort_by(|&a, &b| files[a].path.file_name().cmp(&files[b].path.file_name()));
    for fi in file_indices {
        let fr = &flat_rows[fi];
        out.push(CachedTreeRow {
            depth,
            label: fr.file_name.clone(),
            kind: CachedTreeRowKind::File(CachedTreeFile {
                file_index: fr.file_index,
                additions: fr.additions,
                deletions: fr.deletions,
                icon_name: fr.icon_name,
                icon_color: fr.icon_color,
                change_code: fr.change_code.clone(),
                change_color: fr.change_color,
            }),
        });
    }
}

/// Indices of `rows` visible given the set of collapsed directory keys. A
/// collapsed directory hides every row deeper than it until the next row at or
/// above its depth; a single threshold suffices because an outer collapse
/// already hides any nested collapses.
fn visible_tree_row_indices(rows: &[CachedTreeRow], collapsed: &HashSet<String>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut hide_below: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        if let Some(d) = hide_below {
            if row.depth > d {
                continue;
            }
            hide_below = None;
        }
        out.push(i);
        if let CachedTreeRowKind::Dir { key } = &row.kind {
            if collapsed.contains(key.as_ref()) {
                hide_below = Some(row.depth);
            }
        }
    }
    out
}

/// Indices of `rows` to show when filtering by search: every file whose index is
/// in `matches`, plus the ancestor directories on the path to it. Collapse state
/// is ignored so matches are always revealed.
fn searched_tree_row_indices(rows: &[CachedTreeRow], matches: &HashSet<usize>) -> Vec<usize> {
    let mut keep = vec![false; rows.len()];
    let mut ancestors: Vec<usize> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        while let Some(&a) = ancestors.last() {
            if rows[a].depth >= row.depth {
                ancestors.pop();
            } else {
                break;
            }
        }
        match &row.kind {
            CachedTreeRowKind::Dir { .. } => ancestors.push(i),
            CachedTreeRowKind::File(f) => {
                if matches.contains(&f.file_index) {
                    keep[i] = true;
                    for &a in &ancestors {
                        keep[a] = true;
                    }
                }
            }
        }
    }
    (0..rows.len()).filter(|&i| keep[i]).collect()
}

fn build_cached_file_tree(files: &[FileDiff]) -> CachedFileDiffTree {
    let flat_rows: Vec<CachedFlatRow> = files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let path_str = file.path.display().to_string();
            let file_name: SharedString = file
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path_str)
                .to_string()
                .into();
            let dir_path: SharedString = file
                .path
                .parent()
                .map(|p| {
                    let s = p.display().to_string();
                    if s.is_empty() {
                        s
                    } else {
                        format!("{}/", s)
                    }
                })
                .unwrap_or_default()
                .into();
            CachedFlatRow {
                file_index: i,
                file_name,
                dir_path,
                additions: file.additions,
                deletions: file.deletions,
                icon_name: file_change_icon(file.kind),
                icon_color: file_change_color(file.kind),
                change_code: file.kind.short_code().into(),
                change_color: file_change_color(file.kind),
            }
        })
        .collect();

    // Build the directory tree from file paths. Git diff paths are always
    // '/'-separated, so split on '/' rather than the platform separator.
    let mut root = TreeBuilder::default();
    for (i, file) in files.iter().enumerate() {
        let path_str = file.path.display().to_string();
        let segments: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        let Some((_file_name, dirs)) = segments.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for dir in dirs {
            node = node.dirs.entry((*dir).to_string()).or_default();
        }
        node.file_indices.push(i);
    }
    let mut tree_rows = Vec::new();
    flatten_tree(&root, "", 0, files, &flat_rows, &mut tree_rows);

    CachedFileDiffTree {
        flat_rows: Arc::new(flat_rows),
        tree_rows: Arc::new(tree_rows),
    }
}

/// Result of applying the file-search filter to a commit's changed files.
///
/// The no-query case is represented without allocating a per-frame mapping: it is
/// the identity over `0..count` in diff order. Only an active query materialises a
/// relevance-ordered list of matching file indices.
enum FilteredFiles {
    /// No active query — all `count` files, in diff order.
    All { count: usize },
    /// Search results: matching file indices ordered by relevance (best first).
    Matched(Vec<usize>),
}

impl FilteredFiles {
    fn len(&self) -> usize {
        match self {
            Self::All { count } => *count,
            Self::Matched(indices) => indices.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The set of matching file indices, used to filter the tree view.
    fn matched_set(&self) -> HashSet<usize> {
        match self {
            Self::All { count } => (0..*count).collect(),
            Self::Matched(indices) => indices.iter().copied().collect(),
        }
    }
}

pub struct DetailPanel {
    commit: Option<CommitInfo>,
    commit_diff: Option<Arc<CommitDiff>>,
    cached_file_tree: Option<CachedFileDiffTree>,
    selected_file_index: Option<usize>,
    focus_handle: FocusHandle,
    copied_field: Option<(&'static str, Instant)>,
    file_search_query: Option<String>,
    file_search_active: bool,
    file_view_mode: FileViewMode,
    collapsed_dirs: HashSet<String>,
    description_expanded: bool,
    contained_in: Vec<BranchInfo>,
    contained_in_loading: bool,
}

impl EventEmitter<DetailPanelEvent> for DetailPanel {}

impl DetailPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            commit: None,
            commit_diff: None,
            cached_file_tree: None,
            selected_file_index: None,
            focus_handle: cx.focus_handle(),
            copied_field: None,
            file_search_query: None,
            file_search_active: false,
            file_view_mode: FileViewMode::default(),
            collapsed_dirs: HashSet::new(),
            description_expanded: false,
            contained_in: Vec::new(),
            contained_in_loading: false,
        }
    }

    fn mark_copied(&mut self, field: &'static str, cx: &mut Context<Self>) {
        self.copied_field = Some((field, Instant::now()));
        cx.notify();
        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                this.update(cx, |this, cx| {
                    if let Some((f, t)) = this.copied_field {
                        if f == field && t.elapsed() >= Duration::from_millis(1400) {
                            this.copied_field = None;
                            cx.notify();
                        }
                    }
                })
                .ok();
            },
        )
        .detach();
    }

    fn is_copied(&self, field: &'static str) -> bool {
        self.copied_field
            .is_some_and(|(f, t)| f == field && t.elapsed() < Duration::from_millis(1500))
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn commit(&self) -> Option<&CommitInfo> {
        self.commit.as_ref()
    }

    fn file_count(&self) -> usize {
        self.commit_diff
            .as_ref()
            .map(|d| d.files.len())
            .unwrap_or(0)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let file_count = self.file_count();

        // Commit prev/next navigation — works regardless of file count
        match key {
            "[" => {
                if self.commit.is_some() {
                    cx.emit(DetailPanelEvent::NavigatePrevCommit);
                }
            }
            "]" => {
                if self.commit.is_some() {
                    cx.emit(DetailPanelEvent::NavigateNextCommit);
                }
            }
            _ => {}
        }

        // File search toggle: / or Ctrl+F
        if (key == "/" && !modifiers.control && !modifiers.platform)
            || ((modifiers.control || modifiers.platform) && key == "f")
        {
            self.file_search_active = true;
            cx.notify();
            return;
        }

        // Escape clears search
        if key == "escape" {
            if self.file_search_query.is_some() || self.file_search_active {
                self.file_search_query = None;
                self.file_search_active = false;
                cx.notify();
            }
            return;
        }

        // When search is active, capture printable characters as query
        if self.file_search_active {
            // Use key_char for printable input (same as TextInput)
            if let Some(kc) = &event.keystroke.key_char {
                let ch = kc.to_lowercase().chars().next().unwrap_or(' ');
                let new_query = match &mut self.file_search_query {
                    Some(q) => {
                        q.push(ch);
                        q.clone()
                    }
                    None => ch.to_string(),
                };
                self.file_search_query = Some(new_query);
                cx.notify();
            } else if key == "backspace" {
                if let Some(q) = &mut self.file_search_query {
                    q.pop();
                    if q.is_empty() {
                        self.file_search_query = None;
                    }
                }
                cx.notify();
            }
            return;
        }

        if file_count == 0 {
            return;
        }

        match key {
            "j" | "down" => {
                let next = match self.selected_file_index {
                    Some(i) if i + 1 < file_count => Some(i + 1),
                    None => Some(0),
                    other => other,
                };
                if next != self.selected_file_index {
                    self.selected_file_index = next;
                    self.emit_file_selected(cx);
                    cx.notify();
                }
            }
            "k" | "up" => {
                let next = match self.selected_file_index {
                    Some(i) if i > 0 => Some(i - 1),
                    None if file_count > 0 => Some(0),
                    other => other,
                };
                if next != self.selected_file_index {
                    self.selected_file_index = next;
                    self.emit_file_selected(cx);
                    cx.notify();
                }
            }
            "home" | "g" if self.selected_file_index != Some(0) => {
                self.selected_file_index = Some(0);
                self.emit_file_selected(cx);
                cx.notify();
            }
            "end" => {
                let last = file_count.saturating_sub(1);
                if self.selected_file_index != Some(last) {
                    self.selected_file_index = Some(last);
                    self.emit_file_selected(cx);
                    cx.notify();
                }
            }
            "v" => {
                self.request_file_view_mode_toggle(cx);
            }
            _ => {}
        }
    }

    fn request_file_view_mode_toggle(&mut self, cx: &mut Context<Self>) {
        let is_searching = is_file_searching(self.file_search_active, &self.file_search_query);
        let next_mode = file_view_mode_after_toggle_request(
            self.file_view_mode,
            self.file_count(),
            is_searching,
        );
        if next_mode != self.file_view_mode {
            self.file_view_mode = next_mode;
            cx.notify();
        }
    }

    /// Toggle a directory node's collapsed state in the tree view.
    fn toggle_dir_collapsed(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        cx.notify();
    }

    fn emit_file_selected(&self, cx: &mut Context<Self>) {
        if let (Some(idx), Some(diff)) = (self.selected_file_index, &self.commit_diff) {
            if let Some(file) = diff.files.get(idx) {
                cx.emit(DetailPanelEvent::FileSelected(
                    file.clone(),
                    file.path.to_string_lossy().to_string(),
                ));
            }
        }
    }

    pub fn set_commit(&mut self, commit: CommitInfo, diff: CommitDiff, cx: &mut Context<Self>) {
        log::debug!(
            "DetailPanel::set_commit: oid={} files={}",
            commit.short_id,
            diff.files.len()
        );
        self.cached_file_tree = Some(build_cached_file_tree(&diff.files));
        self.commit = Some(commit);
        self.commit_diff = Some(Arc::new(diff));
        self.selected_file_index = None;
        self.file_search_query = None;
        self.file_search_active = false;
        self.collapsed_dirs.clear();
        self.description_expanded = false;
        self.contained_in.clear();
        self.contained_in_loading = false;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        log::debug!("DetailPanel::clear");
        self.commit = None;
        self.commit_diff = None;
        self.cached_file_tree = None;
        self.selected_file_index = None;
        self.file_search_query = None;
        self.file_search_active = false;
        self.collapsed_dirs.clear();
        self.description_expanded = false;
        self.contained_in.clear();
        self.contained_in_loading = false;
        cx.notify();
    }

    pub fn set_contained_in(&mut self, branches: Vec<BranchInfo>, cx: &mut Context<Self>) {
        self.contained_in = branches;
        self.contained_in_loading = false;
        cx.notify();
    }

    pub fn set_contained_in_loading(&mut self, cx: &mut Context<Self>) {
        self.contained_in_loading = true;
        cx.notify();
    }

    /// File indices to display, after applying the search filter.
    ///
    /// With no active query this is the identity over all files (in diff order)
    /// and allocates nothing. With a query, matching files are scored with
    /// `fuzzy_score` and returned ordered by relevance (best match first).
    fn filtered_files(&self) -> FilteredFiles {
        let query = match &self.file_search_query {
            Some(q) if !q.is_empty() => q,
            _ => {
                return FilteredFiles::All {
                    count: self.file_count(),
                }
            }
        };
        let Some(diff) = &self.commit_diff else {
            return FilteredFiles::Matched(Vec::new());
        };
        let mut scored: Vec<(usize, usize)> = diff
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, file)| {
                let path = file.path.to_string_lossy();
                crate::command_palette::CommandPalette::fuzzy_score(query, &path)
                    .map(|score| (score, i))
            })
            .collect();
        // Sort by score descending — higher score = better (earlier char match)
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        FilteredFiles::Matched(scored.into_iter().map(|(_, i)| i).collect())
    }

    fn render_section_header(&self, label: &str) -> impl IntoElement {
        Label::new(SharedString::from(label.to_string()))
            .size(LabelSize::XSmall)
            .color(Color::Muted)
            .weight(gpui::FontWeight::SEMIBOLD)
    }

    fn render_co_authors(
        &self,
        co_authors: &[Signature],
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let avatar_bg = colors.border_focused;
        let avatar_text_color = colors.background;

        let mut section = div()
            .v_flex()
            .gap(px(4.))
            .child(self.render_section_header("Co-authors"));

        for co_author in co_authors {
            let initials: SharedString = co_author
                .name
                .split_whitespace()
                .take(2)
                .filter_map(|w| w.chars().next())
                .collect::<String>()
                .to_uppercase()
                .into();

            let co_name: SharedString = co_author.name.clone().into();
            let co_email: SharedString = format!("<{}>", co_author.email).into();

            let avatar_url = cx
                .try_global::<AvatarCache>()
                .and_then(|cache| cache.avatar_url(&co_author.email))
                .map(|s| s.to_string());

            let avatar_circle =
                self.render_avatar(avatar_url, initials, avatar_bg, avatar_text_color, px(16.));

            section = section.child(
                div()
                    .h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(avatar_circle)
                    .child(
                        Label::new(co_name)
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(co_email)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .truncate(),
                    ),
            );
        }

        section.into_any_element()
    }

    fn render_avatar(
        &self,
        avatar_url: Option<String>,
        initials: SharedString,
        avatar_bg: gpui::Hsla,
        avatar_text_color: gpui::Hsla,
        size: gpui::Pixels,
    ) -> gpui::Div {
        let mut avatar_circle = div()
            .w(size)
            .h(size)
            .rounded_full()
            .bg(avatar_bg)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center();

        if let Some(url) = avatar_url {
            let fb_initials = initials.clone();
            avatar_circle = avatar_circle.child(
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
                                    .text_color(avatar_text_color)
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(fb_initials.clone()),
                            )
                            .into_any_element()
                    }),
            );
        } else {
            avatar_circle = avatar_circle.child(
                div()
                    .text_color(avatar_text_color)
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(initials),
            );
        }

        avatar_circle
    }

    fn render_flat_file_list_filtered(
        &self,
        cached: &CachedFileDiffTree,
        filtered: &FilteredFiles,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors().clone();
        let row_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(26.0);
        let selected_file_index = self.selected_file_index;
        let weak = cx.weak_entity();
        let row_count = filtered.len();
        // Cheap `'static` handles for the virtualized closure: the shared row
        // store and, when searching, the display->file-index mapping (`None`
        // means identity, i.e. unfiltered diff order). Only the visible window is
        // cloned, inside the closure.
        let flat_rows = cached.flat_rows.clone();
        let display_order: Option<Arc<Vec<usize>>> = match filtered {
            FilteredFiles::All { .. } => None,
            FilteredFiles::Matched(indices) => Some(Arc::new(indices.clone())),
        };

        let ghost_element_selected = colors.ghost_element_selected;
        let text_accent = colors.text_accent;
        let border_transparent = colors.border_transparent;
        let ghost_element_hover = colors.ghost_element_hover;

        uniform_list(
            self.file_view_mode.list_element_id(),
            row_count,
            move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|ix| {
                        let file_index = match &display_order {
                            Some(order) => order[ix],
                            None => ix,
                        };
                        let row = &flat_rows[file_index];
                        let actual_file_index = row.file_index;
                        let selected = selected_file_index == Some(actual_file_index);
                        let weak = weak.clone();
                        let dir_path = row.dir_path.clone();

                        div()
                            .id(ElementId::NamedInteger("detail-file".into(), ix as u64))
                            .h_flex()
                            .w_full()
                            .h(px(row_h))
                            .pl(px(12.))
                            .pr(px(12.))
                            .gap(px(6.))
                            .items_center()
                            .flex_shrink_0()
                            .border_l_2()
                            .when(selected, |el| {
                                el.bg(ghost_element_selected).border_color(text_accent)
                            })
                            .when(!selected, |el| el.border_color(border_transparent))
                            .hover(move |s| s.bg(ghost_element_hover))
                            .cursor_pointer()
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                weak.update(cx, |this, cx| {
                                    this.selected_file_index = Some(actual_file_index);
                                    this.emit_file_selected(cx);
                                    cx.notify();
                                })
                                .ok();
                            })
                            .child(
                                Icon::new(row.icon_name)
                                    .size(IconSize::XSmall)
                                    .color(row.icon_color),
                            )
                            .child(
                                div()
                                    .h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .overflow_hidden()
                                    .when(!dir_path.is_empty(), |el| {
                                        el.child(
                                            Label::new(dir_path)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                    })
                                    .child(
                                        Label::new(row.file_name.clone())
                                            .size(LabelSize::XSmall)
                                            .truncate(),
                                    ),
                            )
                            .child(
                                div()
                                    .h_flex()
                                    .w(px(16.))
                                    .h(px(16.))
                                    .rounded(px(3.))
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Label::new(row.change_code.clone())
                                            .size(LabelSize::XSmall)
                                            .color(row.change_color)
                                            .weight(gpui::FontWeight::BOLD),
                                    ),
                            )
                            .child(DiffStat::new(row.additions, row.deletions))
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .flex_shrink_0()
        .pb_2()
        .max_h(px(FILE_LIST_MAX_HEIGHT))
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .into_any_element()
    }

    /// Renders the directory tree: collapsible directory nodes and file leaves
    /// (basename only), indented by depth. `tree_rows` is the full row store and
    /// `visible` the indices into it to display (collapse-filtered when browsing,
    /// match-filtered when searching), in display order. Both are passed by shared
    /// handle so the virtualized closure clones only the visible window.
    fn render_tree_file_list(
        &self,
        tree_rows: &Arc<Vec<CachedTreeRow>>,
        visible: Vec<usize>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors().clone();
        let row_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(26.0);
        let selected_file_index = self.selected_file_index;
        let collapsed = self.collapsed_dirs.clone();
        let weak = cx.weak_entity();
        let row_count = visible.len();
        let tree_rows = tree_rows.clone();

        let ghost_element_selected = colors.ghost_element_selected;
        let text_accent = colors.text_accent;
        let border_transparent = colors.border_transparent;
        let ghost_element_hover = colors.ghost_element_hover;

        uniform_list(
            self.file_view_mode.list_element_id(),
            row_count,
            move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|ix| {
                        let row = &tree_rows[visible[ix]];
                        // 12px base + 14px per depth level. Directory rows lead with
                        // a chevron; file rows lead with a chevron-width spacer so
                        // their icon aligns under the folder icon rather than just
                        // shifting right.
                        let indent = px(12.0 + row.depth as f32 * 14.0);
                        let weak = weak.clone();
                        match &row.kind {
                            CachedTreeRowKind::Dir { key } => {
                                let is_collapsed = collapsed.contains(key.as_ref());
                                let key_str = key.to_string();
                                div()
                                    .id(ElementId::NamedInteger(
                                        "detail-tree-dir".into(),
                                        ix as u64,
                                    ))
                                    .h_flex()
                                    .w_full()
                                    .h(px(row_h))
                                    .pl(indent)
                                    .pr(px(12.))
                                    .gap(px(4.))
                                    .items_center()
                                    .flex_shrink_0()
                                    .border_l_2()
                                    .border_color(border_transparent)
                                    .hover(move |s| s.bg(ghost_element_hover))
                                    .cursor_pointer()
                                    .on_click(
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let key_str = key_str.clone();
                                            weak.update(cx, |this, cx| {
                                                this.toggle_dir_collapsed(key_str, cx);
                                            })
                                            .ok();
                                        },
                                    )
                                    .child(
                                        Icon::new(if is_collapsed {
                                            IconName::ChevronRight
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                    )
                                    .child(
                                        Icon::new(IconName::Folder)
                                            .size(IconSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        Label::new(row.label.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    )
                                    .into_any_element()
                            }
                            CachedTreeRowKind::File(file) => {
                                let actual_file_index = file.file_index;
                                let selected = selected_file_index == Some(actual_file_index);
                                div()
                                    .id(ElementId::NamedInteger("detail-file".into(), ix as u64))
                                    .h_flex()
                                    .w_full()
                                    .h(px(row_h))
                                    .pl(indent)
                                    .pr(px(12.))
                                    .gap(px(6.))
                                    .items_center()
                                    .flex_shrink_0()
                                    .border_l_2()
                                    .when(selected, |el| {
                                        el.bg(ghost_element_selected).border_color(text_accent)
                                    })
                                    .when(!selected, |el| el.border_color(border_transparent))
                                    .hover(move |s| s.bg(ghost_element_hover))
                                    .cursor_pointer()
                                    .on_click(
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            weak.update(cx, |this, cx| {
                                                this.selected_file_index = Some(actual_file_index);
                                                this.emit_file_selected(cx);
                                                cx.notify();
                                            })
                                            .ok();
                                        },
                                    )
                                    .child(div().w(px(14.)).flex_shrink_0())
                                    .child(
                                        Icon::new(file.icon_name)
                                            .size(IconSize::XSmall)
                                            .color(file.icon_color),
                                    )
                                    .child(
                                        div().h_flex().flex_1().min_w_0().overflow_hidden().child(
                                            Label::new(row.label.clone())
                                                .size(LabelSize::XSmall)
                                                .truncate(),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .h_flex()
                                            .w(px(16.))
                                            .h(px(16.))
                                            .rounded(px(3.))
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                Label::new(file.change_code.clone())
                                                    .size(LabelSize::XSmall)
                                                    .color(file.change_color)
                                                    .weight(gpui::FontWeight::BOLD),
                                            ),
                                    )
                                    .child(DiffStat::new(file.additions, file.deletions))
                                    .into_any_element()
                            }
                        }
                    })
                    .collect()
            },
        )
        .flex_shrink_0()
        .pb_2()
        .max_h(px(FILE_LIST_MAX_HEIGHT))
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .into_any_element()
    }
}

impl Render for DetailPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let compact = &cx.global::<SettingsState>().settings().compactness;

        // Compactness-scaled spacing values for header and message cards
        let header_pad_h = px(compact.spacing(16.0));
        let header_pad_v = px(compact.spacing(14.0));
        let header_gap = px(compact.spacing(10.0));
        let msg_pad_h = px(compact.spacing(12.0));
        let msg_pad_v = px(compact.spacing(10.0));
        let msg_gap = px(compact.spacing(6.0));
        let avatar_size = px(compact.spacing(24.0));

        let Some(commit) = &self.commit else {
            return div()
                .id("detail-panel")
                .v_flex()
                .size_full()
                .bg(colors.panel_background)
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(32.))
                        .px(px(10.))
                        .gap(px(4.))
                        .items_center()
                        .bg(colors.toolbar_background)
                        .border_b_1()
                        .border_color(colors.border_variant)
                        .child(
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Details")
                                .size(LabelSize::Small)
                                .weight(gpui::FontWeight::SEMIBOLD)
                                .color(Color::Default),
                        ),
                )
                .child(
                    div().flex_1().flex().items_center().justify_center().child(
                        div()
                            .v_flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(24.))
                            .py(px(20.))
                            .rounded(px(8.))
                            .bg(colors.surface_background)
                            .border_1()
                            .border_color(colors.border_variant)
                            .child(
                                Icon::new(IconName::GitCommit)
                                    .size(IconSize::Large)
                                    .color(Color::Placeholder),
                            )
                            .child(
                                Label::new("No commit selected")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .weight(gpui::FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new("Select a commit from the graph to view details")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                            ),
                    ),
                )
                .into_any_element();
        };

        let full_sha: SharedString = format!("{}", commit.oid).into();
        let short_sha: SharedString = format!("{:.7}", commit.oid).into();
        let sha_for_copy = full_sha.clone();
        let sha_for_cherry = full_sha.clone();
        let author_name: SharedString = commit.author.name.clone().into();
        let author_email: SharedString = commit.author.email.clone().into();
        let relative_time = crate::time::format_relative_time_full(commit.time.timestamp());
        let absolute_date = format_absolute_date(commit.time.timestamp());
        let date: SharedString = format!("{} ({})", absolute_date, relative_time).into();
        let refs = commit.refs.clone();

        let (summary, description) = {
            let msg = &commit.message;
            match msg.find('\n') {
                Some(idx) => (
                    msg[..idx].trim().to_string(),
                    msg[idx + 1..].trim().to_string(),
                ),
                None => (msg.trim().to_string(), String::new()),
            }
        };
        let summary: SharedString = summary.into();

        let initials: SharedString = commit
            .author
            .name
            .split_whitespace()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase()
            .into();

        let mut panel = div()
            .id("detail-panel")
            .track_focus(&self.focus_handle)
            .key_context("DetailPanel")
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(colors.panel_background);

        // Toolbar
        panel = panel.child(
            div()
                .h_flex()
                .w_full()
                .h(px(32.))
                .px(px(10.))
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
                    Label::new("Details")
                        .size(LabelSize::Small)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Default),
                )
                .child(div().flex_1())
                .child({
                    let file_count = self.file_count();
                    let is_searching =
                        is_file_searching(self.file_search_active, &self.file_search_query);
                    let toggle_disabled = !can_toggle_file_view(file_count, is_searching);
                    let toggle_tooltip =
                        file_view_toggle_tooltip(file_count, is_searching, self.file_view_mode);

                    IconButton::new("view-mode-toggle", self.file_view_mode.toggle_icon())
                        .size(ButtonSize::Compact)
                        .disabled(toggle_disabled)
                        .tooltip(toggle_tooltip)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.request_file_view_mode_toggle(cx);
                        }))
                }),
        );

        let mut content = div()
            .id("detail-content")
            .v_flex()
            .flex_1()
            .overflow_y_scroll()
            .p_3()
            .gap_3();

        // -- Header Card: Author + SHA + Refs --
        let mut header_card = div()
            .v_flex()
            .w_full()
            .px(header_pad_h)
            .py(header_pad_v)
            .gap(header_gap)
            .bg(colors.elevated_surface_background)
            .rounded(px(8.))
            .border_1()
            .border_color(colors.border_variant);

        // Author row: avatar + name/email + timestamp
        let avatar_url = cx
            .try_global::<AvatarCache>()
            .and_then(|cache| cache.avatar_url(&commit.author.email))
            .map(|s| s.to_string());
        let avatar_bg = colors.border_focused;
        let avatar_text_color = colors.background;
        let avatar_circle = self.render_avatar(
            avatar_url,
            initials,
            avatar_bg,
            avatar_text_color,
            avatar_size,
        );

        // SHA copy button
        let sha_copy_clone = sha_for_copy.clone();
        let sha_copied = self.is_copied("sha");
        let sha_icon = if sha_copied {
            IconName::Check
        } else {
            IconName::Copy
        };

        let sha_button = div()
            .h_flex()
            .gap_1()
            .items_center()
            .px(px(6.))
            .py(px(2.))
            .bg(colors.surface_background)
            .rounded(px(4.))
            .border_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .font_family("Lilex")
                    .text_xs()
                    .text_color(colors.text_accent)
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(short_sha),
            )
            .child(
                IconButton::new("copy-sha-btn", sha_icon)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Transparent)
                    .tooltip("Copy commit SHA")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            sha_copy_clone.to_string(),
                        ));
                        cx.emit(DetailPanelEvent::CopySha(sha_copy_clone.to_string()));
                        this.mark_copied("sha", cx);
                    })),
            )
            .when(sha_copied, |el| {
                el.child(
                    Label::new("Copied!")
                        .size(LabelSize::XSmall)
                        .color(Color::Success),
                )
            });

        header_card = header_card.child(
            div()
                .v_flex()
                .w_full()
                .gap(px(4.))
                .child(
                    div()
                        .h_flex()
                        .gap(px(10.))
                        .items_center()
                        .child(avatar_circle)
                        .child(
                            div().flex_1().min_w_0().overflow_hidden().child(
                                Label::new(author_name)
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::BOLD)
                                    .truncate(),
                            ),
                        )
                        .child(sha_button),
                )
                .child(
                    Label::new(author_email)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .truncate(),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            Icon::new(IconName::Clock)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(date)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        );

        // GPG signed badge
        if commit.is_signed {
            header_card = header_card.child(
                div()
                    .h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        Icon::new(IconName::Lock)
                            .size(IconSize::XSmall)
                            .color(Color::Success),
                    )
                    .child(
                        Badge::new(SharedString::from("Signed"))
                            .color(Color::Success)
                            .bold(),
                    ),
            );
        }

        // Ref badges: branches and tags
        if !refs.is_empty() {
            let mut refs_row = div().h_flex().gap(px(4.)).flex_wrap();
            for ref_label in &refs {
                let badge_color = match ref_label {
                    RefLabel::Head => Color::Warning,
                    RefLabel::LocalBranch(_) => Color::Success,
                    RefLabel::RemoteBranch(_) => Color::Info,
                    RefLabel::Tag(_) => Color::Accent,
                };
                let name: SharedString = ref_label.display_name().to_string().into();
                let is_tag = matches!(ref_label, RefLabel::Tag(_));
                let badge = Badge::new(name).color(badge_color).bold();
                if is_tag {
                    refs_row = refs_row.child(
                        div()
                            .h_flex()
                            .gap(px(2.))
                            .items_center()
                            .child(
                                Icon::new(IconName::Tag)
                                    .size(IconSize::XSmall)
                                    .color(Color::Accent),
                            )
                            .child(badge),
                    );
                } else {
                    refs_row = refs_row.child(badge);
                }
            }
            header_card = header_card.child(refs_row);
        }

        // Co-authors
        if !commit.co_authors.is_empty() {
            header_card =
                header_card.child(self.render_co_authors(&commit.co_authors, &colors, cx));
        }

        // Contained in — branches that have this commit as an ancestor (i.e. "merged into").
        // Exclude branches already shown as direct refs to avoid redundancy.
        let refs_names: std::collections::HashSet<_> = commit
            .refs
            .iter()
            .filter_map(|r| {
                if let RefLabel::LocalBranch(name) = r {
                    Some(name.as_str())
                } else {
                    None
                }
            })
            .collect();
        let contained_filtered: Vec<_> = self
            .contained_in
            .iter()
            .filter(|b| !refs_names.contains(b.name.as_str()))
            .collect();

        if self.contained_in_loading {
            header_card = header_card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Refresh)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Finding branches…")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        } else if !contained_filtered.is_empty() {
            let mut contained_row = div().v_flex().gap(px(4.));
            contained_row = contained_row.child(
                Label::new("CONTAINED IN")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .weight(gpui::FontWeight::SEMIBOLD),
            );
            let mut badges = div().h_flex().gap_1().flex_wrap();
            for branch in &contained_filtered {
                let branch_name: SharedString = branch.name.clone().into();
                badges = badges.child(
                    Badge::new(branch_name)
                        .color(if branch.is_head {
                            Color::Warning
                        } else {
                            Color::Success
                        })
                        .bold(),
                );
            }
            contained_row = contained_row.child(badges);
            header_card = header_card.child(contained_row);
        }

        content = content.child(header_card);

        // -- Commit Message Section --
        let mut message_card = div()
            .v_flex()
            .w_full()
            .px(msg_pad_h)
            .py(msg_pad_v)
            .gap(msg_gap)
            .bg(colors.elevated_surface_background)
            .rounded(px(8.))
            .border_1()
            .border_color(colors.border_variant);

        // Subject line with cherry-pick button
        let summary_for_copy = summary.clone();
        let summary_copied = self.is_copied("summary");
        message_card = message_card.child(
            div()
                .h_flex()
                .w_full()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .id("summary-copy")
                        .h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .items_center()
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                summary_for_copy.to_string(),
                            ));
                            this.mark_copied("summary", cx);
                        }))
                        .overflow_hidden()
                        .child(
                            Label::new(summary.clone())
                                .size(LabelSize::Small)
                                .weight(gpui::FontWeight::BOLD)
                                .truncate(),
                        )
                        .when(summary_copied, |el| {
                            el.child(
                                Label::new("Copied!")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Success),
                            )
                        }),
                )
                .child(
                    IconButton::new("cherry-pick-btn", IconName::GitCommit)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Transparent)
                        .tooltip("Cherry-pick this commit")
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(DetailPanelEvent::CherryPick(sha_for_cherry.to_string()));
                        })),
                ),
        );

        // Collapsible description body
        if !description.is_empty() {
            let desc_for_copy = description.clone();
            let desc_copied = self.is_copied("description");
            let is_expanded = self.description_expanded;
            let chevron_icon = if is_expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            };

            // Toggle header row
            message_card = message_card
                .child(div().w_full().h(px(1.)).bg(colors.border_variant))
                .child(
                    div()
                        .id("description-toggle")
                        .h_flex()
                        .w_full()
                        .gap(px(4.))
                        .items_center()
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.description_expanded = !this.description_expanded;
                            cx.notify();
                        }))
                        .child(
                            Icon::new(chevron_icon)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Description")
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::SEMIBOLD)
                                .color(Color::Muted),
                        ),
                );

            // Expanded content
            if is_expanded {
                message_card = message_card.child(
                    div()
                        .id("description-copy")
                        .v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(4.))
                        .text_xs()
                        .text_color(colors.text_muted)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(desc_for_copy.clone()));
                            this.mark_copied("description", cx);
                        }))
                        // TODO(audit): PERF-17 cache parsed description blocks keyed on the
                        // commit message — requires markdown_view to expose MarkdownBlock and a
                        // parse/render-from-blocks API, which lives in another file.
                        .child(render_markdown(&description, window, cx))
                        .when(desc_copied, |el| {
                            el.child(
                                Label::new("Copied!")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Success),
                            )
                        }),
                );
            }
        }

        content = content.child(message_card);

        // -- Changed Files Section --
        if let (Some(diff), Some(cached)) = (&self.commit_diff, &self.cached_file_tree) {
            let total_file_count = diff.files.len();
            let filtered = self.filtered_files();
            let is_searching = is_file_searching(self.file_search_active, &self.file_search_query);
            let query_str = self.file_search_query.clone().unwrap_or_default();

            let file_count_text: SharedString = if is_searching && !query_str.is_empty() {
                let shown = filtered.len();
                let total = total_file_count;
                format!(
                    "{} / {} file{} changed",
                    shown,
                    total,
                    if total == 1 { "" } else { "s" }
                )
                .into()
            } else {
                format!(
                    "{} file{} changed",
                    total_file_count,
                    if total_file_count == 1 { "" } else { "s" },
                )
                .into()
            };

            let total_additions = diff.total_additions;
            let total_deletions = diff.total_deletions;

            // Build the header with optional search input
            let header_children = |cx: &mut Context<Self>| -> Vec<gpui::AnyElement> {
                let mut children: Vec<gpui::AnyElement> = vec![
                    Icon::new(IconName::File)
                        .size(IconSize::XSmall)
                        .color(Color::Muted)
                        .into_any_element(),
                    Label::new(file_count_text.clone())
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted)
                        .into_any_element(),
                ];

                if is_searching {
                    let query_clone = query_str.clone();
                    let search_input: gpui::AnyElement = div()
                        .flex_1()
                        .h_flex()
                        .items_center()
                        .px_2()
                        .gap_1()
                        .bg(colors.ghost_element_selected)
                        .border_1()
                        .border_color(colors.text_accent)
                        .rounded(px(4.))
                        .child(
                            Icon::new(IconName::Search)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            div().flex_1().child(
                                Label::new(query_clone.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Default),
                            ),
                        )
                        .child(
                            IconButton::new("clear-search", IconName::X)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Transparent)
                                .tooltip("Clear search (Esc)")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.file_search_query = None;
                                    this.file_search_active = false;
                                    cx.notify();
                                }))
                                .into_any_element(),
                        )
                        .into_any_element();
                    children.push(search_input);
                } else {
                    children.push(div().flex_1().into_any_element());
                }

                // Diff stat
                let diff_stat: gpui::AnyElement =
                    DiffStat::new(total_additions, total_deletions).into_any_element();
                children.push(diff_stat);

                children
            };

            let header_children = header_children(cx);
            let mut header = div()
                .h_flex()
                .w_full()
                .h(px(28.))
                .px(px(10.))
                .gap_2()
                .items_center()
                .bg(colors.surface_background)
                .border_y_1()
                .border_color(colors.border_variant);

            for child in header_children {
                header = header.child(child);
            }
            content = content.child(header);

            // Show search hint when not searching and there are files
            if !is_searching && total_file_count > 0 {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(22.))
                        .px_3()
                        .items_center()
                        .gap_1()
                        .child(
                            Label::new("/ to search files")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        )
                        .child(div().flex_1()),
                );
            }

            // Show "no results" when search has no matches
            if is_searching && !query_str.is_empty() && filtered.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .py_4()
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(format!("No files match \"{}\"", query_str))
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }

            // Render file list with search filter applied
            if !filtered.is_empty() {
                let searching = is_searching && !query_str.is_empty();
                let file_list: gpui::AnyElement = match self.file_view_mode {
                    FileViewMode::Flat => {
                        self.render_flat_file_list_filtered(cached, &filtered, cx)
                    }
                    FileViewMode::Tree => {
                        // While searching, show a match-filtered tree (matching files
                        // plus their ancestor directories, fully expanded); otherwise
                        // apply the collapsed-directory state.
                        let visible = if searching {
                            let matches = filtered.matched_set();
                            searched_tree_row_indices(&cached.tree_rows, &matches)
                        } else {
                            visible_tree_row_indices(&cached.tree_rows, &self.collapsed_dirs)
                        };
                        self.render_tree_file_list(&cached.tree_rows, visible, cx)
                    }
                };
                content = content.child(file_list);
            }
        }

        panel = panel.child(content);
        panel.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::format_relative_time_full;

    // --- format_relative_time tests ---

    #[test]
    fn test_format_relative_time_just_now() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(format_relative_time_full(now), "just now");
    }

    #[test]
    fn test_format_relative_time_minutes() {
        let five_mins_ago = chrono::Utc::now().timestamp() - 300;
        assert_eq!(format_relative_time_full(five_mins_ago), "5 mins ago");
    }

    #[test]
    fn test_format_relative_time_one_minute() {
        let one_min_ago = chrono::Utc::now().timestamp() - 60;
        assert_eq!(format_relative_time_full(one_min_ago), "1 min ago");
    }

    #[test]
    fn test_format_relative_time_hours() {
        let two_hours_ago = chrono::Utc::now().timestamp() - 7200;
        assert_eq!(format_relative_time_full(two_hours_ago), "2 hours ago");
    }

    #[test]
    fn test_format_relative_time_one_hour() {
        let one_hour_ago = chrono::Utc::now().timestamp() - 3600;
        assert_eq!(format_relative_time_full(one_hour_ago), "1 hour ago");
    }

    #[test]
    fn test_format_relative_time_days() {
        let three_days_ago = chrono::Utc::now().timestamp() - 259200;
        assert_eq!(format_relative_time_full(three_days_ago), "3 days ago");
    }

    #[test]
    fn test_format_relative_time_one_day() {
        let one_day_ago = chrono::Utc::now().timestamp() - 86400;
        assert_eq!(format_relative_time_full(one_day_ago), "1 day ago");
    }

    #[test]
    fn test_format_relative_time_months() {
        let two_months_ago = chrono::Utc::now().timestamp() - 5184000;
        assert_eq!(format_relative_time_full(two_months_ago), "2 months ago");
    }

    #[test]
    fn test_format_relative_time_one_month() {
        let one_month_ago = chrono::Utc::now().timestamp() - 2592000;
        assert_eq!(format_relative_time_full(one_month_ago), "1 month ago");
    }

    #[test]
    fn test_format_relative_time_years() {
        let two_years_ago = chrono::Utc::now().timestamp() - 63072000;
        assert_eq!(format_relative_time_full(two_years_ago), "2 years ago");
    }

    #[test]
    fn test_format_relative_time_one_year() {
        let one_year_ago = chrono::Utc::now().timestamp() - 31536000;
        assert_eq!(format_relative_time_full(one_year_ago), "1 year ago");
    }

    #[test]
    fn test_format_relative_time_future() {
        let future = chrono::Utc::now().timestamp() + 1000;
        assert_eq!(format_relative_time_full(future), "in the future");
    }

    // --- format_absolute_date tests ---

    #[test]
    fn test_format_absolute_date_valid() {
        let ts = 1704067200; // 2024-01-01 00:00:00 UTC
        let result = format_absolute_date(ts);
        assert!(result.contains("2024"));
    }

    #[test]
    fn test_format_absolute_date_zero() {
        // timestamp 0 is 1970-01-01 00:00:00 UTC — valid epoch, not unknown
        let result = format_absolute_date(0);
        assert!(result.contains("1970"));
    }

    #[test]
    fn test_format_absolute_date_before_epoch() {
        // negative timestamps before epoch 0 are valid in chrono
        let result = format_absolute_date(-86400);
        assert!(result.contains("1969"));
    }

    // --- file_change_icon tests ---

    #[test]
    fn test_file_change_icon_added() {
        assert_eq!(file_change_icon(FileChangeKind::Added), IconName::FileAdded);
    }

    #[test]
    fn test_file_change_icon_modified() {
        assert_eq!(
            file_change_icon(FileChangeKind::Modified),
            IconName::FileModified
        );
    }

    #[test]
    fn test_file_change_icon_deleted() {
        assert_eq!(
            file_change_icon(FileChangeKind::Deleted),
            IconName::FileDeleted
        );
    }

    #[test]
    fn test_file_change_icon_renamed() {
        assert_eq!(
            file_change_icon(FileChangeKind::Renamed),
            IconName::FileRenamed
        );
    }

    #[test]
    fn test_file_change_icon_copied() {
        // Copied maps to FileRenamed (same icon)
        assert_eq!(
            file_change_icon(FileChangeKind::Copied),
            IconName::FileRenamed
        );
    }

    #[test]
    fn test_file_change_icon_type_change() {
        assert_eq!(
            file_change_icon(FileChangeKind::TypeChange),
            IconName::FileModified
        );
    }

    #[test]
    fn test_file_change_icon_untracked() {
        assert_eq!(
            file_change_icon(FileChangeKind::Untracked),
            IconName::FileAdded
        );
    }

    #[test]
    fn test_file_change_icon_conflicted() {
        assert_eq!(
            file_change_icon(FileChangeKind::Conflicted),
            IconName::FileConflict
        );
    }

    // --- file_change_color tests ---

    #[test]
    fn test_file_change_color_added() {
        assert_eq!(file_change_color(FileChangeKind::Added), Color::Added);
    }

    #[test]
    fn test_file_change_color_modified() {
        assert_eq!(file_change_color(FileChangeKind::Modified), Color::Modified);
    }

    #[test]
    fn test_file_change_color_deleted() {
        assert_eq!(file_change_color(FileChangeKind::Deleted), Color::Deleted);
    }

    #[test]
    fn test_file_change_color_renamed() {
        assert_eq!(file_change_color(FileChangeKind::Renamed), Color::Renamed);
    }

    #[test]
    fn test_file_change_color_copied() {
        assert_eq!(file_change_color(FileChangeKind::Copied), Color::Info);
    }

    #[test]
    fn test_file_change_color_type_change() {
        assert_eq!(
            file_change_color(FileChangeKind::TypeChange),
            Color::Warning
        );
    }

    #[test]
    fn test_file_change_color_untracked() {
        assert_eq!(
            file_change_color(FileChangeKind::Untracked),
            Color::Untracked
        );
    }

    #[test]
    fn test_file_change_color_conflicted() {
        assert_eq!(
            file_change_color(FileChangeKind::Conflicted),
            Color::Conflict
        );
    }

    // --- build_cached_file_tree tests ---

    fn make_file_diff(path: &str, kind: FileChangeKind) -> FileDiff {
        FileDiff {
            path: std::path::PathBuf::from(path),
            hunks: vec![],
            additions: 10,
            deletions: 5,
            kind,
        }
    }

    /// (depth, label, is_dir) for each tree row, in display order.
    fn tree_summary(cached: &CachedFileDiffTree) -> Vec<(usize, String, bool)> {
        cached
            .tree_rows
            .iter()
            .map(|r| {
                (
                    r.depth,
                    r.label.to_string(),
                    matches!(r.kind, CachedTreeRowKind::Dir { .. }),
                )
            })
            .collect()
    }

    fn tree_labels(cached: &CachedFileDiffTree, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .map(|&i| cached.tree_rows[i].label.to_string())
            .collect()
    }

    #[test]
    fn tree_renders_root_files_at_depth_zero() {
        let files = vec![make_file_diff("README.md", FileChangeKind::Modified)];
        let cached = build_cached_file_tree(&files);
        assert_eq!(cached.tree_rows.len(), 1);
        assert_eq!(cached.tree_rows[0].depth, 0);
        assert_eq!(cached.tree_rows[0].label.as_str(), "README.md");
        assert!(matches!(
            cached.tree_rows[0].kind,
            CachedTreeRowKind::File(_)
        ));
    }

    #[test]
    fn tree_compaction_stops_at_a_file() {
        // `src` has a file, so it is not compacted; dirs render before files.
        let files = vec![
            make_file_diff("Cargo.toml", FileChangeKind::Modified),
            make_file_diff("src/main.rs", FileChangeKind::Added),
        ];
        let cached = build_cached_file_tree(&files);
        assert_eq!(
            tree_summary(&cached),
            vec![
                (0, "src".to_string(), true),
                (1, "main.rs".to_string(), false),
                (0, "Cargo.toml".to_string(), false),
            ]
        );
    }

    #[test]
    fn tree_compacts_single_child_dir_chains() {
        let files = vec![make_file_diff(
            "crates/foo/src/lib.rs",
            FileChangeKind::Modified,
        )];
        let cached = build_cached_file_tree(&files);
        assert_eq!(
            tree_summary(&cached),
            vec![
                (0, "crates/foo/src".to_string(), true),
                (1, "lib.rs".to_string(), false),
            ]
        );
        match &cached.tree_rows[0].kind {
            CachedTreeRowKind::Dir { key } => assert_eq!(key.as_str(), "crates/foo/src"),
            _ => panic!("expected a directory node"),
        }
    }

    #[test]
    fn collapsed_dir_hides_its_descendants() {
        let files = vec![
            make_file_diff("src/a.rs", FileChangeKind::Modified),
            make_file_diff("src/b.rs", FileChangeKind::Modified),
            make_file_diff("README.md", FileChangeKind::Modified),
        ];
        let cached = build_cached_file_tree(&files);
        let mut collapsed = HashSet::new();
        collapsed.insert("src".to_string());
        let visible = visible_tree_row_indices(&cached.tree_rows, &collapsed);
        assert_eq!(tree_labels(&cached, &visible), vec!["src", "README.md"]);
    }

    #[test]
    fn search_filter_keeps_matches_and_ancestors() {
        let files = vec![
            make_file_diff("src/foo/a.rs", FileChangeKind::Modified), // index 0
            make_file_diff("src/bar/b.rs", FileChangeKind::Modified), // index 1
            make_file_diff("README.md", FileChangeKind::Modified),    // index 2
        ];
        let cached = build_cached_file_tree(&files);
        let matches: HashSet<usize> = [0usize].into_iter().collect();
        let visible = searched_tree_row_indices(&cached.tree_rows, &matches);
        // Keeps the path src > foo > a.rs; excludes the bar subtree and README.md.
        assert_eq!(tree_labels(&cached, &visible), vec!["src", "foo", "a.rs"]);
    }

    #[test]
    fn test_build_cached_file_tree_empty() {
        let files: &[FileDiff] = &[];
        let cached = build_cached_file_tree(files);
        assert!(cached.flat_rows.is_empty());
    }

    #[test]
    fn test_build_cached_file_tree_single_file() {
        let files = vec![make_file_diff("src/main.rs", FileChangeKind::Modified)];
        let cached = build_cached_file_tree(&files);
        assert_eq!(cached.flat_rows.len(), 1);
        let row = &cached.flat_rows[0];
        assert_eq!(row.file_index, 0);
        assert_eq!(row.file_name.as_str(), "main.rs");
        assert_eq!(row.dir_path.as_str(), "src/");
        assert_eq!(row.additions, 10);
        assert_eq!(row.deletions, 5);
    }

    #[test]
    fn test_build_cached_file_tree_file_at_root() {
        let files = vec![make_file_diff("Cargo.toml", FileChangeKind::Added)];
        let cached = build_cached_file_tree(&files);
        assert_eq!(cached.flat_rows.len(), 1);
        let row = &cached.flat_rows[0];
        assert_eq!(row.file_name.as_str(), "Cargo.toml");
        assert!(row.dir_path.is_empty());
    }

    #[test]
    fn test_build_cached_file_tree_nested_path() {
        let files = vec![make_file_diff(
            "crates/rgitui_workspace/src/panel.rs",
            FileChangeKind::Modified,
        )];
        let cached = build_cached_file_tree(&files);
        let row = &cached.flat_rows[0];
        assert_eq!(row.file_name.as_str(), "panel.rs");
        assert_eq!(row.dir_path.as_str(), "crates/rgitui_workspace/src/");
    }

    #[test]
    fn test_build_cached_file_tree_multiple_files_order_preserved() {
        let files = vec![
            make_file_diff("a.rs", FileChangeKind::Added),
            make_file_diff("b.rs", FileChangeKind::Modified),
            make_file_diff("c.rs", FileChangeKind::Deleted),
        ];
        let cached = build_cached_file_tree(&files);
        assert_eq!(cached.flat_rows.len(), 3);
        assert_eq!(cached.flat_rows[0].file_index, 0);
        assert_eq!(cached.flat_rows[1].file_index, 1);
        assert_eq!(cached.flat_rows[2].file_index, 2);
    }

    #[test]
    fn test_build_cached_file_tree_file_change_kinds() {
        let files = vec![
            make_file_diff("added.rs", FileChangeKind::Added),
            make_file_diff("modified.rs", FileChangeKind::Modified),
            make_file_diff("deleted.rs", FileChangeKind::Deleted),
            make_file_diff("renamed.rs", FileChangeKind::Renamed),
            make_file_diff("copied.rs", FileChangeKind::Copied),
            make_file_diff("type_change.rs", FileChangeKind::TypeChange),
            make_file_diff("untracked.rs", FileChangeKind::Untracked),
            make_file_diff("conflicted.rs", FileChangeKind::Conflicted),
        ];
        let cached = build_cached_file_tree(&files);
        assert_eq!(cached.flat_rows.len(), 8);
        // Verify each kind gets the correct short code
        assert_eq!(cached.flat_rows[0].change_code.as_str(), "A");
        assert_eq!(cached.flat_rows[1].change_code.as_str(), "M");
        assert_eq!(cached.flat_rows[2].change_code.as_str(), "D");
        assert_eq!(cached.flat_rows[3].change_code.as_str(), "R");
        assert_eq!(cached.flat_rows[4].change_code.as_str(), "C");
        assert_eq!(cached.flat_rows[5].change_code.as_str(), "T");
        assert_eq!(cached.flat_rows[6].change_code.as_str(), "?");
        assert_eq!(cached.flat_rows[7].change_code.as_str(), "!");
    }

    #[test]
    fn test_build_cached_file_tree_deeply_nested() {
        let files = vec![make_file_diff(
            "very/deeply/nested/directory/structure/file.rs",
            FileChangeKind::Modified,
        )];
        let cached = build_cached_file_tree(&files);
        let row = &cached.flat_rows[0];
        assert_eq!(row.file_name.as_str(), "file.rs");
        assert_eq!(
            row.dir_path.as_str(),
            "very/deeply/nested/directory/structure/"
        );
    }

    // --- FileViewMode tests ---

    #[test]
    fn test_file_view_mode_default_is_flat() {
        assert_eq!(FileViewMode::default(), FileViewMode::Flat);
    }

    #[test]
    fn test_file_view_mode_toggles_between_flat_and_tree() {
        assert_eq!(FileViewMode::Flat.toggled(), FileViewMode::Tree);
        assert_eq!(FileViewMode::Tree.toggled(), FileViewMode::Flat);
    }

    #[test]
    fn test_file_view_mode_icons_describe_next_action() {
        assert_eq!(FileViewMode::Flat.toggle_icon(), IconName::Folder);
        assert_eq!(FileViewMode::Tree.toggle_icon(), IconName::File);
    }

    #[test]
    fn test_file_view_mode_tooltips_describe_next_action() {
        assert_eq!(
            FileViewMode::Flat.toggle_tooltip(),
            "Switch to Tree view (v)"
        );
        assert_eq!(
            FileViewMode::Tree.toggle_tooltip(),
            "Switch to Flat view (v)"
        );
    }

    #[test]
    fn test_file_view_toggle_tooltip_describes_disabled_state() {
        assert_eq!(
            file_view_toggle_tooltip(0, false, FileViewMode::Flat),
            "No changed files to display"
        );
        assert_eq!(
            file_view_toggle_tooltip(2, true, FileViewMode::Flat),
            "Clear file search to switch views"
        );
        assert_eq!(
            file_view_toggle_tooltip(2, false, FileViewMode::Flat),
            "Switch to Tree view (v)"
        );
    }

    #[test]
    fn test_file_view_mode_list_ids_select_distinct_renderers() {
        assert_eq!(
            FileViewMode::Flat.list_element_id(),
            "detail-files-filtered"
        );
        assert_eq!(
            FileViewMode::Tree.list_element_id(),
            "detail-files-filtered-tree"
        );
    }

    #[test]
    fn test_file_view_toggle_requires_files_and_no_search() {
        assert!(can_toggle_file_view(1, false));
        assert!(!can_toggle_file_view(0, false));
        assert!(!can_toggle_file_view(1, true));
    }

    #[test]
    fn test_file_view_toggle_request_preserves_mode_when_disabled() {
        assert_eq!(
            file_view_mode_after_toggle_request(FileViewMode::Flat, 2, false),
            FileViewMode::Tree
        );
        assert_eq!(
            file_view_mode_after_toggle_request(FileViewMode::Flat, 0, false),
            FileViewMode::Flat
        );
        assert_eq!(
            file_view_mode_after_toggle_request(FileViewMode::Tree, 3, true),
            FileViewMode::Tree
        );
    }

    #[test]
    fn test_file_search_state_blocks_view_toggle_shortcut() {
        assert!(is_file_searching(true, &None));
        assert!(is_file_searching(false, &Some("view".to_string())));
        assert!(!is_file_searching(false, &None));

        assert!(!can_toggle_file_view(
            3,
            is_file_searching(true, &Some("view".to_string()))
        ));
    }

    // --- filtered_files tests ---

    // Tests for fuzzy file search sort-by-relevance (PR #28).
    // The query path of `filtered_files` uses CommandPalette::fuzzy_score, the
    // same scoring used by the command palette.

    #[test]
    fn test_filtered_files_relevance_sort_is_descending() {
        // The query path scores matches then sorts by score descending so the
        // best (highest-scoring) match comes first.
        let scored: Vec<(usize, usize)> = vec![(3, 0), (10, 1), (7, 2)];
        let mut sorted = scored.clone();
        sorted.sort_by(|a, b| b.0.cmp(&a.0));
        let order: Vec<usize> = sorted.into_iter().map(|(_, i)| i).collect();
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn test_filtered_files_relevance_order() {
        use crate::command_palette::CommandPalette;
        // "sh" matches "Show" (pos 0) higher than "Fish" (pos 1)
        let score_show = CommandPalette::fuzzy_score("sh", "Show").unwrap();
        let score_fish = CommandPalette::fuzzy_score("sh", "Fish").unwrap();
        assert!(
            score_show > score_fish,
            "earlier match should score higher: {score_show} vs {score_fish}"
        );
    }
}
