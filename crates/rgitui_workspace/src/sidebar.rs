use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, KeyDownEvent,
    Render, SharedString, Window,
};
use rgitui_git::{
    BranchInfo, FileChangeKind, FileStatus, RemoteInfo, StashEntry, TagInfo, WorktreeInfo,
};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, DiffStat, Disclosure, IconButton, IconName, Label, LabelSize,
    TextInput, TextInputEvent,
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
    TagCheckout(String),
    TagDelete(String),
    StashSelected(usize),
    StashApply(usize),
    StashDrop(usize),
    WorktreeSelected(usize),
    WorktreeCreate,
    WorktreeRemove(usize),
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
    Worktrees,
    StagedChanges,
    UnstagedChanges,
}

#[derive(Default, Clone)]
struct FileTreeNode {
    file_indices: Vec<usize>,
    children: BTreeMap<String, FileTreeNode>,
    /// Precomputed total file count (self + all descendants), avoiding
    /// O(n*d) repeated traversal during every render.
    file_count: usize,
}

/// Represents a navigable item in the sidebar's flat list.
#[derive(Debug, Clone)]
enum SidebarItem {
    SectionHeader(SidebarSection),
    LocalBranch(usize),  // index into local branches
    Remote(usize),       // index into remotes
    RemoteBranch(usize), // index into remote branches
    Tag(usize),          // index into tags
    Stash(usize),        // index into stashes
    Worktree(usize),     // index into worktrees
    StagedFile(usize),   // index into staged files
    UnstagedFile(usize), // index into unstaged files
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
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    stashes: Vec<StashEntry>,
    worktrees: Vec<WorktreeInfo>,
    staged: Vec<FileStatus>,
    unstaged: Vec<FileStatus>,
    selected_file: Option<(String, bool)>,
    /// Currently selected stash index (persists after click for visual highlight).
    selected_stash: Option<usize>,
    /// Currently selected tag name (persists after click for visual highlight).
    selected_tag: Option<String>,
    /// Currently selected worktree index (persists after click for visual highlight).
    selected_worktree: Option<usize>,
    /// Tracks which directory groups are collapsed in the file change sections.
    /// Key is "staged:<dir>" or "unstaged:<dir>".
    collapsed_dirs: HashSet<String>,
    /// Current repository name displayed in the sidebar header.
    repo_name: String,
    /// Focus handle for keyboard navigation.
    focus_handle: FocusHandle,
    /// Index into the flat navigable items list for keyboard nav.
    keyboard_index: Option<usize>,
    /// Pre-partitioned local branches (rebuilt in update_branches).
    local_branches: Vec<BranchInfo>,
    /// Pre-partitioned remote branches (rebuilt in update_branches).
    remote_branches: Vec<BranchInfo>,
    /// Cached file tree for staged changes (rebuilt in update_status).
    cached_staged_tree: FileTreeNode,
    /// Cached file tree for unstaged changes (rebuilt in update_status).
    cached_unstaged_tree: FileTreeNode,
    /// Cached flat list of navigable items for keyboard navigation.
    cached_nav_items: Vec<SidebarItem>,
    /// Current branch filter text (case-insensitive substring).
    branch_filter: String,
    /// Editor entity backing the branch filter input.
    branch_filter_editor: Entity<TextInput>,
    /// Whether the branch filter input is currently visible/active.
    branch_filter_active: bool,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let expanded_sections = vec![
            SidebarSection::LocalBranches,
            SidebarSection::Remotes,
            SidebarSection::StagedChanges,
            SidebarSection::UnstagedChanges,
        ];
        let cached_nav_items = vec![
            SidebarItem::SectionHeader(SidebarSection::LocalBranches),
            SidebarItem::SectionHeader(SidebarSection::Remotes),
            SidebarItem::SectionHeader(SidebarSection::RemoteBranches),
            SidebarItem::SectionHeader(SidebarSection::Tags),
            SidebarItem::SectionHeader(SidebarSection::Stashes),
            SidebarItem::SectionHeader(SidebarSection::Worktrees),
            SidebarItem::SectionHeader(SidebarSection::StagedChanges),
            SidebarItem::SectionHeader(SidebarSection::UnstagedChanges),
        ];

