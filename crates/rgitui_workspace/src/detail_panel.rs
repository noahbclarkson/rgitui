use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
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
            format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
        }
        3600..=86399 => {
            let hours = diff / 3600;
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        }
        86400..=2591999 => {
            let days = diff / 86400;
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        }
        2592000..=31535999 => {
            let months = diff / 2592000;
            format!(
                "{} month{} ago",
                months,
                if months == 1 { "" } else { "s" }
            )
        }
        _ => {
            let years = diff / 31536000;
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileViewMode {
    Flat,
    Tree,
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
    flat_entries: Vec<CachedFlatEntry>,
    tree: CachedTreeNode,
}

struct CachedFlatEntry {
    file_index: usize,
    file_name: SharedString,
    dir_path: SharedString,
}

#[derive(Default)]
struct CachedTreeNode {
    files: Vec<(usize, SharedString)>,
    children: BTreeMap<String, CachedTreeNode>,
}

fn build_cached_tree(files: &[FileDiff]) -> CachedTreeNode {
    let mut root = CachedTreeNode::default();
    for (i, file) in files.iter().enumerate() {
        let path_str = file.path.display().to_string();
        let parts: Vec<&str> = path_str.split('/').collect();
        let file_name: SharedString = file
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path_str)
            .to_string()
            .into();
        let mut node = &mut root;
        for (pi, part) in parts.iter().enumerate() {
            if pi == parts.len() - 1 {
                node.files.push((i, file_name.clone()));
            } else {
                node = node.children.entry(part.to_string()).or_default();
            }
        }
    }
    sort_cached_tree(&mut root);
    root
}

fn sort_cached_tree(node: &mut CachedTreeNode) {
    node.files
        .sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    for child in node.children.values_mut() {
        sort_cached_tree(child);
    }
}

fn cached_tree_count(node: &CachedTreeNode) -> usize {
    node.files.len()
        + node
            .children
            .values()
            .map(cached_tree_count)
            .sum::<usize>()
}

fn build_cached_file_tree(files: &[FileDiff]) -> CachedFileDiffTree {
    let flat_entries = files
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
            CachedFlatEntry {
                file_index: i,
                file_name,
                dir_path,
            }
        })
        .collect();
    let tree = build_cached_tree(files);
    CachedFileDiffTree {
        flat_entries,
        tree,
    }
}

