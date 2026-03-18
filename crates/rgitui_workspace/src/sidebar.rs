use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_git::{BranchInfo, FileChangeKind, FileStatus, RemoteInfo, StashEntry, TagInfo};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, DiffStat, Disclosure, IconButton, IconName, Label, LabelSize,
};

/// Events from the sidebar.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    BranchSelected(String),
    BranchCheckout(String),
    BranchCreate,
    BranchDelete(String),
    BranchRename(String),
    MergeBranch(String),
    RemoteFetch(String),
    RemotePull(String),
    RemotePush(String),
    RemoteRemove(String),
    TagSelected(String),
    TagDelete(String),
    StashSelected(usize),
    StashApply(usize),
    StashDrop(usize),
    FileSelected { path: String, staged: bool },
    StageFile(String),
    UnstageFile(String),
    StageAll,
    UnstageAll,
    DiscardFile(String),
    OpenRepo,
}

/// Sidebar sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SidebarSection {
    LocalBranches,
    Remotes,
    RemoteBranches,
    Tags,
    Stashes,
    StagedChanges,
    UnstagedChanges,
}

#[derive(Default)]
struct FileTreeNode<'a> {
    files: Vec<&'a FileStatus>,
    children: BTreeMap<String, FileTreeNode<'a>>,
}

/// Represents a navigable item in the sidebar's flat list.
#[derive(Debug, Clone)]
enum SidebarItem {
    SectionHeader(SidebarSection),
    LocalBranch(usize),   // index into local branches
    Remote(usize),        // index into remotes
    RemoteBranch(usize),  // index into remote branches
    Tag(usize),           // index into tags
    Stash(usize),         // index into stashes
    StagedFile(usize),    // index into staged files
    UnstagedFile(usize),  // index into unstaged files
}

struct FileRowCtx<'a> {
    staged: bool,
    indent: gpui::Pixels,
    file_idx: usize,
    colors: &'a rgitui_theme::ThemeColors,
}