        let branch_filter_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Filter branches…");
            ti
        });

        cx.subscribe(
            &branch_filter_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(text) = event {
                    this.branch_filter = text.to_string();
                    this.rebuild_nav_items();
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            expanded_sections,
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            staged: Vec::new(),
            unstaged: Vec::new(),
            selected_file: None,
            selected_stash: None,
            selected_tag: None,
            selected_worktree: None,
            collapsed_dirs: HashSet::new(),
            repo_name: String::new(),
            focus_handle: cx.focus_handle(),
            keyboard_index: None,
            local_branches: Vec::new(),
            remote_branches: Vec::new(),
            cached_staged_tree: FileTreeNode::default(),
            cached_unstaged_tree: FileTreeNode::default(),
            cached_nav_items,
            branch_filter: String::new(),
            branch_filter_editor,
            branch_filter_active: false,
        }
    }

    /// Returns the branch indices that pass the current filter.
    fn filtered_local_indices(&self) -> Vec<usize> {
        if self.branch_filter.is_empty() {
            return (0..self.local_branches.len()).collect();
        }
        let filter_lc = self.branch_filter.to_lowercase();
        self.local_branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name.to_lowercase().contains(&filter_lc))
            .map(|(i, _)| i)
            .collect()
    }

    /// Rebuild the cached navigable items list from current state.
    fn rebuild_nav_items(&mut self) {
        let is_expanded =
            |section: SidebarSection, expanded: &[SidebarSection]| expanded.contains(&section);

        let mut items = Vec::new();

        items.push(SidebarItem::SectionHeader(SidebarSection::LocalBranches));
        if is_expanded(SidebarSection::LocalBranches, &self.expanded_sections) {
            for i in self.filtered_local_indices() {
                items.push(SidebarItem::LocalBranch(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::Remotes));
        if is_expanded(SidebarSection::Remotes, &self.expanded_sections) {
            for i in 0..self.remotes.len() {
                items.push(SidebarItem::Remote(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::RemoteBranches));
        if is_expanded(SidebarSection::RemoteBranches, &self.expanded_sections) {
            for i in 0..self.remote_branches.len() {
                items.push(SidebarItem::RemoteBranch(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::Tags));
        if is_expanded(SidebarSection::Tags, &self.expanded_sections) {
            for i in 0..self.tags.len() {
                items.push(SidebarItem::Tag(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::Stashes));
        if is_expanded(SidebarSection::Stashes, &self.expanded_sections) {
            for i in 0..self.stashes.len() {
                items.push(SidebarItem::Stash(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::Worktrees));
        if is_expanded(SidebarSection::Worktrees, &self.expanded_sections) {
            for i in 0..self.worktrees.len() {
                items.push(SidebarItem::Worktree(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::StagedChanges));
        if is_expanded(SidebarSection::StagedChanges, &self.expanded_sections) {
            for i in 0..self.staged.len() {
                items.push(SidebarItem::StagedFile(i));
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::UnstagedChanges));
        if is_expanded(SidebarSection::UnstagedChanges, &self.expanded_sections) {
            for i in 0..self.unstaged.len() {
                items.push(SidebarItem::UnstagedFile(i));
            }
        }

        self.cached_nav_items = items;
    }

    /// Activate the currently selected keyboard item (Enter key).
    fn activate_keyboard_item(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.keyboard_index else {
            return;
        };
        let Some(item) = self.cached_nav_items.get(idx).cloned() else {
            return;
        };

        match item {
            SidebarItem::SectionHeader(section) => {
                self.toggle_section(section, cx);
            }
            SidebarItem::LocalBranch(i) => {
                if let Some(branch) = self.local_branches.get(i) {
                    cx.emit(SidebarEvent::BranchCheckout(branch.name.clone()));
                }
            }
            SidebarItem::Remote(i) => {
                if let Some(remote) = self.remotes.get(i) {
                    cx.emit(SidebarEvent::RemoteFetch(remote.name.clone()));
                }
            }
            SidebarItem::RemoteBranch(i) => {
                if let Some(branch) = self.remote_branches.get(i) {
                    cx.emit(SidebarEvent::BranchCheckout(branch.name.clone()));
                }
            }
            SidebarItem::Tag(i) => {
                if let Some(tag) = self.tags.get(i) {
                    cx.emit(SidebarEvent::TagSelected(tag.name.clone()));
                }
            }
            SidebarItem::Stash(i) => {
                cx.emit(SidebarEvent::StashSelected(i));
            }
            SidebarItem::Worktree(i) => {
                cx.emit(SidebarEvent::WorktreeSelected(i));
            }
            SidebarItem::StagedFile(i) => {
                if let Some(file) = self.staged.get(i) {
                    let path = file.path.display().to_string();
                    self.selected_file = Some((path.clone(), true));
                    cx.emit(SidebarEvent::FileSelected { path, staged: true });
                    cx.notify();
                }
            }
            SidebarItem::UnstagedFile(i) => {
                if let Some(file) = self.unstaged.get(i) {
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

        if self.cached_nav_items.is_empty() {
            return;
        }

        let wants_filter = (key == "/" && !ctrl) || (key == "f" && ctrl);
        if wants_filter && !self.branch_filter_active {
            self.branch_filter_active = true;
            self.branch_filter_editor.update(cx, |editor, cx| {
                editor.focus(_window, cx);
            });
            cx.notify();
            cx.stop_propagation();
            return;
        }

        // Block Ctrl+F (graph search) when branch filter is active
        // so Ctrl+F re-focuses the filter input instead.
        if ctrl && self.branch_filter_active {
            cx.stop_propagation();
            return;
        }

        if ctrl {
            return;
        }

        match key {
            "escape" => {
                if self.branch_filter_active || !self.branch_filter.is_empty() {
                    self.branch_filter_active = false;
                    self.branch_filter.clear();
                    self.branch_filter_editor.update(cx, |editor, cx| {
                        editor.clear(cx);
                    });
                    self.rebuild_nav_items();
                    cx.notify();
                    cx.stop_propagation();
                }
            }
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
                let max = self.cached_nav_items.len().saturating_sub(1);
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
                self.keyboard_index = Some(self.cached_nav_items.len().saturating_sub(1));
                cx.notify();
            }
            "s" => {
                if let Some(idx) = self.keyboard_index {
                    if let Some(item) = self.cached_nav_items.get(idx).cloned() {
                        match item {
                            SidebarItem::StagedFile(i) => {
                                if let Some(file) = self.staged.get(i) {
                                    cx.emit(SidebarEvent::UnstageFile(
                                        file.path.display().to_string(),
                                    ));
                                }
                            }
                            SidebarItem::UnstagedFile(i) => {
                                if let Some(file) = self.unstaged.get(i) {
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
                if let Some(idx) = self.keyboard_index {
                    if let Some(item) = self.cached_nav_items.get(idx).cloned() {
                        match item {
                            SidebarItem::Tag(i) => {
                                if let Some(tag) = self.tags.get(i) {
                                    cx.emit(SidebarEvent::TagDelete(tag.name.clone()));
                                }
                            }
                            SidebarItem::Stash(i) => {
                                cx.emit(SidebarEvent::StashDrop(i));
                            }
                            SidebarItem::LocalBranch(i) => {
                                if let Some(branch) = self.local_branches.get(i) {
                                    if !branch.is_head {
                                        cx.emit(SidebarEvent::BranchDelete(branch.name.clone()));
                                    }
                                }
                            }
                            SidebarItem::UnstagedFile(i) => {
                                if let Some(file) = self.unstaged.get(i) {
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
        branches.sort_by(|a, b| {
            if a.is_remote != b.is_remote {
                return a.is_remote.cmp(&b.is_remote);
            }
            if a.is_head != b.is_head {
                return b.is_head.cmp(&a.is_head);
            }
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        });
        let (local, remote): (Vec<_>, Vec<_>) = branches.into_iter().partition(|b| !b.is_remote);
        if self.local_branches == local && self.remote_branches == remote {
            return;
        }
        self.local_branches = local;
        self.remote_branches = remote;
        self.rebuild_nav_items();
        cx.notify();
    }

    pub fn update_tags(&mut self, mut tags: Vec<TagInfo>, cx: &mut Context<Self>) {
        tags.sort_by_key(|a| a.name.to_lowercase());
        if self.tags == tags {
            return;
        }
        self.tags = tags;
        self.rebuild_nav_items();
        cx.notify();
    }

    pub fn update_remotes(&mut self, remotes: Vec<RemoteInfo>, cx: &mut Context<Self>) {
        if self.remotes == remotes {
            return;
        }
        self.remotes = remotes;
        self.rebuild_nav_items();
        cx.notify();
    }

    pub fn update_stashes(&mut self, stashes: Vec<StashEntry>, cx: &mut Context<Self>) {
        if self.stashes == stashes {
            return;
        }
        self.stashes = stashes;
        self.selected_stash = None;
        self.rebuild_nav_items();
        cx.notify();
    }

    pub fn update_worktrees(&mut self, worktrees: Vec<WorktreeInfo>, cx: &mut Context<Self>) {
        if self.worktrees == worktrees {
            return;
        }
        self.worktrees = worktrees;
        self.selected_worktree = None;
        self.rebuild_nav_items();
        cx.notify();
    }

    pub fn update_status(
        &mut self,
        staged: Vec<FileStatus>,
        unstaged: Vec<FileStatus>,
        cx: &mut Context<Self>,
    ) {
        if self.staged == staged && self.unstaged == unstaged {
            return;
        }
        self.cached_staged_tree = Self::build_file_tree(&staged);
        self.cached_unstaged_tree = Self::build_file_tree(&unstaged);
        self.staged = staged;
        self.unstaged = unstaged;
        self.rebuild_nav_items();
        cx.notify();
    }

    /// Expand the Local Branches section and focus the sidebar for branch switching.
    pub fn ensure_branches_visible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .expanded_sections
            .contains(&SidebarSection::LocalBranches)
        {
            self.expanded_sections.push(SidebarSection::LocalBranches);
            self.rebuild_nav_items();
        }
        self.focus_handle.focus(window, cx);
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
        self.rebuild_nav_items();
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

    fn build_file_tree(files: &[FileStatus]) -> FileTreeNode {
        let mut root = FileTreeNode::default();
        for (idx, file) in files.iter().enumerate() {
            let mut node = &mut root;
            if let Some(parent) = file.path.parent() {
                for component in parent.iter().filter_map(|part| part.to_str()) {
                    if component.is_empty() {
                        continue;
                    }
                    node = node.children.entry(component.to_string()).or_default();
                }
            }
            node.file_indices.push(idx);
        }
        Self::sort_file_tree(&mut root, files);
        Self::compute_file_counts(&mut root);
        root
    }

    /// Recursively compute `file_count` for each node (post-order).
    fn compute_file_counts(node: &mut FileTreeNode) {
        for child in node.children.values_mut() {
            Self::compute_file_counts(child);
        }
        node.file_count =
            node.file_indices.len() + node.children.values().map(|c| c.file_count).sum::<usize>();
    }

    fn sort_file_tree(node: &mut FileTreeNode, files: &[FileStatus]) {
        node.file_indices.sort_by(|&a, &b| {
            let a_name = files[a]
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let b_name = files[b]
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            a_name.to_lowercase().cmp(&b_name.to_lowercase())
        });
        for child in node.children.values_mut() {
            Self::sort_file_tree(child, files);
        }
    }

    #[allow(dead_code)]
    fn file_tree_file_count(node: &FileTreeNode) -> usize {
        node.file_indices.len()
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
        let item_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(24.0);
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
        let file_path: SharedString = file.path.display().to_string().into();
        let is_selected = self
            .selected_file
            .as_ref()
            .is_some_and(|(p, s)| p.as_str() == file_path.as_ref() && *s == staged);
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
            .on_click({
                let file_path = file_path.clone();
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_file = Some((file_path.to_string(), staged));
                    cx.emit(SidebarEvent::FileSelected {
                        path: file_path.to_string(),
                        staged,
                    });
                    cx.notify();
                })
            })
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
                        .id(ElementId::NamedInteger(
                            if staged {
                                "unstage-action".into()
                            } else {
                                "stage-action".into()
                            },
                            i as u64,
                        ))
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
                        .on_click({
                            let file_path = file_path.clone();
                            cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                cx.emit(if staged {
                                    SidebarEvent::UnstageFile(file_path.to_string())
                                } else {
                                    SidebarEvent::StageFile(file_path.to_string())
                                });
                            })
                        })
                        .child(if staged { "\u{2212}" } else { "+" }),
                ),
        );

        if !staged {
            let ghost_hover2 = colors.ghost_element_hover;
            row = row.child(
                div()
                    .id(ElementId::NamedInteger("discard-action".into(), i as u64))
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
                        cx.emit(SidebarEvent::DiscardFile(file_path.to_string()));
                    }))
                    .child("\u{00d7}"),
            );
        }

        body.child(row)
    }

    fn render_file_tree(
        &self,
        mut body: gpui::Stateful<gpui::Div>,
        node: &FileTreeNode,
        files: &[FileStatus],
        tree_ctx: (&str, &str, usize),
        ctx: &mut FileRowCtx<'_>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let item_h = cx
            .global::<SettingsState>()
            .settings()
            .compactness
            .spacing(24.0);
        let (prefix, parent_path, depth) = tree_ctx;
        let file_indent = px(if depth == 0 {
            16.0
        } else {
            16.0 + depth as f32 * 14.0
        });

        for &file_idx in &node.file_indices {
            ctx.indent = file_indent;
            body = self.render_file_row(body, &files[file_idx], ctx, cx);
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

            while display_node.file_indices.is_empty() && display_node.children.len() == 1 {
                let Some((next_name, next_child)) = display_node.children.iter().next() else {
                    break;
                };
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
            let file_count = display_node.file_count;

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
                    files,
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
        let local_branch_count = self.local_branches.len();
        let branch_filter = self.branch_filter.clone();
        let filtered_count = if branch_filter.is_empty() {
            local_branch_count
        } else {
            self.filtered_local_indices().len()
        };

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
                            Label::new(if branch_filter.is_empty() {
                                SharedString::from(format!("{}", local_branch_count))
                            } else {
                                SharedString::from(format!(
                                    "{}/{}",
                                    filtered_count, local_branch_count
                                ))
                            })
                            .size(LabelSize::XSmall)
                            .color(if !branch_filter.is_empty() {
                                Color::Accent
                            } else {
                                Color::Muted
                            }),
                        ),
                ),
        );

        // Branch filter input (shown when active or filter is non-empty)
        let branch_filter_active = self.branch_filter_active;
        if branch_filter_active || !branch_filter.is_empty() {
            content = content.child(
                div()
                    .id("branch-filter-row")
                    .h_flex()
                    .w_full()
                    .h(px(24.))
                    .px(px(8.))
                    .gap(px(4.))
                    .items_center()
                    .bg(colors.editor_background)
                    .border_b_1()
                    .border_color(colors.border_focused)
                    .child(
                        rgitui_ui::Icon::new(IconName::Search)
                            .size(rgitui_ui::IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1().child(self.branch_filter_editor.clone())),
            );
        }

        if local_expanded {
            let visible_branch_indices = self.filtered_local_indices();
            if visible_branch_indices.is_empty() {
                let empty_msg = if !branch_filter.is_empty() {
                    "No matching branches"
                } else {
                    "No local branches"
                };
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new(empty_msg)
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for i in visible_branch_indices {
                let branch = &self.local_branches[i];
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name: SharedString = branch.name.clone().into();
                let branch_name: SharedString = name.clone();
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
                    .when(kb_active, |el| {
                        el.bg(colors.ghost_element_hover)
                            .border_l_2()
                            .border_color(kb_accent)
                    })
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .cursor_pointer()
                    .on_click({
                        let branch_name = branch_name.clone();
                        cx.listener(move |_this, event: &ClickEvent, _, cx| {
                            if event.click_count() >= 2 {
                                cx.emit(SidebarEvent::BranchCheckout(branch_name.to_string()));
                            } else {
                                cx.emit(SidebarEvent::BranchSelected(branch_name.to_string()));
                            }
                        })
                    });

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

                // Non-HEAD branches get action buttons: Checkout, Merge, Rename, Delete
                if !is_head {
                    let bn_checkout = branch_name.clone();
                    let bn_merge = branch_name.clone();
                    let bn_rename = branch_name.clone();
                    let bn_delete = branch_name.clone();
                    item = item.child(
                        div()
                            .ml_auto()
                            .h_flex()
                            .gap(px(2.))
                            .child(
                                IconButton::new(
                                    ElementId::NamedInteger("checkout-branch".into(), i as u64),
                                    IconName::Check,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Success)
                                .tooltip("Checkout branch")
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::BranchCheckout(
                                            bn_checkout.to_string(),
                                        ));
                                    },
                                )),
                            )
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
                                        cx.emit(SidebarEvent::MergeBranch(bn_merge.to_string()));
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
                                        cx.emit(SidebarEvent::BranchRename(bn_rename.to_string()));
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
                                        cx.emit(SidebarEvent::BranchDelete(bn_delete.to_string()));
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
                let remote_name: SharedString = remote.name.clone().into();
                let display_name: SharedString = remote_name.clone();
                let url_text = remote
                    .url
                    .as_deref()
                    .or(remote.push_url.as_deref())
                    .unwrap_or("No URL configured")
                    .to_string();
                let fetch_remote = remote_name.clone();
                let pull_remote = remote_name.clone();
                let push_remote = remote_name.clone();
                let remove_remote = remote_name;

                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("remote-item".into(), i as u64))
                        .v_flex()
                        .w_full()
                        .px_2()
                        .py_1()
                        .pl(px(16.))
                        .gap_1()
                        .when(kb_active, |el| {
                            el.bg(colors.ghost_element_hover)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
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
                                        ElementId::NamedInteger("remote-fetch".into(), i as u64),
                                        IconName::Refresh,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Info)
                                    .tooltip("Fetch from remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemoteFetch(
                                                fetch_remote.to_string(),
                                            ));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger("remote-pull".into(), i as u64),
                                        IconName::ArrowDown,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Warning)
                                    .tooltip("Pull from remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemotePull(
                                                pull_remote.to_string(),
                                            ));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger("remote-push".into(), i as u64),
                                        IconName::ArrowUp,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Success)
                                    .tooltip("Push to remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemotePush(
                                                push_remote.to_string(),
                                            ));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger("remote-remove".into(), i as u64),
                                        IconName::Trash,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Deleted)
                                    .tooltip("Remove remote")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::RemoteRemove(
                                                remove_remote.to_string(),
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
        let remote_branch_count = self.remote_branches.len();

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
                            Label::new(SharedString::from(format!("{}", remote_branch_count)))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if remote_expanded {
            for (i, branch) in self.remote_branches.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let name: SharedString = branch.name.clone().into();
                let remote_branch_name = name.clone();
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
                        .when(kb_active, |el| {
                            el.bg(colors.ghost_element_hover)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click({
                            let remote_branch_name = remote_branch_name.clone();
                            cx.listener(move |_this, event: &ClickEvent, _, cx| {
                                if event.click_count() >= 2 {
                                    cx.emit(SidebarEvent::BranchCheckout(
                                        remote_branch_name.to_string(),
                                    ));
                                } else {
                                    cx.emit(SidebarEvent::BranchSelected(
                                        remote_branch_name.to_string(),
                                    ));
                                }
                            })
                        })
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
                let is_selected = self.selected_tag.as_ref().is_some_and(|s| s == &tag.name);
                let name: SharedString = tag.name.clone().into();
                let tag_select = name.clone();
                let tag_delete = name.clone();
                let tag_checkout = name.clone();
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
                        .when(is_selected, |el| {
                            el.bg(colors.ghost_element_selected)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .when(kb_active && !is_selected, |el| {
                            el.bg(colors.ghost_element_hover)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_tag = Some(tag_select.to_string());
                            cx.emit(SidebarEvent::TagSelected(tag_select.to_string()));
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
                                ElementId::NamedInteger("checkout-tag".into(), i as u64),
                                IconName::ArrowDown,
                            )
                            .size(ButtonSize::Compact)
                            .color(Color::Muted)
                            .tooltip("Checkout tag")
                            .on_click(cx.listener(
                                move |_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::TagCheckout(tag_checkout.to_string()));
                                },
                            )),
                        )
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
                                    cx.emit(SidebarEvent::TagDelete(tag_delete.to_string()));
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
                let is_selected = self.selected_stash == Some(i);
                let msg: SharedString = stash.message.clone().into();
                let stash_index = stash.index;
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
                        .when(is_selected, |el| {
                            el.bg(colors.ghost_element_selected)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .when(kb_active && !is_selected, |el| {
                            el.bg(colors.ghost_element_hover)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_stash = Some(stash_index);
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
                                        ElementId::NamedInteger("apply-stash".into(), i as u64),
                                        IconName::Check,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Success)
                                    .tooltip("Apply stash")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::StashApply(stash_index));
                                        },
                                    )),
                                )
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger("drop-stash".into(), i as u64),
                                        IconName::Trash,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Deleted)
                                    .tooltip("Drop stash")
                                    .on_click(cx.listener(
                                        move |_this, _: &ClickEvent, _, cx| {
                                            cx.emit(SidebarEvent::StashDrop(stash_index));
                                        },
                                    )),
                                ),
                        ),
                );
            }
        }

        // -- Worktrees --
        let worktrees_expanded = self.is_expanded(SidebarSection::Worktrees);
        let icon = if worktrees_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        content = content.child(
            div()
                .id("section-worktrees")
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
                    this.toggle_section(SidebarSection::Worktrees, cx);
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
                    Label::new("Worktrees")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div().id("new-worktree-btn").child(
                        Button::new("new-worktree", "New Worktree")
                            .icon(IconName::Plus)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .color(Color::Muted)
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.emit(SidebarEvent::WorktreeCreate);
                            })),
                    ),
                )
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
                            Label::new(SharedString::from(format!("{}", self.worktrees.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if worktrees_expanded {
            if self.worktrees.is_empty() {
                content = content.child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(16.))
                        .items_center()
                        .child(
                            Label::new("No worktrees")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                );
            }
            for (i, worktree) in self.worktrees.iter().enumerate() {
                let kb_active = keyboard_index == Some(nav_idx);
                nav_idx += 1;
                let is_selected = self.selected_worktree == Some(i);
                let worktree_index = i;
                let name: SharedString = worktree.name.clone().into();
                let path: SharedString = worktree.path.display().to_string().into();
                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("worktree-item".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(item_h))
                        .px_2()
                        .pl(px(16.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .when(is_selected, |el| {
                            el.bg(colors.ghost_element_selected)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .when(kb_active && !is_selected, |el| {
                            el.bg(colors.ghost_element_hover)
                                .border_l_2()
                                .border_color(kb_accent)
                        })
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_worktree = Some(worktree_index);
                            cx.emit(SidebarEvent::WorktreeSelected(worktree_index));
                        }))
                        .child(
                            rgitui_ui::Icon::new(IconName::GitBranch)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(if worktree.is_current {
                                    Color::Info
                                } else {
                                    Color::Muted
                                }),
                        )
                        .child(Label::new(name.clone()).size(LabelSize::XSmall).color(
                            if worktree.is_current {
                                Color::Info
                            } else {
                                Color::Default
                            },
                        ))
                        .when(worktree.is_current, |el| {
                            el.child(
                                Label::new("(current)")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                        })
                        .when(worktree.is_locked, |el| {
                            el.child(
                                rgitui_ui::Icon::new(IconName::Lock)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(Color::Warning),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            Label::new(path)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                );
            }
        }

        // Separator between refs and file changes
        content = content.child(div().w_full().h(px(1.)).my(px(4.)).bg(colors.border));

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
            .child(Disclosure::new(
                "staged-disclosure",
                "Staged",
                staged_expanded,
            ))
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
                        breakdown =
                            breakdown.child(Label::new(text).size(LabelSize::XSmall).color(color));
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
                nav_idx += self.staged.len();
                let tree = self.cached_staged_tree.clone();
                let mut ctx = FileRowCtx {
                    staged: true,
                    indent: px(16.0),
                    file_idx: 0,
                    colors: &colors,
                };
                let staged_body = div().id("staged-body").v_flex().w_full().flex_shrink_0();
                content = content.child(self.render_file_tree(
                    staged_body,
                    &tree,
                    &self.staged,
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
            .child(Disclosure::new(
                "unstaged-disclosure",
                "Unstaged",
                unstaged_expanded,
            ))
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
                        breakdown =
                            breakdown.child(Label::new(text).size(LabelSize::XSmall).color(color));
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
                let tree = self.cached_unstaged_tree.clone();
                let mut ctx = FileRowCtx {
                    staged: false,
                    indent: px(16.0),
                    file_idx: 0,
                    colors: &colors,
                };
                let unstaged_body = div().id("unstaged-body").v_flex().w_full().flex_shrink_0();
                content = content.child(self.render_file_tree(
                    unstaged_body,
                    &tree,
                    &self.unstaged,
                    ("unstaged", "", 0),
                    &mut ctx,
                    cx,
                ));
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use rgitui_git::{FileChangeKind, FileStatus};
    use std::path::PathBuf;

    // --- file_change_symbol tests ---

    #[test]
    fn test_file_change_symbol_all_kinds() {
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Added), "+");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Modified), "~");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Deleted), "-");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Renamed), "R");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Copied), "C");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::TypeChange), "T");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Untracked), "?");
        assert_eq!(Sidebar::file_change_symbol(FileChangeKind::Conflicted), "!");
    }

    // --- file_kind_counts tests ---

    #[test]
    fn test_file_kind_counts_empty() {
        let files: Vec<FileStatus> = vec![];
        let counts = Sidebar::file_kind_counts(&files);
        assert!(counts.is_empty());
    }

    #[test]
    fn test_file_kind_counts_single_kind() {
        let files = vec![
            make_file("src/main.rs", FileChangeKind::Modified),
            make_file("src/lib.rs", FileChangeKind::Modified),
        ];
        let counts = Sidebar::file_kind_counts(&files);
        assert_eq!(*counts.get(&FileChangeKind::Modified).unwrap(), 2);
        assert_eq!(counts.len(), 1);
    }

    #[test]
    fn test_file_kind_counts_multiple_kinds() {
        let files = vec![
            make_file("src/main.rs", FileChangeKind::Added),
            make_file("src/lib.rs", FileChangeKind::Modified),
            make_file("README.md", FileChangeKind::Added),
            make_file("Cargo.toml", FileChangeKind::Deleted),
        ];
        let counts = Sidebar::file_kind_counts(&files);
        assert_eq!(*counts.get(&FileChangeKind::Added).unwrap(), 2);
        assert_eq!(*counts.get(&FileChangeKind::Modified).unwrap(), 1);
        assert_eq!(*counts.get(&FileChangeKind::Deleted).unwrap(), 1);
        assert_eq!(counts.len(), 3);
    }

    #[test]
    fn test_file_kind_counts_all_kinds() {
        let files = vec![
            make_file("added.rs", FileChangeKind::Added),
            make_file("modified.rs", FileChangeKind::Modified),
            make_file("deleted.rs", FileChangeKind::Deleted),
            make_file("renamed.rs", FileChangeKind::Renamed),
            make_file("copied.rs", FileChangeKind::Copied),
            make_file("typechange.rs", FileChangeKind::TypeChange),
            make_file("untracked.rs", FileChangeKind::Untracked),
            make_file("conflicted.rs", FileChangeKind::Conflicted),
        ];
        let counts = Sidebar::file_kind_counts(&files);
        for kind in [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
            FileChangeKind::Copied,
            FileChangeKind::TypeChange,
            FileChangeKind::Untracked,
            FileChangeKind::Conflicted,
        ] {
            assert_eq!(*counts.get(&kind).unwrap(), 1, "kind {kind:?}");
        }
        assert_eq!(counts.len(), 8);
    }

    // --- build_file_tree tests ---

    #[test]
    fn test_build_file_tree_empty() {
        let files: Vec<FileStatus> = vec![];
        let tree = Sidebar::build_file_tree(&files);
        assert!(tree.file_indices.is_empty());
        assert!(tree.children.is_empty());
    }

    #[test]
    fn test_build_file_tree_flat_files() {
        // Files with no directory component go directly to root
        let files = vec![
            make_file("a.rs", FileChangeKind::Modified),
            make_file("b.rs", FileChangeKind::Added),
        ];
        let tree = Sidebar::build_file_tree(&files);
        // Root-level files are indexed at root with empty parent path
        assert_eq!(tree.file_indices.len(), 2);
    }

    #[test]
    fn test_build_file_tree_nested_files() {
        let files = vec![
            make_file("src/main.rs", FileChangeKind::Modified),
            make_file("src/lib.rs", FileChangeKind::Added),
            make_file("src/foo/bar.rs", FileChangeKind::Deleted),
            make_file("tests/integration_test.rs", FileChangeKind::Added),
        ];
        let tree = Sidebar::build_file_tree(&files);
        assert!(tree.children.contains_key("src"));
        assert!(tree.children.contains_key("tests"));
        // src/foo/bar.rs: parent is src/foo, so "foo" is a child of "src" with bar.rs as a file
        let foo_node = tree
            .children
            .get("src")
            .unwrap()
            .children
            .get("foo")
            .unwrap();
        assert_eq!(foo_node.file_indices.len(), 1); // bar.rs
                                                    // tests/integration_test.rs: parent is tests
        assert_eq!(tree.children.get("tests").unwrap().file_indices.len(), 1); // integration_test.rs
    }

    #[test]
    fn test_build_file_tree_deep_nesting() {
        let files = vec![make_file("a/b/c/d/e.rs", FileChangeKind::Modified)];
        let tree = Sidebar::build_file_tree(&files);
        let node = tree
            .children
            .get("a")
            .unwrap()
            .children
            .get("b")
            .unwrap()
            .children
            .get("c")
            .unwrap()
            .children
            .get("d")
            .unwrap();
        assert_eq!(node.file_indices.len(), 1);
    }

    // --- file_tree_file_count tests ---

    #[test]
    fn test_file_tree_file_count_empty() {
        let node = FileTreeNode::default();
        assert_eq!(Sidebar::file_tree_file_count(&node), 0);
    }

    #[test]
    fn test_file_tree_file_count_only_root_files() {
        let files = vec![
            make_file("a.rs", FileChangeKind::Added),
            make_file("b.rs", FileChangeKind::Modified),
        ];
        let tree = Sidebar::build_file_tree(&files);
        assert_eq!(Sidebar::file_tree_file_count(&tree), 2);
    }

    #[test]
    fn test_file_tree_file_count_nested() {
        let files = vec![
            make_file("src/main.rs", FileChangeKind::Modified),
            make_file("src/lib.rs", FileChangeKind::Added),
            make_file("tests/test.rs", FileChangeKind::Modified),
        ];
        let tree = Sidebar::build_file_tree(&files);
        assert_eq!(Sidebar::file_tree_file_count(&tree), 3);
    }

    #[test]
    fn test_file_tree_file_count_deep() {
        let files = vec![
            make_file("a/b/c.rs", FileChangeKind::Added),
            make_file("a/d.rs", FileChangeKind::Modified),
        ];
        let tree = Sidebar::build_file_tree(&files);
        // a/b/c.rs and a/d.rs = 2 files total
        assert_eq!(Sidebar::file_tree_file_count(&tree), 2);
    }

    // --- filtered_local_indices tests ---
    // Requires Sidebar struct with nav_items set up — testing the pure helper logic.

    #[test]
    fn test_file_tree_node_default() {
        let node = FileTreeNode::default();
        assert!(node.file_indices.is_empty());
        assert!(node.children.is_empty());
    }

    // --- Helper ---
    fn make_file(path: &str, kind: FileChangeKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind,
            old_path: None,
            additions: 0,
            deletions: 0,
        }
    }
}