pub struct DetailPanel {
    commit: Option<CommitInfo>,
    commit_diff: Option<Arc<CommitDiff>>,
    cached_file_tree: Option<CachedFileDiffTree>,
    selected_file_index: Option<usize>,
    focus_handle: FocusHandle,
    file_view_mode: FileViewMode,
    collapsed_dirs: HashSet<String>,
    copied_field: Option<(&'static str, Instant)>,
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
            file_view_mode: FileViewMode::Flat,
            collapsed_dirs: HashSet::new(),
            copied_field: None,
        }
    }

    fn mark_copied(&mut self, field: &'static str, cx: &mut Context<Self>) {
        self.copied_field = Some((field, Instant::now()));
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
        })
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
        let file_count = self.file_count();
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
        self.cached_file_tree = Some(build_cached_file_tree(&diff.files));
        self.commit = Some(commit);
        self.commit_diff = Some(Arc::new(diff));
        self.selected_file_index = None;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.commit = None;
        self.commit_diff = None;
        self.cached_file_tree = None;
        self.selected_file_index = None;
        cx.notify();
    }

    fn is_dir_collapsed(&self, dir: &str) -> bool {
        self.collapsed_dirs.contains(dir)
    }

    fn toggle_dir(&mut self, dir: &str, cx: &mut Context<Self>) {
        let key = dir.to_string();
        if self.collapsed_dirs.contains(&key) {
            self.collapsed_dirs.remove(&key);
        } else {
            self.collapsed_dirs.insert(key);
        }
        cx.notify();
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

        let mut section = div().v_flex().gap(px(4.)).child(
            self.render_section_header("Co-authors"),
        );

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

    fn render_flat_file_list(
        &self,
        diff: &CommitDiff,
        cached: &CachedFileDiffTree,
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

        #[derive(Clone)]
        struct FlatRowData {
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

        let rows: Vec<FlatRowData> = cached
            .flat_entries
            .iter()
            .map(|entry| {
                let file = &diff.files[entry.file_index];
                FlatRowData {
                    file_index: entry.file_index,
                    file_name: entry.file_name.clone(),
                    dir_path: entry.dir_path.clone(),
                    additions: file.additions,
                    deletions: file.deletions,
                    icon_name: file_change_icon(file.kind),
                    icon_color: file_change_color(file.kind),
                    change_code: file.kind.short_code().into(),
                    change_color: file_change_color(file.kind),
                }
            })
            .collect();

        let row_count = rows.len();

        let ghost_element_selected = colors.ghost_element_selected;
        let text_accent = colors.text_accent;
        let border_transparent = colors.border_transparent;
        let ghost_element_hover = colors.ghost_element_hover;

        uniform_list(
            "detail-files-list",
            row_count,
            move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|ix| {
                        let row = &rows[ix];
                        let selected = selected_file_index == Some(row.file_index);
                        let file_idx = row.file_index;
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
                                el.bg(ghost_element_selected)
                                    .border_color(text_accent)
                            })
                            .when(!selected, |el| el.border_color(border_transparent))
                            .hover(move |s| s.bg(ghost_element_hover))
                            .cursor_pointer()
                            .on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    weak.update(cx, |this, cx| {
                                        this.selected_file_index = Some(file_idx);
                                        this.emit_file_selected(cx);
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            )
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
        .flex_1()
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .into_any_element()
    }

    fn render_file_row(
        &self,
        file_list: gpui::Stateful<gpui::Div>,
        i: usize,
        file: &FileDiff,
        file_name: SharedString,
        dir_path: SharedString,
        indent: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = cx.colors().clone();
        let row_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(26.0);

        let additions = file.additions;
        let deletions = file.deletions;
        let selected = self.selected_file_index == Some(i);
        let file_idx = i;

        let icon_name = file_change_icon(file.kind);
        let icon_color = file_change_color(file.kind);
        let change_code: SharedString = file.kind.short_code().into();
        let change_color = file_change_color(file.kind);

        file_list.child(
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
                    Icon::new(icon_name)
                        .size(IconSize::XSmall)
                        .color(icon_color),
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
                            Label::new(change_code)
                                .size(LabelSize::XSmall)
                                .color(change_color)
                                .weight(gpui::FontWeight::BOLD),
                        ),
                )
                .child(DiffStat::new(additions, deletions)),
        )
    }

    fn render_tree_file_list(
        &self,
        diff: &CommitDiff,
        cached: &CachedFileDiffTree,
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut file_list = div()
            .id("detail-files-list")
            .v_flex()
            .w_full()
            .flex_shrink_0()
            .pb_2();

        file_list = self.render_cached_tree_node(file_list, &cached.tree, diff, "", 0, colors, cx);
        file_list
    }

    fn render_cached_tree_node(
        &self,
        mut container: gpui::Stateful<gpui::Div>,
        node: &CachedTreeNode,
        diff: &CommitDiff,
        parent_path: &str,
        depth: usize,
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let row_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(26.0);
        let file_indent = px(if depth == 0 {
            16.0
        } else {
            16.0 + depth as f32 * 14.0
        });

        for (i, file_name) in &node.files {
            let file = &diff.files[*i];
            container = self.render_file_row(
                container,
                *i,
                file,
                file_name.clone(),
                SharedString::default(),
                file_indent,
                cx,
            );
        }

        for (dir_name, child) in &node.children {
            let mut full_dir = if parent_path.is_empty() {
                dir_name.clone()
            } else {
                format!("{}/{}", parent_path, dir_name)
            };
            let mut display_label = format!("{dir_name}/");
            let mut display_node = child;

            while display_node.files.is_empty() && display_node.children.len() == 1 {
                let Some((next_name, next_child)) = display_node.children.iter().next() else {
                    break;
                };
                full_dir = format!("{full_dir}/{next_name}");
                display_label = format!("{display_label}{next_name}/");
                display_node = next_child;
            }

            let dir_collapsed = self.is_dir_collapsed(&full_dir);
            let dir_icon = if dir_collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            };
            let dir_clone = full_dir.clone();
            let dir_indent = px(16.0 + depth as f32 * 14.0);
            let file_count = cached_tree_count(display_node);

            container = container.child(
                div()
                    .id(SharedString::from(format!("detail-dir-{full_dir}")))
                    .h_flex()
                    .w_full()
                    .h(px(row_h))
                    .flex_shrink_0()
                    .px_2()
                    .pl(dir_indent)
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_selected))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_dir(&dir_clone, cx);
                    }))
                    .child(
                        Icon::new(dir_icon)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(SharedString::from(display_label))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .truncate(),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .h_flex()
                            .h(px(16.))
                            .min_w(px(20.))
                            .px(px(6.))
                            .rounded(px(8.))
                            .bg(colors.ghost_element_hover)
                            .items_center()
                            .justify_center()
                            .child(
                                Label::new(SharedString::from(format!("{file_count}")))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    ),
            );

            if !dir_collapsed {
                container = self.render_cached_tree_node(
                    container,
                    display_node,
                    diff,
                    &full_dir,
                    depth + 1,
                    colors,
                    cx,
                );
            }
        }

        container
    }
}