/// The left sidebar panel with branches, tags, stashes, and working tree status.
pub struct Sidebar {
    expanded_sections: Vec<SidebarSection>,
    branches: Vec<BranchInfo>,
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    stashes: Vec<StashEntry>,
    staged: Vec<FileStatus>,
    unstaged: Vec<FileStatus>,
    selected_file: Option<(String, bool)>, // (path, is_staged)
    /// Tracks which directory groups are collapsed in the file change sections.
    /// Key is "staged:<dir>" or "unstaged:<dir>".
    collapsed_dirs: HashSet<String>,
    /// Current repository name displayed in the sidebar header.
    repo_name: String,
    /// Focus handle for keyboard navigation.
    focus_handle: FocusHandle,
    /// Index into the flat navigable items list for keyboard nav.
    keyboard_index: Option<usize>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            expanded_sections: vec![
                SidebarSection::LocalBranches,
                SidebarSection::Remotes,
                SidebarSection::StagedChanges,
                SidebarSection::UnstagedChanges,
            ],
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            staged: Vec::new(),
            unstaged: Vec::new(),
            selected_file: None,
            collapsed_dirs: HashSet::new(),
            repo_name: String::new(),
            focus_handle: cx.focus_handle(),
            keyboard_index: None,
        }
    }

    /// Build the flat list of navigable items based on current expansion state.
    fn navigable_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();

        // Local branches section
        items.push(SidebarItem::SectionHeader(SidebarSection::LocalBranches));
        if self.is_expanded(SidebarSection::LocalBranches) {
            let local_count = self.branches.iter().filter(|b| !b.is_remote).count();
            for i in 0..local_count {
                items.push(SidebarItem::LocalBranch(i));
            }
        }

        // Remotes section
        items.push(SidebarItem::SectionHeader(SidebarSection::Remotes));
        if self.is_expanded(SidebarSection::Remotes) {
            for i in 0..self.remotes.len() {
                items.push(SidebarItem::Remote(i));
            }
        }

        // Remote branches section
        items.push(SidebarItem::SectionHeader(SidebarSection::RemoteBranches));
        if self.is_expanded(SidebarSection::RemoteBranches) {
            let remote_count = self.branches.iter().filter(|b| b.is_remote).count();
            for i in 0..remote_count {
                items.push(SidebarItem::RemoteBranch(i));
            }
        }

        // Tags section
        items.push(SidebarItem::SectionHeader(SidebarSection::Tags));
        if self.is_expanded(SidebarSection::Tags) {
            for i in 0..self.tags.len() {
                items.push(SidebarItem::Tag(i));
            }
        }

        // Stashes section
        items.push(SidebarItem::SectionHeader(SidebarSection::Stashes));
        if self.is_expanded(SidebarSection::Stashes) {
            for i in 0..self.stashes.len() {
                items.push(SidebarItem::Stash(i));
            }
        }

        // Staged changes section
        items.push(SidebarItem::SectionHeader(SidebarSection::StagedChanges));
        if self.is_expanded(SidebarSection::StagedChanges) {
            for i in 0..self.staged.len() {
                items.push(SidebarItem::StagedFile(i));
            }
        }

        // Unstaged changes section
        items.push(SidebarItem::SectionHeader(SidebarSection::UnstagedChanges));
        if self.is_expanded(SidebarSection::UnstagedChanges) {
            for i in 0..self.unstaged.len() {
                items.push(SidebarItem::UnstagedFile(i));
            }
        }

        items
    }

    /// Activate the currently selected keyboard item (Enter key).
    fn activate_keyboard_item(&mut self, cx: &mut Context<Self>) {
        let items = self.navigable_items();
        let Some(idx) = self.keyboard_index else {
            return;
        };
        let Some(item) = items.get(idx) else {
            return;
        };

        match item {
            SidebarItem::SectionHeader(section) => {
                self.toggle_section(*section, cx);
            }
            SidebarItem::LocalBranch(i) => {
                let local_branches: Vec<_> =
                    self.branches.iter().filter(|b| !b.is_remote).collect();
                if let Some(branch) = local_branches.get(*i) {
                    cx.emit(SidebarEvent::BranchCheckout(branch.name.clone()));
                }
            }
            SidebarItem::Remote(i) => {
                // Select the remote (could trigger fetch in the future)
                if let Some(remote) = self.remotes.get(*i) {
                    cx.emit(SidebarEvent::RemoteFetch(remote.name.clone()));
                }
            }
            SidebarItem::RemoteBranch(i) => {
                let remote_branches: Vec<_> =
                    self.branches.iter().filter(|b| b.is_remote).collect();
                if let Some(branch) = remote_branches.get(*i) {
                    cx.emit(SidebarEvent::BranchSelected(branch.name.clone()));
                }
            }
            SidebarItem::Tag(i) => {
                if let Some(tag) = self.tags.get(*i) {
                    cx.emit(SidebarEvent::TagSelected(tag.name.clone()));
                }
            }
            SidebarItem::Stash(i) => {
                cx.emit(SidebarEvent::StashSelected(*i));
            }
            SidebarItem::StagedFile(i) => {
                if let Some(file) = self.staged.get(*i) {
                    let path = file.path.display().to_string();
                    self.selected_file = Some((path.clone(), true));
                    cx.emit(SidebarEvent::FileSelected { path, staged: true });
                    cx.notify();
                }
            }
            SidebarItem::UnstagedFile(i) => {
                if let Some(file) = self.unstaged.get(*i) {
                    let path = file.path.display().to_string();
                    self.selected_file = Some((path.clone(), false));
                    cx.emit(SidebarEvent::FileSelected {
                        path,
                        staged: false,
                    });
                    cx.notify();
                }
            }
        }
    }

    /// Handle keyboard events for sidebar navigation.
    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let ctrl = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;

        if ctrl {
            return; // Let workspace handle Ctrl+ shortcuts
        }

        let items = self.navigable_items();
        if items.is_empty() {
            return;
        }

        match key {
            "up" | "k" => {
                let new_idx = match self.keyboard_index {
                    Some(i) if i > 0 => i - 1,
                    Some(_) => 0,
                    None => 0,
                };
                self.keyboard_index = Some(new_idx);
                cx.notify();
            }
            "down" | "j" => {
                let max = items.len().saturating_sub(1);
                let new_idx = match self.keyboard_index {
                    Some(i) => (i + 1).min(max),
                    None => 0,
                };
                self.keyboard_index = Some(new_idx);
                cx.notify();
            }
            "enter" | " " => {
                self.activate_keyboard_item(cx);
            }
            "home" => {
                self.keyboard_index = Some(0);
                cx.notify();
            }
            "end" => {
                self.keyboard_index = Some(items.len().saturating_sub(1));
                cx.notify();
            }
            "s" => {
                // Stage/unstage the selected file
                if let Some(idx) = self.keyboard_index {
                    if let Some(item) = items.get(idx) {
                        match item {
                            SidebarItem::StagedFile(i) => {
                                if let Some(file) = self.staged.get(*i) {
                                    cx.emit(SidebarEvent::UnstageFile(
                                        file.path.display().to_string(),
                                    ));
                                }
                            }
                            SidebarItem::UnstagedFile(i) => {
                                if let Some(file) = self.unstaged.get(*i) {
                                    cx.emit(SidebarEvent::StageFile(
                                        file.path.display().to_string(),
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "x" | "delete" => {
                // Delete the selected item (tag, stash, branch, file)
                if let Some(idx) = self.keyboard_index {
                    let items = self.navigable_items();
                    if let Some(item) = items.get(idx) {
                        match item {
                            SidebarItem::Tag(i) => {
                                if let Some(tag) = self.tags.get(*i) {
                                    cx.emit(SidebarEvent::TagDelete(tag.name.clone()));
                                }
                            }
                            SidebarItem::Stash(i) => {
                                cx.emit(SidebarEvent::StashDrop(*i));
                            }
                            SidebarItem::LocalBranch(i) => {
                                let local_branches: Vec<_> =
                                    self.branches.iter().filter(|b| !b.is_remote).collect();
                                if let Some(branch) = local_branches.get(*i) {
                                    if !branch.is_head {
                                        cx.emit(SidebarEvent::BranchDelete(
                                            branch.name.clone(),
                                        ));
                                    }
                                }
                            }
                            SidebarItem::UnstagedFile(i) => {
                                if let Some(file) = self.unstaged.get(*i) {
                                    cx.emit(SidebarEvent::DiscardFile(
                                        file.path.display().to_string(),
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Focus the sidebar for keyboard navigation.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Check if the sidebar is currently focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn set_repo_name(&mut self, name: String, cx: &mut Context<Self>) {
        self.repo_name = name;
        cx.notify();
    }

    pub fn update_branches(&mut self, mut branches: Vec<BranchInfo>, cx: &mut Context<Self>) {
        // Sort branches alphabetically, with HEAD branch first among locals
        branches.sort_by(|a, b| {
            if a.is_remote != b.is_remote {
                return a.is_remote.cmp(&b.is_remote);
            }
            // HEAD branch sorts first within its group
            if a.is_head != b.is_head {
                return b.is_head.cmp(&a.is_head);
            }
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        });
        self.branches = branches;
        cx.notify();
    }

    pub fn update_tags(&mut self, mut tags: Vec<TagInfo>, cx: &mut Context<Self>) {
        tags.sort_by_key(|a| a.name.to_lowercase());
        self.tags = tags;
        cx.notify();
    }

    pub fn update_remotes(&mut self, remotes: Vec<RemoteInfo>, cx: &mut Context<Self>) {
        self.remotes = remotes;
        cx.notify();
    }

    pub fn update_stashes(&mut self, stashes: Vec<StashEntry>, cx: &mut Context<Self>) {
        self.stashes = stashes;
        cx.notify();
    }

    pub fn update_status(
        &mut self,
        staged: Vec<FileStatus>,
        unstaged: Vec<FileStatus>,
        cx: &mut Context<Self>,
    ) {
        self.staged = staged;
        self.unstaged = unstaged;
        // Note: we intentionally do NOT reset collapsed_dirs here
        // so that collapsed directory groups persist across refreshes.
        cx.notify();
    }

    fn is_expanded(&self, section: SidebarSection) -> bool {
        self.expanded_sections.contains(&section)
    }

    fn toggle_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if let Some(pos) = self.expanded_sections.iter().position(|s| *s == section) {
            self.expanded_sections.remove(pos);
        } else {
            self.expanded_sections.push(section);
        }
        cx.notify();
    }

    fn is_dir_collapsed(&self, prefix: &str, dir: &str) -> bool {
        self.collapsed_dirs.contains(&format!("{}:{}", prefix, dir))
    }

    fn toggle_dir(&mut self, prefix: &str, dir: &str, cx: &mut Context<Self>) {
        let key = format!("{}:{}", prefix, dir);
        if self.collapsed_dirs.contains(&key) {
            self.collapsed_dirs.remove(&key);
        } else {
            self.collapsed_dirs.insert(key);
        }
        cx.notify();
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

    fn file_change_symbol(kind: FileChangeKind) -> &'static str {
        match kind {
            FileChangeKind::Added => "+",
            FileChangeKind::Modified => "~",
            FileChangeKind::Deleted => "-",
            FileChangeKind::Renamed => "R",
            FileChangeKind::Copied => "C",
            FileChangeKind::TypeChange => "T",
            FileChangeKind::Untracked => "?",
            FileChangeKind::Conflicted => "!",
        }
    }

    fn file_kind_counts(files: &[FileStatus]) -> HashMap<FileChangeKind, usize> {
        let mut counts: HashMap<FileChangeKind, usize> = HashMap::new();
        for file in files {
            *counts.entry(file.kind).or_insert(0) += 1;
        }
        counts
    }

    /// Map a file change kind to an appropriate icon.
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

    fn build_file_tree(files: &[FileStatus]) -> FileTreeNode<'_> {
        let mut root = FileTreeNode::default();
        for file in files {
            let mut node = &mut root;
            if let Some(parent) = file.path.parent() {
                for component in parent.iter().filter_map(|part| part.to_str()) {
                    if component.is_empty() {
                        continue;
                    }
                    node = node.children.entry(component.to_string()).or_default();
                }
            }
            node.files.push(file);
        }
        Self::sort_file_tree(&mut root);
        root
    }

    fn sort_file_tree(node: &mut FileTreeNode<'_>) {
        node.files.sort_by(|a, b| {
            let a_name = a.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let b_name = b.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            a_name.to_lowercase().cmp(&b_name.to_lowercase())
        });
        for child in node.children.values_mut() {
            Self::sort_file_tree(child);
        }
    }

    fn file_tree_file_count(node: &FileTreeNode<'_>) -> usize {
        node.files.len()
            + node
                .children
                .values()
                .map(Self::file_tree_file_count)
                .sum::<usize>()
    }

    fn render_file_row(
        &self,
        body: gpui::Stateful<gpui::Div>,
        file: &FileStatus,
        ctx: &mut FileRowCtx<'_>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let item_h = cx.global::<SettingsState>().settings().compactness.spacing(24.0);
        let staged = ctx.staged;
        let indent = ctx.indent;
        let colors = ctx.colors;
        let file_idx = &mut ctx.file_idx;
        let i = *file_idx;
        *file_idx += 1;

        let file_name: SharedString = file
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
            .into();
        let kind_icon = Self::file_change_icon(file.kind);
        let color = Self::file_change_color(file.kind);
        let additions = file.additions;
        let deletions = file.deletions;
        let file_path = file.path.display().to_string();
        let file_path_select = file_path.clone();
        let file_path_primary = file_path.clone();
        let file_path_secondary = file_path.clone();
        let is_selected = self
            .selected_file
            .as_ref()
            .is_some_and(|(p, s)| p == &file_path && *s == staged);
        let has_stats = additions > 0 || deletions > 0;

        let mut row = div()
            .id(ElementId::NamedInteger(
                if staged {
                    "staged-file".into()
                } else {
                    "unstaged-file".into()
                },
                i as u64,
            ))
            .group("sidebar-file-row")
            .h_flex()
            .w_full()
            .h(px(item_h))
            .flex_shrink_0()
            .px_2()
            .pl(indent)
            .gap_1()
            .items_center()
            .when(is_selected, |el| el.bg(colors.ghost_element_selected))
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_selected))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.selected_file = Some((file_path_select.clone(), staged));
                cx.emit(SidebarEvent::FileSelected {
                    path: file_path_select.clone(),
                    staged,
                });
                cx.notify();
            }))
            .child(
                rgitui_ui::Icon::new(kind_icon)
                    .size(rgitui_ui::IconSize::XSmall)
                    .color(color),
            )
            .child(
                Label::new(file_name)
                    .size(LabelSize::XSmall)
                    .color(color)
                    .truncate(),
            )
            .child(div().flex_1());

        if has_stats {
            row = row.child(DiffStat::new(additions, deletions));
        }

        let stage_color = if staged {
            Color::Deleted.color(cx)
        } else {
            Color::Added.color(cx)
        };
        let discard_color = Color::Deleted.color(cx);
        let ghost_hover = colors.ghost_element_hover;

        row = row.child(
            div()
                .id(ElementId::NamedInteger(
                    if staged {
                        "unstage-btn".into()
                    } else {
                        "stage-discard-btn".into()
                    },
                    i as u64,
                ))
                .h_flex()
                .gap(px(2.))
                .invisible()
                .group_hover("sidebar-file-row", |s| s.visible())
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "{}-{}",
                            if staged { "unstage" } else { "stage" },
                            i
                        )))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(18.))
                        .h(px(18.))
                        .rounded(px(3.))
                        .text_color(stage_color)
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .hover(move |s| s.bg(ghost_hover))
                        .cursor_pointer()
                        .on_click(cx.listener(
                            move |_this, _: &ClickEvent, _, cx| {
                                cx.emit(if staged {
                                    SidebarEvent::UnstageFile(file_path_primary.clone())
                                } else {
                                    SidebarEvent::StageFile(file_path_primary.clone())
                                });
                            },
                        ))
                        .child(if staged { "\u{2212}" } else { "+" }),
                ),
        );

        if !staged {
            let ghost_hover2 = colors.ghost_element_hover;
            row = row.child(
                div()
                    .id(SharedString::from(format!("discard-{}", i)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(18.))
                    .h(px(18.))
                    .rounded(px(3.))
                    .text_color(discard_color)
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .hover(move |s| s.bg(ghost_hover2))
                    .cursor_pointer()
                    .invisible()
                    .group_hover("sidebar-file-row", |s| s.visible())
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SidebarEvent::DiscardFile(file_path_secondary.clone()));
                    }))
                    .child("\u{00d7}"),
            );
        }

        body.child(row)
    }

    fn render_file_tree(
        &self,
        mut body: gpui::Stateful<gpui::Div>,
        node: &FileTreeNode<'_>,
        tree_ctx: (&str, &str, usize),
        ctx: &mut FileRowCtx<'_>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let item_h = cx.global::<SettingsState>().settings().compactness.spacing(24.0);
        let (prefix, parent_path, depth) = tree_ctx;
        let file_indent = px(if depth == 0 {
            16.0
        } else {
            16.0 + depth as f32 * 14.0
        });

        for file in &node.files {
            ctx.indent = file_indent;
            body = self.render_file_row(body, file, ctx, cx);
        }
        let colors = ctx.colors;

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
                display_label = format!("{}{next_name}/", display_label);
                display_node = next_child;
            }

            let dir_collapsed = self.is_dir_collapsed(prefix, &full_dir);
            let dir_icon = if dir_collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            };
            let prefix_key = prefix.to_string();
            let dir_clone = full_dir.clone();
            let dir_indent = px(16.0 + depth as f32 * 14.0);
            let file_count = Self::file_tree_file_count(display_node);

            body = body.child(
                div()
                    .id(SharedString::from(format!("{prefix}-dir-{full_dir}")))
                    .h_flex()
                    .w_full()
                    .h(px(item_h))
                    .flex_shrink_0()
                    .px_2()
                    .pl(dir_indent)
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_dir(&prefix_key, &dir_clone, cx);
                    }))
                    .child(
                        rgitui_ui::Icon::new(dir_icon)
                            .size(rgitui_ui::IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        rgitui_ui::Icon::new(IconName::Folder)
                            .size(rgitui_ui::IconSize::XSmall)
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
                body = self.render_file_tree(
                    body,
                    display_node,
                    (prefix, &full_dir, depth + 1),
                    ctx,
                    cx,
                );
            }
        }

        body
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let item_h = compactness.spacing(24.0);
        let header_h = compactness.spacing(26.0);

        // Compute navigable items for keyboard highlight matching
        let _nav_items = self.navigable_items();
        let keyboard_index = self.keyboard_index;

        let panel = div()
            .id("sidebar-panel")
            .track_focus(&self.focus_handle)
            .key_context("Sidebar")
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w_full()
            .h_full()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border_variant);

        let mut content = div()
            .id("sidebar-content")
            .v_flex()
            .w_full()
            .flex_shrink_0();

        // -- Sidebar Header: repo name + open repo button --
        {
            let repo_label: SharedString = if self.repo_name.is_empty() {
                "No Repository".into()
            } else {
                self.repo_name.clone().into()
            };

            content = content.child(
                div()
                    .id("sidebar-header")
                    .h_flex()
                    .w_full()
                    .h(px(header_h))
                    .px(px(8.))
                    .gap(px(4.))
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        rgitui_ui::Icon::new(IconName::Folder)
                            .size(rgitui_ui::IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(repo_label)
                                .size(LabelSize::Small)
                                .weight(gpui::FontWeight::SEMIBOLD)
                                .truncate(),
                        ),
                    )
                    .child(
                        IconButton::new("sidebar-open-repo", IconName::Plus)
                            .size(ButtonSize::Compact)
                            .color(Color::Muted)
                            .tooltip("Open repository")
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.emit(SidebarEvent::OpenRepo);
                            })),
                    ),
            );
        }

        // Track flat navigation index for keyboard highlighting
        let mut nav_idx: usize = 0;
        let kb_accent = colors.border_focused;

        // -- Local Branches --
        let local_branches: Vec<BranchInfo> = self
            .branches
            .iter()
            .filter(|b| !b.is_remote)
            .cloned()
            .collect();

        let local_expanded = self.is_expanded(SidebarSection::LocalBranches);
        let icon = if local_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-local-branches")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::LocalBranches, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::GitBranch)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Branches")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(15.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", local_branches.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if local_expanded {
            if local_branches.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No local branches")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for (i, branch) in local_branches.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name: SharedString = branch.name.clone().into();
                let branch_name = branch.name.clone();
                let branch_name_select = branch.name.clone();
                let branch_name_merge = branch.name.clone();
                let branch_name_rename = branch.name.clone();
                let is_head = branch.is_head;

                let mut item = div()
                    .id(ElementId::NamedInteger("local-branch".into(), i as u64))
                    .h_flex()
                    .w_full()
                    .h(px(item_h))
                    .px_2()
                    .pl(px(16.))
                    .gap(px(4.))
                    .items_center()
                    .when(kb_active, |el| el.bg(colors.ghost_element_hover).border_l_2().border_color(kb_accent))
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                        if event.click_count() >= 2 {
                            // Double-click: checkout this branch
                            cx.emit(SidebarEvent::BranchCheckout(branch_name.clone()));
                        } else {
                            // Single-click: select
                            cx.emit(SidebarEvent::BranchSelected(branch_name_select.clone()));
                        }
                    }));

                if is_head {
                    item = item
                        .bg(colors.ghost_element_selected)
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .child(
                                    rgitui_ui::Icon::new(IconName::Dot)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Accent),
                                ),
                        )
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .child(
                                    rgitui_ui::Icon::new(IconName::GitBranch)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Accent),
                                ),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Accent)
                                .weight(gpui::FontWeight::BOLD)
                                .truncate(),
                        );
                } else {
                    item = item
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .child(
                                    rgitui_ui::Icon::new(IconName::DotOutline)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .child(
                                    rgitui_ui::Icon::new(IconName::GitBranch)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        );
                }

                if branch.ahead > 0 || branch.behind > 0 {
                    item = item.child(div().flex_1()).child(
                        div()
                            .h_flex()
                            .gap(px(4.))
                            .flex_shrink_0()
                            .when(branch.ahead > 0, |el| {
                                el.child(
                                    div()
                                        .h_flex()
                                        .gap(px(1.))
                                        .items_center()
                                        .child(
                                            rgitui_ui::Icon::new(IconName::ArrowUp)
                                                .size(rgitui_ui::IconSize::XSmall)
                                                .color(Color::Success),
                                        )
                                        .child(
                                            Label::new(SharedString::from(format!(
                                                "{}",
                                                branch.ahead
                                            )))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Success),
                                        ),
                                )
                            })
                            .when(branch.behind > 0, |el| {
                                el.child(
                                    div()
                                        .h_flex()
                                        .gap(px(1.))
                                        .items_center()
                                        .child(
                                            rgitui_ui::Icon::new(IconName::ArrowDown)
                                                .size(rgitui_ui::IconSize::XSmall)
                                                .color(Color::Warning),
                                        )
                                        .child(
                                            Label::new(SharedString::from(format!(
                                                "{}",
                                                branch.behind
                                            )))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Warning),
                                        ),
                                )
                            }),
                    );
                }

                // Non-HEAD branches get "Merge" and "Delete" buttons
                if !is_head {
                    let branch_name_delete = branch.name.clone();
                    item = item.child(
                        div()
                            .ml_auto()
                            .h_flex()
                            .gap(px(2.))
                            .child(
                                IconButton::new(
                                    ElementId::NamedInteger("merge-branch".into(), i as u64),
                                    IconName::GitMerge,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Muted)
                                .tooltip("Merge into current branch")
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::MergeBranch(
                                            branch_name_merge.clone(),
                                        ));
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    ElementId::NamedInteger("rename-branch".into(), i as u64),
                                    IconName::Edit,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Muted)
                                .tooltip("Rename branch")
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::BranchRename(
                                            branch_name_rename.clone(),
                                        ));
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    ElementId::NamedInteger("delete-branch".into(), i as u64),
                                    IconName::Trash,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Deleted)
                                .tooltip("Delete branch")
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::BranchDelete(
                                            branch_name_delete.clone(),
                                        ));
                                    },
                                )),
                            ),
                    );
                }

                content = content.child(item);
            }
        }

        // -- Remotes --
        let remotes_expanded = self.is_expanded(SidebarSection::Remotes);
        let icon = if remotes_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-remotes")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::Remotes, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::ExternalLink)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Remotes")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(15.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", self.remotes.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if remotes_expanded {
            if self.remotes.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No remotes configured")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for (i, remote) in self.remotes.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name = remote.name.clone();
                let display_name: SharedString = remote.name.clone().into();
                let url_text = remote
                    .url
                    .as_deref()
                    .or(remote.push_url.as_deref())
                    .unwrap_or("No URL configured")
                    .to_string();
                let fetch_remote = name.clone();
                let pull_remote = name.clone();
                let push_remote = name.clone();
                let remove_remote = name.clone();

                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("remote-item".into(), i as u64))
                        .v_flex()
                        .w_full()
                        .px_2()
                        .py_1()
                        .pl(px(16.))
                        .gap_1()
                        .when(kb_active, |el| el.bg(colors.ghost_element_hover).border_l_2().border_color(kb_accent))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .child(
                            div()
                                .h_flex()
                                .w_full()
                                .gap_2()
                                .items_center()
                                .overflow_hidden()
                                .child(
                                    rgitui_ui::Icon::new(IconName::ExternalLink)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Info),
                                )
                                .child(
                                    Label::new(display_name)
                                        .size(LabelSize::XSmall)
                                        .weight(gpui::FontWeight::SEMIBOLD)
                                        .truncate(),
                                )
                                .child(div().flex_1())
                                .child(
                                    IconButton::new(
                                        format!("remote-fetch-{}", i),
                                        IconName::Refresh,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Info)
                                    .tooltip("Fetch from remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemoteFetch(
                                                fetch_remote.clone(),
                                            ));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        format!("remote-pull-{}", i),
                                        IconName::ArrowDown,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Warning)
                                    .tooltip("Pull from remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemotePull(pull_remote.clone()));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        format!("remote-push-{}", i),
                                        IconName::ArrowUp,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Success)
                                    .tooltip("Push to remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemotePush(push_remote.clone()));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        format!("remote-remove-{}", i),
                                        IconName::Trash,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Deleted)
                                    .tooltip("Remove remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemoteRemove(
                                                remove_remote.clone(),
                                            ));
                                        },
                                    )),
                                ),
                        )
                        .child(
                            Label::new(SharedString::from(url_text))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                );
            }
        }

        // -- Remote Branches --
        let remote_branches: Vec<BranchInfo> = self
            .branches
            .iter()
            .filter(|b| b.is_remote)
            .cloned()
            .collect();

        let remote_expanded = self.is_expanded(SidebarSection::RemoteBranches);
        let icon = if remote_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-remote-branches")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::RemoteBranches, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Remote Branches")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(15.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", remote_branches.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if remote_expanded {
            for (i, branch) in remote_branches.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name: SharedString = branch.name.clone().into();
                let remote_branch_name = branch.name.clone();
                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("remote-branch".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(item_h))
                        .px_2()
                        .pl(px(16.))
                        .gap(px(4.))
                        .items_center()
                        .when(kb_active, |el| el.bg(colors.ghost_element_hover).border_l_2().border_color(kb_accent))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::BranchSelected(remote_branch_name.clone()));
                        }))
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .child(
                                    rgitui_ui::Icon::new(IconName::GitBranch)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                );
            }
        }

        // -- Tags --
        let tags_expanded = self.is_expanded(SidebarSection::Tags);
        let icon = if tags_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-tags")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::Tags, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::Tag)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Tags")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(15.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", self.tags.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if tags_expanded {
            if self.tags.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No tags")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for (i, tag) in self.tags.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name: SharedString = tag.name.clone().into();
                let tag_name = tag.name.clone();
                let tag_name_delete = tag.name.clone();
                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("tag-item".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(item_h))
                        .px_2()
                        .pl(px(16.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .when(kb_active, |el| el.bg(colors.ghost_element_hover).border_l_2().border_color(kb_accent))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::TagSelected(tag_name.clone()));
                        }))
                        .child(
                            rgitui_ui::Icon::new(IconName::Tag)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Warning),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                        )
                        .child(div().flex_1())
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("delete-tag".into(), i as u64),
                                IconName::Trash,
                            )
                            .size(ButtonSize::Compact)
                            .color(Color::Deleted)
                            .tooltip("Delete tag")
                            .on_click(cx.listener(
                                move |_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::TagDelete(
                                        tag_name_delete.clone(),
                                    ));
                                },
                            )),
                        ),
                );
            }
        }

        // -- Stashes --
        let stashes_expanded = self.is_expanded(SidebarSection::Stashes);
        let icon = if stashes_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-stashes")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::Stashes, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::Stash)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Stashes")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(15.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", self.stashes.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if stashes_expanded {
            if self.stashes.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No stashes")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for (i, stash) in self.stashes.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let msg: SharedString = stash.message.clone().into();
                let stash_index = stash.index;
                let stash_index_apply = stash.index;
                let stash_index_drop = stash.index;
                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("stash-item".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(item_h))
                        .px_2()
                        .pl(px(16.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .when(kb_active, |el| el.bg(colors.ghost_element_hover).border_l_2().border_color(kb_accent))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::StashSelected(stash_index));
                        }))
                        .child(
                            rgitui_ui::Icon::new(IconName::Stash)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(Label::new(msg).size(LabelSize::XSmall).truncate())
                        .child(div().flex_1())
                        .child(
                            div()
                                .h_flex()
                                .gap(px(2.))
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger(
                                            "apply-stash".into(),
                                            i as u64,
                                        ),
                                        IconName::Check,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Success)
                                    .tooltip("Apply stash")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::StashApply(
                                                stash_index_apply,
                                            ));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger(
                                            "drop-stash".into(),
                                            i as u64,
                                        ),
                                        IconName::Trash,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Deleted)
                                    .tooltip("Drop stash")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::StashDrop(
                                                stash_index_drop,
                                            ));
                                        },
                                    )),
                                ),
                        ),
                );
            }
        }

        // Separator between refs and file changes
        content = content.child(
            div()
                .w_full()
                .h(px(1.))
                .my(px(4.))
                .bg(colors.border),
        );

        // -- Staged Changes --
        let staged_expanded = self.is_expanded(SidebarSection::StagedChanges);
        let staged_count = self.staged.len();
        let has_staged = staged_count > 0;
        let staged_kind_counts = Self::file_kind_counts(&self.staged);

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;

        let mut staged_header = div()
                .id("section-staged")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::StagedChanges, cx);
                }))
                .child(
                    Disclosure::new("staged-disclosure", "Staged", staged_expanded),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::Check)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(if has_staged {
                            Color::Added
                        } else {
                            Color::Muted
                        }),
                );

        if has_staged {
            let kind_order = [
                FileChangeKind::Added,
                FileChangeKind::Modified,
                FileChangeKind::Deleted,
                FileChangeKind::Renamed,
                FileChangeKind::Copied,
                FileChangeKind::TypeChange,
                FileChangeKind::Untracked,
                FileChangeKind::Conflicted,
            ];
            let mut breakdown = div().h_flex().gap(px(3.));
            for kind in &kind_order {
                if let Some(&count) = staged_kind_counts.get(kind) {
                    if count > 0 {
                        let symbol = Self::file_change_symbol(*kind);
                        let color = Self::file_change_color(*kind);
                        let text: SharedString = format!("{}{}", symbol, count).into();
                        breakdown = breakdown.child(
                            Label::new(text)
                                .size(LabelSize::XSmall)
                                .color(color),
                        );
                    }
                }
            }
            staged_header = staged_header.child(breakdown);
        }

        staged_header = staged_header
                .child(div().flex_1())
                .when(has_staged, |el| {
                    el.child(
                        div().id("unstage-all-btn").child(
                            Button::new("unstage-all", "Unstage All")
                                .icon(IconName::Minus)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Subtle)
                                .color(Color::Muted)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::UnstageAll);
                                })),
                        ),
                    )
                })
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(if staged_count > 0 {
                            colors.ghost_element_selected
                        } else {
                            colors.ghost_element_hover
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", staged_count)))
                                .size(LabelSize::XSmall)
                                .color(if staged_count > 0 {
                                    Color::Added
                                } else {
                                    Color::Muted
                                }),
                        ),
                );
        content = content.child(staged_header);

        if staged_expanded {
            if self.staged.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No staged changes")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            } else {
                nav_idx += self.staged.len(); // Track file items in nav index
                let staged_files = self.staged.clone();
                let tree = Self::build_file_tree(&staged_files);
                let mut ctx = FileRowCtx { staged: true, indent: px(16.0), file_idx: 0, colors: &colors };
                let staged_body = div().id("staged-body").v_flex().w_full().flex_shrink_0();
                content = content.child(self.render_file_tree(
                    staged_body,
                    &tree,
                    ("staged", "", 0),
                    &mut ctx,
                    cx,
                ));
            }
        }

        // -- Unstaged Changes --
        let unstaged_expanded = self.is_expanded(SidebarSection::UnstagedChanges);
        let unstaged_count = self.unstaged.len();
        let has_unstaged = unstaged_count > 0;
        let unstaged_kind_counts = Self::file_kind_counts(&self.unstaged);

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;

        let mut unstaged_header = div()
                .id("section-unstaged")
                .h_flex()
                .w_full()
                .h(px(item_h))
                .px(px(8.))
                .gap(px(4.))
                .items_center()
                .bg(colors.toolbar_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .when(kb_active, |el| el.border_l_2().border_color(kb_accent))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::UnstagedChanges, cx);
                }))
                .child(
                    Disclosure::new("unstaged-disclosure", "Unstaged", unstaged_expanded),
                )
                .child(
                    rgitui_ui::Icon::new(IconName::Edit)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(if has_unstaged {
                            Color::Modified
                        } else {
                            Color::Muted
                        }),
                );

        if has_unstaged {
            let kind_order = [
                FileChangeKind::Added,
                FileChangeKind::Modified,
                FileChangeKind::Deleted,
                FileChangeKind::Renamed,
                FileChangeKind::Copied,
                FileChangeKind::TypeChange,
                FileChangeKind::Untracked,
                FileChangeKind::Conflicted,
            ];
            let mut breakdown = div().h_flex().gap(px(3.));
            for kind in &kind_order {
                if let Some(&count) = unstaged_kind_counts.get(kind) {
                    if count > 0 {
                        let symbol = Self::file_change_symbol(*kind);
                        let color = Self::file_change_color(*kind);
                        let text: SharedString = format!("{}{}", symbol, count).into();
                        breakdown = breakdown.child(
                            Label::new(text)
                                .size(LabelSize::XSmall)
                                .color(color),
                        );
                    }
                }
            }
            unstaged_header = unstaged_header.child(breakdown);
        }

        unstaged_header = unstaged_header
                .child(div().flex_1())
                .when(has_unstaged, |el| {
                    el.child(
                        div().id("stage-all-btn").child(
                            Button::new("stage-all", "Stage All")
                                .icon(IconName::Plus)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Subtle)
                                .color(Color::Muted)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::StageAll);
                                })),
                        ),
                    )
                })
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(if unstaged_count > 0 {
                            colors.ghost_element_selected
                        } else {
                            colors.ghost_element_hover
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", unstaged_count)))
                                .size(LabelSize::XSmall)
                                .color(if unstaged_count > 0 {
                                    Color::Modified
                                } else {
                                    Color::Muted
                                }),
                        ),
                );
        content = content.child(unstaged_header);

        if unstaged_expanded {
            if self.unstaged.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("Working tree clean")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            } else {
                nav_idx += self.unstaged.len(); // Track file items in nav index
                let unstaged_files = self.unstaged.clone();
                let tree = Self::build_file_tree(&unstaged_files);
                let mut ctx = FileRowCtx { staged: false, indent: px(16.0), file_idx: 0, colors: &colors };
                let unstaged_body = div().id("unstaged-body").v_flex().w_full().flex_shrink_0();
                content = content.child(self.render_file_tree(
                    unstaged_body,
                    &tree,
                    ("unstaged", "", 0),
                    &mut ctx,
                    cx,
                ));
            }
        }

        let _ = nav_idx; // Suppress unused warning
        panel.child(
            div()
                .id("sidebar-scroll")
                .v_flex()
                .w_full()
                .flex_1()
                .overflow_y_scroll()
                .child(content),
        )
    }
}
