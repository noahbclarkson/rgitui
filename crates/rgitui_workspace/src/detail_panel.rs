use std::collections::HashSet;
use std::sync::Arc;

#[derive(Default, Clone, Copy)]
#[allow(dead_code)]
enum FileViewMode {
    #[default]
    Flat,
    Tree,
}
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, App, ClickEvent, ClipboardItem, Context, ElementId, EventEmitter,
    FocusHandle, KeyDownEvent, ListSizingBehavior, ObjectFit, Render, SharedString, WeakEntity,
    Window,
};
use rgitui_git::{CommitDiff, CommitInfo, FileChangeKind, FileDiff, RefLabel, Signature};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    AvatarCache, Badge, ButtonSize, ButtonStyle, DiffStat, Icon, IconButton, IconName, IconSize,
    Label, LabelSize,
};

use crate::markdown_view::render_markdown;

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
    flat_rows: Vec<CachedFlatRow>,
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

fn build_cached_file_tree(files: &[FileDiff]) -> CachedFileDiffTree {
    let flat_rows = files
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
    CachedFileDiffTree { flat_rows }
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
            _ => {}
        }
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
        log::debug!("DetailPanel::set_commit: oid={} files={}", commit.short_id, diff.files.len());
        self.cached_file_tree = Some(build_cached_file_tree(&diff.files));
        self.commit = Some(commit);
        self.commit_diff = Some(Arc::new(diff));
        self.selected_file_index = None;
        self.file_search_query = None;
        self.file_search_active = false;
        self.collapsed_dirs.clear();
        self.description_expanded = false;
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
        cx.notify();
    }

    /// Returns file indices matching the search query (using fuzzy_score),
    /// or all files if no query is set.
    fn filtered_file_indices(&self) -> Vec<usize> {
        let query = match &self.file_search_query {
            Some(q) if !q.is_empty() => q,
            _ => return (0..self.file_count()).collect(),
        };
        let Some(diff) = &self.commit_diff else {
            return vec![];
        };
        diff.files
            .iter()
            .enumerate()
            .filter_map(|(i, file)| {
                let path = file.path.to_string_lossy();
                crate::command_palette::CommandPalette::fuzzy_score(query, &path).map(|_| i)
            })
            .collect()
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
        filtered_indices: &[usize],
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
        let rows: Vec<_> = filtered_indices
            .iter()
            .filter_map(|&fi| cached.flat_rows.get(fi).cloned())
            .collect();
        let row_count = rows.len();

        let ghost_element_selected = colors.ghost_element_selected;
        let text_accent = colors.text_accent;
        let border_transparent = colors.border_transparent;
        let ghost_element_hover = colors.ghost_element_hover;

        uniform_list(
            "detail-files-filtered",
            row_count,
            move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|ix| {
                        let row = &rows[ix];
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
        .h(px(row_count as f32 * row_h + 8.0))
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .into_any_element()
    }

    /// Renders a filtered tree: only files/dirs whose path matches the search are shown.
    /// Uses the same cached tree but only emits nodes that contain matching files.
    fn render_tree_file_list_filtered(
        &self,
        diff: &CommitDiff,
        _cached: &CachedFileDiffTree,
        colors: &rgitui_theme::ThemeColors,
        filtered_indices: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let filter_set: std::collections::HashSet<usize> =
            filtered_indices.iter().cloned().collect();
        let row_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(26.0);

        let mut file_list = div()
            .id("detail-files-filtered-tree")
            .v_flex()
            .w_full()
            .flex_shrink_0()
            .pb_2();

        // Collect which top-level dirs contain matching files
        let mut matching_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for &fi in filtered_indices {
            if let Some(file) = diff.files.get(fi) {
                let path_str = file.path.display().to_string();
                if let Some(pos) = path_str.rfind('/') {
                    matching_dirs.insert(path_str[..pos].to_string());
                }
            }
        }

        // Render only matching top-level items (files + dirs that have matches)
        for (i, file) in diff.files.iter().enumerate() {
            if !filter_set.contains(&i) {
                continue;
            }
            let path_str = file.path.display().to_string();
            let file_name: SharedString = file
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path_str)
                .to_string()
                .into();
            let dir_path: SharedString = if let Some(pos) = path_str.rfind('/') {
                format!("{}/", &path_str[..pos]).into()
            } else {
                SharedString::default()
            };

            let indent = px(16.0);
            let colors = colors.clone();
            let selected = self.selected_file_index == Some(i);
            let file_idx = i;

            file_list = file_list.child(
                div()
                    .id(ElementId::NamedInteger("detail-file".into(), i as u64))
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
                        el.bg(colors.ghost_element_selected)
                            .border_color(colors.text_accent)
                    })
                    .when(!selected, |el| el.border_color(colors.border_transparent))
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.selected_file_index = Some(file_idx);
                        this.emit_file_selected(cx);
                        cx.notify();
                    }))
                    .child(
                        Icon::new(file_change_icon(file.kind))
                            .size(IconSize::XSmall)
                            .color(file_change_color(file.kind)),
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
                            .child(Label::new(file_name).size(LabelSize::XSmall).truncate()),
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
                                Label::new(file.kind.short_code())
                                    .size(LabelSize::XSmall)
                                    .color(file_change_color(file.kind))
                                    .weight(gpui::FontWeight::BOLD),
                            ),
                    )
                    .child(DiffStat::new(file.additions, file.deletions)),
            );
        }

        file_list
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
                ),
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
                    .font_family("monospace")
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
            let filtered_indices = self.filtered_file_indices();
            let is_searching = self.file_search_active || self.file_search_query.is_some();
            let query_str = self.file_search_query.clone().unwrap_or_default();

            let file_count_text: SharedString = if is_searching && !query_str.is_empty() {
                let shown = filtered_indices.len();
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
            if is_searching && !query_str.is_empty() && filtered_indices.is_empty() {
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
            if !filtered_indices.is_empty() {
                let file_list: gpui::AnyElement = match self.file_view_mode {
                    FileViewMode::Flat => {
                        self.render_flat_file_list_filtered(cached, &filtered_indices, cx)
                    }
                    FileViewMode::Tree => self
                        .render_tree_file_list_filtered(
                            diff,
                            cached,
                            &colors,
                            &filtered_indices,
                            cx,
                        )
                        .into_any_element(),
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
}
