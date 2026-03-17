use std::collections::{BTreeMap, HashSet};

use gpui::prelude::*;
use gpui::{
    div, img, px, ClickEvent, ClipboardItem, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, ObjectFit, Render, SharedString, Window,
};
use rgitui_git::{CommitDiff, CommitInfo, FileChangeKind, FileDiff, RefLabel};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    AvatarCache, Badge, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label,
    LabelSize,
};

use crate::markdown_view::render_markdown;

/// Format a unix timestamp as a human-readable relative time.
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
            format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
        }
        _ => {
            let years = diff / 31536000;
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        }
    }
}

/// Events from the detail panel.
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

#[derive(Default)]
struct FileDiffTreeNode<'a> {
    files: Vec<(usize, &'a FileDiff)>,
    children: BTreeMap<String, FileDiffTreeNode<'a>>,
}

fn build_file_diff_tree(files: &[FileDiff]) -> FileDiffTreeNode<'_> {
    let mut root = FileDiffTreeNode::default();
    for (i, file) in files.iter().enumerate() {
        let path_str = file.path.display().to_string();
        let parts: Vec<&str> = path_str.split('/').collect();
        let mut node = &mut root;
        for (pi, part) in parts.iter().enumerate() {
            if pi == parts.len() - 1 {
                node.files.push((i, file));
            } else {
                node = node
                    .children
                    .entry(part.to_string())
                    .or_default();
            }
        }
    }
    sort_file_diff_tree(&mut root);
    root
}

fn sort_file_diff_tree(node: &mut FileDiffTreeNode<'_>) {
    node.files.sort_by(|a, b| {
        let a_name = a.1.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let b_name = b.1.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        a_name.to_lowercase().cmp(&b_name.to_lowercase())
    });
    for child in node.children.values_mut() {
        sort_file_diff_tree(child);
    }
}