impl Render for DetailPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let row_h = compactness.spacing(26.0);

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
                        .h(px(row_h))
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
                                    Label::new(
                                        "Select a commit from the graph to view details",
                                    )
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
        let relative_time = format_relative_time(commit.time.timestamp());
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
                .h(px(28.))
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
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                ),
        );

        let mut content = div()
            .id("detail-content")
            .v_flex()
            .flex_1()
            .overflow_y_scroll();

        // -- Header Card: Author + SHA + Refs --
        let mut header_card = div()
            .v_flex()
            .w_full()
            .mx_2()
            .mt_2()
            .px_3()
            .py_3()
            .gap(px(10.))
            .bg(colors.elevated_surface_background)
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border_variant);

        // Author row: avatar + name/email + timestamp
        let avatar_url = cx
            .try_global::<AvatarCache>()
            .and_then(|cache| cache.avatar_url(&commit.author.email))
            .map(|s| s.to_string());
        let avatar_bg = colors.border_focused;
        let avatar_text_color = colors.background;
        let avatar_circle =
            self.render_avatar(avatar_url, initials, avatar_bg, avatar_text_color, px(24.));

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
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(
                                    Label::new(author_name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::BOLD)
                                        .truncate(),
                                ),
                        ),
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

        // SHA row
        let sha_copy_clone = sha_for_copy.clone();
        let sha_copied = self.is_copied("sha");
        let sha_icon = if sha_copied {
            IconName::Check
        } else {
            IconName::Copy
        };

        header_card = header_card.child(
            div()
                .h_flex()
                .gap(px(8.))
                .items_center()
                .child(
                    Label::new("SHA")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(
                    div()
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
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            sha_copy_clone.to_string(),
                                        ));
                                        cx.emit(DetailPanelEvent::CopySha(
                                            sha_copy_clone.to_string(),
                                        ));
                                        this.mark_copied("sha", cx);
                                    },
                                )),
                        )
                        .when(sha_copied, |el| {
                            el.child(
                                Label::new("Copied!")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Success),
                            )
                        }),
                )
                .when(!sha_copied, |el| {
                    el.child(
                        Label::new(full_sha)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                }),
        );

        // Parent commits as interactive badges
        if !commit.parent_oids.is_empty() {
            let mut parents_row = div()
                .h_flex()
                .gap(px(8.))
                .items_center()
                .flex_wrap()
                .child(
                    Label::new("Parents")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .weight(gpui::FontWeight::SEMIBOLD),
                );

            for parent_oid in &commit.parent_oids {
                let parent_short: SharedString = format!("{:.7}", parent_oid).into();
                parents_row = parents_row.child(
                    div()
                        .h_flex()
                        .gap(px(4.))
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
                                .child(parent_short),
                        ),
                );
            }

            header_card = header_card.child(parents_row);
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
            .mx_2()
            .mt_2()
            .px_3()
            .py_3()
            .gap(px(8.))
            .bg(colors.elevated_surface_background)
            .rounded(px(6.))
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

        // Description body
        if !description.is_empty() {
            let desc_for_copy = description.clone();
            let desc_copied = self.is_copied("description");
            message_card = message_card.child(
                div()
                    .w_full()
                    .h(px(1.))
                    .bg(colors.border_variant),
            ).child(
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

        content = content.child(message_card);

        // -- Changed Files Section --
        if let (Some(diff), Some(cached)) = (&self.commit_diff, &self.cached_file_tree) {
            let file_count = diff.files.len();
            let file_count_text: SharedString = format!(
                "{} file{} changed",
                file_count,
                if file_count == 1 { "" } else { "s" },
            )
            .into();
            let total_additions = diff.total_additions;
            let total_deletions = diff.total_deletions;

            let is_flat = matches!(self.file_view_mode, FileViewMode::Flat);
            let is_tree = matches!(self.file_view_mode, FileViewMode::Tree);

            content = content.child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(28.))
                    .mt_2()
                    .px(px(10.))
                    .gap(px(4.))
                    .items_center()
                    .bg(colors.surface_background)
                    .border_y_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(IconName::File)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(file_count_text)
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                IconButton::new("flat-view", IconName::Menu)
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Transparent)
                                    .selected(is_flat)
                                    .tooltip("Flat file list")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.file_view_mode = FileViewMode::Flat;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                IconButton::new("tree-view", IconName::Folder)
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Transparent)
                                    .selected(is_tree)
                                    .tooltip("Tree file view")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.file_view_mode = FileViewMode::Tree;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(DiffStat::new(total_additions, total_deletions)),
            );

            let file_list: gpui::AnyElement = match self.file_view_mode {
                FileViewMode::Flat => self.render_flat_file_list(diff, cached, cx),
                FileViewMode::Tree => {
                    self.render_tree_file_list(diff, cached, &colors, cx)
                        .into_any_element()
                }
            };
            content = content.child(file_list);
        }

        panel = panel.child(content);
        panel.into_any_element()
    }
}