fn file_diff_tree_count(node: &FileDiffTreeNode<'_>) -> usize {
    node.files.len()
        + node
            .children
            .values()
            .map(file_diff_tree_count)
            .sum::<usize>()
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

/// Displays commit metadata and changed files list.
pub struct DetailPanel {
    commit: Option<CommitInfo>,
    commit_diff: Option<CommitDiff>,
    selected_file_index: Option<usize>,
    focus_handle: FocusHandle,
    file_view_mode: FileViewMode,
    collapsed_dirs: HashSet<String>,
}

impl EventEmitter<DetailPanelEvent> for DetailPanel {}

impl DetailPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            commit: None,
            commit_diff: None,
            selected_file_index: None,
            focus_handle: cx.focus_handle(),
            file_view_mode: FileViewMode::Flat,
            collapsed_dirs: HashSet::new(),
        }
    }

    /// Focus the detail panel for keyboard navigation.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Check if the detail panel is currently focused.
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
            "home" | "g" => {
                if self.selected_file_index != Some(0) {
                    self.selected_file_index = Some(0);
                    self.emit_file_selected(cx);
                    cx.notify();
                }
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
        self.commit = Some(commit);
        self.commit_diff = Some(diff);
        self.selected_file_index = None;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.commit = None;
        self.commit_diff = None;
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

    fn render_flat_file_list(
        &self,
        files: &[FileDiff],
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut file_list = div()
            .id("detail-files-list")
            .v_flex()
            .w_full()
            .flex_shrink_0()
            .pb_2();

        for (i, file) in files.iter().enumerate() {
            file_list = self.render_file_row(file_list, i, file, colors, px(12.), cx);
        }

        file_list
    }

    fn render_file_row(
        &self,
        file_list: gpui::Stateful<gpui::Div>,
        i: usize,
        file: &FileDiff,
        colors: &rgitui_theme::ThemeColors,
        indent: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let path_str = file.path.display().to_string();
        let file_name: SharedString = file
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path_str)
            .to_string()
            .into();

        let show_dir_path = matches!(self.file_view_mode, FileViewMode::Flat);
        let dir_path: SharedString = if show_dir_path {
            file.path
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
                .into()
        } else {
            SharedString::default()
        };

        let additions_str: SharedString = format!("+\u{2009}{}", file.additions).into();
        let deletions_str: SharedString = format!("\u{2012}\u{2009}{}", file.deletions).into();
        let selected = self.selected_file_index == Some(i);
        let file_diff = file.clone();
        let path_for_event = path_str;

        let icon_name = file_change_icon(file.kind);
        let icon_color = file_change_color(file.kind);

        file_list.child(
            div()
                .id(ElementId::NamedInteger("detail-file".into(), i as u64))
                .h_flex()
                .w_full()
                .h(px(26.))
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
                .when(!selected, |el| {
                    el.border_color(gpui::Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 0.0,
                        a: 0.0,
                    })
                })
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_file_index = Some(i);
                    cx.emit(DetailPanelEvent::FileSelected(
                        file_diff.clone(),
                        path_for_event.clone(),
                    ));
                    cx.notify();
                }))
                .child(Icon::new(icon_name).size(IconSize::XSmall).color(icon_color))
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
                            Label::new(file_name)
                                .size(LabelSize::XSmall)
                                .truncate(),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .flex_shrink_0()
                        .child(
                            Label::new(additions_str)
                                .size(LabelSize::XSmall)
                                .color(Color::Added),
                        )
                        .child(
                            Label::new(deletions_str)
                                .size(LabelSize::XSmall)
                                .color(Color::Deleted),
                        ),
                ),
        )
    }

    fn render_tree_file_list(
        &self,
        files: &[FileDiff],
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let tree = build_file_diff_tree(files);
        let mut file_list = div()
            .id("detail-files-list")
            .v_flex()
            .w_full()
            .flex_shrink_0()
            .pb_2();

        file_list = self.render_tree_node(file_list, &tree, "", 0, colors, cx);
        file_list
    }

    fn render_tree_node(
        &self,
        mut container: gpui::Stateful<gpui::Div>,
        node: &FileDiffTreeNode<'_>,
        parent_path: &str,
        depth: usize,
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let file_indent = px(if depth == 0 {
            16.0
        } else {
            16.0 + depth as f32 * 14.0
        });

        for &(i, file) in &node.files {
            container = self.render_file_row(container, i, file, colors, file_indent, cx);
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
                let (next_name, next_child) = display_node.children.iter().next().unwrap();
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
            let file_count = file_diff_tree_count(display_node);

            container = container.child(
                div()
                    .id(SharedString::from(format!("detail-dir-{full_dir}")))
                    .h_flex()
                    .w_full()
                    .h(px(26.))
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
                container = self.render_tree_node(
                    container,
                    display_node,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();

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
                        .h(px(26.))
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
                                .gap(px(8.))
                                .px(px(24.))
                                .py(px(16.))
                                .rounded(px(8.))
                                .bg(gpui::Hsla {
                                    a: 0.03,
                                    ..colors.text
                                })
                                .child(
                                    Icon::new(IconName::GitCommit)
                                        .size(IconSize::Large)
                                        .color(Color::Placeholder),
                                )
                                .child(
                                    Label::new("No commit selected")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new("Click a commit in the graph to view details")
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
        let author_email: SharedString = format!("<{}>", commit.author.email).into();
        let relative_time = format_relative_time(commit.time.timestamp());
        let date: SharedString = format!(
            "{} ({})",
            commit.time.format("%Y-%m-%d %H:%M"),
            relative_time
        )
        .into();
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
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(colors.panel_background);

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

        let mut card = div()
            .v_flex()
            .w_full()
            .mx_2()
            .mt_2()
            .px_3()
            .py_2()
            .gap(px(6.))
            .bg(colors.elevated_surface_background)
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border_variant);

        card = card.child(
            div()
                .h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(
                    div().flex_1().min_w_0().child(
                        Label::new(summary)
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::BOLD),
                    ),
                )
                .child(
                    IconButton::new("cherry-pick-btn", IconName::GitCommit)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Transparent)
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(DetailPanelEvent::CherryPick(sha_for_cherry.to_string()));
                        })),
                ),
        );

        if !refs.is_empty() {
            let mut refs_row = div().h_flex().gap_1().flex_wrap();
            for ref_label in &refs {
                let badge_color = match ref_label {
                    RefLabel::Head => Color::Warning,
                    RefLabel::LocalBranch(_) => Color::Success,
                    RefLabel::RemoteBranch(_) => Color::Info,
                    RefLabel::Tag(_) => Color::Accent,
                };
                let name: SharedString = ref_label.display_name().to_string().into();
                let badge = Badge::new(name).color(badge_color).bold();
                refs_row = refs_row.child(badge);
            }
            card = card.child(refs_row);
        }

        if !description.is_empty() {
            card = card.child(
                div()
                    .w_full()
                    .min_w_0()
                    .pt_1()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(render_markdown(&description, _window, cx)),
            );
        }

        let avatar_url = cx
            .try_global::<AvatarCache>()
            .and_then(|cache| cache.avatar_url(&commit.author.email))
            .map(|s| s.to_string());
        let avatar_bg = colors.border_focused;
        let avatar_text_color = colors.background;
        let mut avatar_circle = div()
            .w(px(20.))
            .h(px(20.))
            .rounded_full()
            .bg(avatar_bg)
            .flex()
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
                    .child(initials.clone()),
            );
        }

        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(avatar_circle)
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .h_flex()
                                .gap_1()
                                .overflow_hidden()
                                .child(
                                    Label::new(author_name)
                                        .size(LabelSize::XSmall)
                                        .weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(author_email)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                        .child(Label::new(date).size(LabelSize::XSmall).color(Color::Muted)),
                ),
        );

        let sha_copy_clone = sha_for_copy.clone();
        card = card.child(
            div()
                .h_flex()
                .gap_2()
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
                                .text_color(Color::Accent.color(cx))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(short_sha),
                        )
                        .child(
                            IconButton::new("copy-sha-btn", IconName::Copy)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Transparent)
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            sha_copy_clone.to_string(),
                                        ));
                                        cx.emit(DetailPanelEvent::CopySha(
                                            sha_copy_clone.to_string(),
                                        ));
                                    },
                                )),
                        ),
                )
                .child(
                    Label::new(full_sha)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if !commit.parent_oids.is_empty() {
            let parents_text: SharedString = commit
                .parent_oids
                .iter()
                .map(|oid| format!("{:.7}", oid))
                .collect::<Vec<_>>()
                .join(", ")
                .into();
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Label::new("Parents")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(parents_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        content = content.child(card);

        if let Some(diff) = &self.commit_diff {
            let file_count = diff.files.len();
            let file_count_text: SharedString = format!(
                "{} file{} changed",
                file_count,
                if file_count == 1 { "" } else { "s" },
            )
            .into();
            let additions_text: SharedString =
                format!("+\u{2009}{}", diff.total_additions).into();
            let deletions_text: SharedString =
                format!("\u{2012}\u{2009}{}", diff.total_deletions).into();

            let is_flat = matches!(self.file_view_mode, FileViewMode::Flat);
            let is_tree = matches!(self.file_view_mode, FileViewMode::Tree);

            content = content.child(
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
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.file_view_mode = FileViewMode::Tree;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Label::new(additions_text)
                                    .size(LabelSize::XSmall)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .color(Color::Added),
                            )
                            .child(
                                Label::new(deletions_text)
                                    .size(LabelSize::XSmall)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .color(Color::Deleted),
                            ),
                    ),
            );

            let files = diff.files.clone();
            let file_list = match self.file_view_mode {
                FileViewMode::Flat => self.render_flat_file_list(&files, &colors, cx),
                FileViewMode::Tree => self.render_tree_file_list(&files, &colors, cx),
            };
            content = content.child(file_list);
        }

        panel = panel.child(content);
        panel.into_any_element()
    }
}
