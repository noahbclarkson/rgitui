use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    KeyDownEvent, ListSizingBehavior, Render, SharedString, WeakEntity, Window,
};
use rgitui_git::{
    BranchInfo, FileChangeKind, FileStatus, RemoteInfo, StashEntry, TagInfo, WorktreeInfo,
};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, IconButton, IconName, Label, LabelSize, TextInput,
    TextInputEvent, Tooltip,
};

/// Events from the sidebar.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    BranchSelected(String),
    BranchCheckout(String),
    BranchCreate,
    BranchDelete(String),
    BranchRename(String),
    BranchCopyName(String),
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
    StashPop(usize),
    StashBranch(usize),
    WorktreeSelected(usize),
    WorktreeCreate,
    WorktreeRemove(usize),
    FileSelected { path: String, staged: bool },
    StageFile(String),
    UnstageFile(String),
    StageAll,
    UnstageAll,
    DiscardFile(String),
    AcceptConflictOurs(String),
    AcceptConflictTheirs(String),
    ConflictFileSelected(String),
    OpenRepo,
    ToggleDir(String), // dir_key: "staged:path" or "unstaged:path"
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

/// A flattened file tree item for virtualized rendering via `uniform_list`.
/// Avoids O(n) DOM construction every frame by only building visible rows.
#[derive(Debug, Clone)]
pub enum FlatFileItem {
    File {
        /// Index into the staged/unstaged `FileStatus` array.
        file_idx: usize,
        /// Indent in pixels (computed from tree depth).
        indent: gpui::Pixels,
    },
    Dir {
        /// Full key into `collapsed_dirs`, e.g. "staged:src" or "unstaged:src/lib".
        dir_key: String,
        /// Display label for the directory row (always ends with /).
        label: SharedString,
        /// Number of descendant files (for the badge).
        file_count: usize,
        /// Whether this directory is currently collapsed.
        collapsed: bool,
        /// Indent in pixels.
        indent: gpui::Pixels,
    },
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
    /// Flattened staged file tree for virtualized `uniform_list` rendering.
    flattened_staged: Vec<FlatFileItem>,
    /// Flattened unstaged file tree for virtualized `uniform_list` rendering.
    flattened_unstaged: Vec<FlatFileItem>,
    /// Flattened local branches for virtualized `uniform_list` rendering.
    flattened_local_branches: Vec<usize>, // indices into self.local_branches
    /// Flattened remote branches for virtualized `uniform_list` rendering.
    flattened_remote_branches: Vec<usize>, // indices into self.remote_branches
    /// Flattened tags for virtualized `uniform_list` rendering.
    flattened_tags: Vec<usize>, // indices into self.tags
    /// Flattened stashes for virtualized `uniform_list` rendering.
    flattened_stashes: Vec<usize>, // indices into self.stashes
    /// Flattened worktrees for virtualized `uniform_list` rendering.
    flattened_worktrees: Vec<usize>, // indices into self.worktrees
    /// Flattened remotes for virtualized `uniform_list` rendering.
    flattened_remotes: Vec<usize>, // indices into self.remotes
    /// Cached flat list of navigable items for keyboard navigation.
    cached_nav_items: Vec<SidebarItem>,
    /// Current branch filter text (case-insensitive substring).
    branch_filter: String,
    /// Editor entity backing the branch filter input.
    branch_filter_editor: Entity<TextInput>,
    /// Whether the branch filter input is currently visible/active.
    branch_filter_active: bool,
    /// Whether "My Branches" filter is active (show only branches authored by current user).
    my_branches_active: bool,
    /// Current user email for "My Branches" filtering.
    current_user_email: Option<String>,
}

/// Pure filtering function: returns indices into `branches` whose names contain
/// `filter` (case-insensitive substring) and, if `current_user_email` is `Some`
/// and `my_branches` is true, also have a matching author email.
pub(crate) fn filter_local_branch_indices(
    branches: &[BranchInfo],
    filter: &str,
    my_branches: bool,
    current_user_email: Option<&str>,
) -> Vec<usize> {
    let filter_lc = filter.to_lowercase();
    branches
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            let name_match = filter.is_empty() || b.name.to_lowercase().contains(&filter_lc);
            let author_match = if !my_branches {
                true
            } else {
                match (&b.author_email, current_user_email) {
                    (Some(ae), Some(ue)) => ae.eq_ignore_ascii_case(ue),
                    _ => false,
                }
            };
            name_match && author_match
        })
        .map(|(i, _)| i)
        .collect()
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
            ti.set_placeholder("Filter branches...");
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
            flattened_staged: Vec::new(),
            flattened_unstaged: Vec::new(),
            flattened_local_branches: Vec::new(),
            flattened_remote_branches: Vec::new(),
            flattened_tags: Vec::new(),
            flattened_stashes: Vec::new(),
            flattened_worktrees: Vec::new(),
            flattened_remotes: Vec::new(),
            cached_nav_items,
            branch_filter: String::new(),
            branch_filter_editor,
            branch_filter_active: false,
            my_branches_active: false,
            current_user_email: None,
        }
    }

    /// Returns the branch indices that pass the current branch filter and "My Branches" toggle.
    fn filtered_local_indices(&self, current_user_email: Option<&str>) -> Vec<usize> {
        filter_local_branch_indices(
            &self.local_branches,
            &self.branch_filter,
            self.my_branches_active,
            current_user_email,
        )
    }

    /// Rebuild the cached navigable items list from current state.
    fn rebuild_nav_items(&mut self) {
        let is_expanded =
            |section: SidebarSection, expanded: &[SidebarSection]| expanded.contains(&section);

        let mut items = Vec::new();

        items.push(SidebarItem::SectionHeader(SidebarSection::LocalBranches));
        if is_expanded(SidebarSection::LocalBranches, &self.expanded_sections) {
            for i in self.filtered_local_indices(self.current_user_email.as_deref()) {
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

    /// Set the current user email for "My Branches" filtering.
    pub fn set_current_user_email(&mut self, email: Option<&str>, cx: &mut Context<Self>) {
        let same = match (&self.current_user_email, email) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if same {
            return;
        }
        self.current_user_email = email.map(String::from);
        self.rebuild_flattened_branches();
        self.rebuild_nav_items();
        cx.notify();
    }

    /// Rebuild flattened local branches (used after filter changes).
    fn rebuild_flattened_branches(&mut self) {
        self.flattened_local_branches.clear();
        let visible_branch_indices =
            self.filtered_local_indices(self.current_user_email.as_deref());
        self.flattened_local_branches.extend(visible_branch_indices);
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

        // Rebuild flattened local branches for virtualized rendering.
        self.flattened_local_branches.clear();
        let visible_branch_indices =
            self.filtered_local_indices(self.current_user_email.as_deref());
        self.flattened_local_branches.extend(visible_branch_indices);

        // Rebuild flattened remote branches for virtualized rendering.
        self.flattened_remote_branches.clear();
        self.flattened_remote_branches
            .extend(0..self.remote_branches.len());

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

        // Rebuild flattened tags for virtualized rendering.
        self.flattened_tags.clear();
        self.flattened_tags.extend(0..self.tags.len());

        cx.notify();
    }

    pub fn update_remotes(&mut self, remotes: Vec<RemoteInfo>, cx: &mut Context<Self>) {
        if self.remotes == remotes {
            return;
        }
        self.remotes = remotes;
        self.rebuild_nav_items();

        // Rebuild flattened remotes for virtualized rendering.
        self.flattened_remotes.clear();
        self.flattened_remotes.extend(0..self.remotes.len());

        cx.notify();
    }

    pub fn update_stashes(&mut self, stashes: Vec<StashEntry>, cx: &mut Context<Self>) {
        if self.stashes == stashes {
            return;
        }
        self.stashes = stashes;
        self.selected_stash = None;
        self.rebuild_nav_items();

        // Rebuild flattened stashes for virtualized rendering.
        self.flattened_stashes.clear();
        self.flattened_stashes.extend(0..self.stashes.len());

        cx.notify();
    }

    pub fn update_worktrees(&mut self, worktrees: Vec<WorktreeInfo>, cx: &mut Context<Self>) {
        if self.worktrees == worktrees {
            return;
        }
        let previously_selected_path = self.selected_worktree.and_then(|index| {
            self.worktrees
                .get(index)
                .map(|worktree| worktree.path.clone())
        });
        self.worktrees = worktrees;
        self.selected_worktree = previously_selected_path.and_then(|path| {
            self.worktrees
                .iter()
                .position(|worktree| worktree.path == path)
        });
        self.rebuild_nav_items();

        // Rebuild flattened worktrees for virtualized rendering.
        self.flattened_worktrees.clear();
        self.flattened_worktrees.extend(0..self.worktrees.len());

        cx.notify();
    }

    pub fn set_selected_worktree_by_path(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        let next = path.and_then(|path| {
            self.worktrees
                .iter()
                .position(|worktree| worktree.path == path)
        });
        if self.selected_worktree != next {
            self.selected_worktree = next;
            cx.notify();
        }
    }

    pub fn update_status(
        &mut self,
        staged: Vec<FileStatus>,
        unstaged: Vec<FileStatus>,
        cx: &mut Context<Self>,
    ) {
        log::debug!(
            "Sidebar::update_status: staged={} unstaged={}",
            staged.len(),
            unstaged.len()
        );
        if self.staged == staged && self.unstaged == unstaged {
            return;
        }
        self.cached_staged_tree = Self::build_file_tree(&staged);
        self.cached_unstaged_tree = Self::build_file_tree(&unstaged);
        self.staged = staged;
        self.unstaged = unstaged;

        // Rebuild flattened lists for virtualized rendering.
        // Clone tree roots to avoid chaining self borrows (E0502).
        let staged_tree = self.cached_staged_tree.clone();
        let unstaged_tree = self.cached_unstaged_tree.clone();
        let collapsed_dirs = self.collapsed_dirs.clone();
        self.flattened_staged.clear();
        Self::flatten_tree(
            &staged_tree,
            "staged",
            "",
            0,
            &mut self.flattened_staged,
            &collapsed_dirs,
        );
        self.flattened_unstaged.clear();
        Self::flatten_tree(
            &unstaged_tree,
            "unstaged",
            "",
            0,
            &mut self.flattened_unstaged,
            &collapsed_dirs,
        );

        // Rebuild flattened local branches for virtualized rendering.
        self.flattened_local_branches.clear();
        let visible_branch_indices =
            self.filtered_local_indices(self.current_user_email.as_deref());
        self.flattened_local_branches.extend(visible_branch_indices);

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

    pub(crate) fn toggle_dir(&mut self, prefix: &str, dir: &str, cx: &mut Context<Self>) {
        let key = format!("{}:{}", prefix, dir);
        if self.collapsed_dirs.contains(&key) {
            self.collapsed_dirs.remove(&key);
        } else {
            self.collapsed_dirs.insert(key);
        }

        // Rebuild flattened lists to reflect the new collapse state.
        // use take() to avoid &mut conflict with self.cached_* clone borrows.
        let staged_tree = self.cached_staged_tree.clone();
        let unstaged_tree = self.cached_unstaged_tree.clone();
        let collapsed_dirs = self.collapsed_dirs.clone();
        let mut new_staged = Vec::new();
        let mut new_unstaged = Vec::new();
        Self::flatten_tree(
            &staged_tree,
            "staged",
            "",
            0,
            &mut new_staged,
            &collapsed_dirs,
        );
        Self::flatten_tree(
            &unstaged_tree,
            "unstaged",
            "",
            0,
            &mut new_unstaged,
            &collapsed_dirs,
        );
        // Swap using take to avoid &mut conflict
        self.flattened_staged = new_staged;
        self.flattened_unstaged = new_unstaged;

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

    /// Flatten a file tree into a list, respecting the current collapsed state.
    /// Directories whose keys are in `collapsed_dirs` have their children skipped.
    /// File items store the original `file_idx`; directory items store the full
    /// `collapsed_dirs` key and the number of descendant files.
    fn flatten_tree(
        node: &FileTreeNode,
        prefix: &str,
        parent_path: &str,
        depth: usize,
        items: &mut Vec<FlatFileItem>,
        collapsed_dirs: &HashSet<String>,
    ) {
        let file_indent = px(16.0 + depth as f32 * 14.0);

        // Emit file rows first.
        for &file_idx in &node.file_indices {
            items.push(FlatFileItem::File {
                file_idx,
                indent: file_indent,
            });
        }

        // Then emit directories (pre-order: dir before its children).
        for (dir_name, child) in &node.children {
            let full_dir = if parent_path.is_empty() {
                dir_name.clone()
            } else {
                format!("{}/{}", parent_path, dir_name)
            };
            let dir_key = format!("{}:{}", prefix, full_dir);
            let _collapsed = collapsed_dirs.contains(&dir_key);

            // Coalesce single-child directories for a cleaner tree.
            let mut display_label = format!("{}/", dir_name);
            let mut display_node = child;
            let mut display_full_dir = full_dir;
            while display_node.file_indices.is_empty() && display_node.children.len() == 1 {
                let Some((next_name, next_child)) = display_node.children.iter().next() else {
                    break;
                };
                display_full_dir = format!("{}/{}", display_full_dir, next_name);
                display_label = format!("{}/{}/", display_label.trim_end_matches("/"), next_name);
                display_node = next_child;
            }
            let display_key = format!("{}:{}", prefix, display_full_dir);
            let display_collapsed = collapsed_dirs.contains(&display_key);

            items.push(FlatFileItem::Dir {
                dir_key: display_key,
                label: SharedString::from(display_label),
                file_count: display_node.file_count,
                collapsed: display_collapsed,
                indent: px(16.0 + depth as f32 * 14.0),
            });

            // Recurse into children only if not collapsed.
            if !display_collapsed {
                Self::flatten_tree(
                    display_node,
                    prefix,
                    &display_full_dir,
                    depth + 1,
                    items,
                    collapsed_dirs,
                );
            }
        }
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
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let item_h = compactness.spacing(24.0);
        let header_h = compactness.spacing(26.0);
        let sidebar_weak: WeakEntity<Sidebar> = cx.weak_entity();

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
        let filtered_count = if self.my_branches_active || !branch_filter.is_empty() {
            self.filtered_local_indices(self.current_user_email.as_deref())
                .len()
        } else {
            local_branch_count
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
                            Label::new(if !self.my_branches_active && branch_filter.is_empty() {
                                SharedString::from(format!("{}", local_branch_count))
                            } else {
                                SharedString::from(format!(
                                    "{}/{}",
                                    filtered_count, local_branch_count
                                ))
                            })
                            .size(LabelSize::XSmall)
                            .color(
                                if !self.my_branches_active && !branch_filter.is_empty() {
                                    Color::Accent
                                } else {
                                    Color::Muted
                                },
                            ),
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
                    .child(div().flex_1().child(self.branch_filter_editor.clone()))
                    .child(
                        IconButton::new("my-branches-filter", IconName::User)
                            .size(ButtonSize::Compact)
                            .color(if self.my_branches_active {
                                Color::Accent
                            } else {
                                Color::Muted
                            })
                            .tooltip(if self.my_branches_active {
                                "Show all branches"
                            } else {
                                "Show only my branches"
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.my_branches_active = !this.my_branches_active;
                                this.rebuild_flattened_branches();
                                this.rebuild_nav_items();
                                cx.notify();
                            })),
                    ),
            );
        }

        if local_expanded {
            let visible_branch_indices =
                self.filtered_local_indices(self.current_user_email.as_deref());
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

            // Virtualized local branches list
            nav_idx += self.flattened_local_branches.len();
            let flattened = self.flattened_local_branches.clone();
            let branches = self.local_branches.clone();
            let w = Rc::new(sidebar_weak.clone());
            let colors = colors.clone();

            let list = uniform_list(
                "local-branches-list",
                flattened.len(),
                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                    let w = w.clone();
                    range.map(|i| {
                        let branch_idx = flattened[i];
                        let branch = &branches[branch_idx];
                        let kb_active = keyboard_index == Some(i);
                        let name: SharedString = branch.name.clone().into();
                        let is_head = branch.is_head;

                        let w_click = w.clone();
                        let branch_name_for_click = name.clone();

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
                                    .border_color(colors.border_focused)
                            })
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .active(|s| s.bg(colors.ghost_element_active))
                            .cursor_pointer()
                            .on_click(move |event: &ClickEvent, _: &mut Window, cx: &mut App| {
                                if event.click_count() >= 2 {
                                    let _ = w_click.clone().update(cx, |_this, cx| {
                                        cx.emit(SidebarEvent::BranchCheckout(branch_name_for_click.to_string()));
                                    });
                                } else {
                                    let _ = w_click.clone().update(cx, |_this, cx| {
                                        cx.emit(SidebarEvent::BranchSelected(branch_name_for_click.to_string()));
                                    });
                                }
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
                                    Label::new(name.clone())
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
                                    Label::new(name.clone())
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
                                    .items_center()
                                    .when(branch.ahead > 0, |el| {
                                        el.child(
                                            Badge::new(format!("{}", branch.ahead))
                                                .color(Color::Success)
                                                .prefix("+"),
                                        )
                                    })
                                    .when(branch.behind > 0, |el| {
                                        el.child(
                                            Badge::new(format!("{}", branch.behind))
                                                .color(Color::Warning)
                                                .prefix("-"),
                                        )
                                    }),
                            );
                        }

                        // All branches: Copy name button (HEAD + non-HEAD).
                        // Non-HEAD branches also get: Checkout, Merge, Rename, Delete.
                        {
                            let w_cp = w.clone();
                            let bn_cp = name.clone();
                            let mut actions = div()
                                .ml_auto()
                                .h_flex()
                                .gap(px(2.))
                                .child(
                                    IconButton::new(
                                        ElementId::NamedInteger("copy-branch-name".into(), i as u64),
                                        IconName::Copy,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Muted)
                                    .tooltip("Copy branch name")
                                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        let _ = w_cp.clone().update(cx, |_this, cx| {
                                            cx.emit(SidebarEvent::BranchCopyName(bn_cp.to_string()));
                                        });
                                    }),
                                );

                            if !is_head {
                                let w_co = w.clone();
                                let bn_co = name.clone();
                                let w_mg = w.clone();
                                let bn_mg = name.clone();
                                let w_rn = w.clone();
                                let bn_rn = name.clone();
                                let w_dl = w.clone();
                                let bn_dl = name.clone();
                                actions = actions
                                    .child(
                                        IconButton::new(
                                            ElementId::NamedInteger("checkout-branch".into(), i as u64),
                                            IconName::Check,
                                        )
                                        .size(ButtonSize::Compact)
                                        .color(Color::Success)
                                        .tooltip("Checkout branch")
                                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ = w_co.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::BranchCheckout(bn_co.to_string()));
                                            });
                                        }),
                                    )
                                    .child(
                                        IconButton::new(
                                            ElementId::NamedInteger("merge-branch".into(), i as u64),
                                            IconName::GitMerge,
                                        )
                                        .size(ButtonSize::Compact)
                                        .color(Color::Muted)
                                        .tooltip("Merge into current branch")
                                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ = w_mg.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::MergeBranch(bn_mg.to_string()));
                                            });
                                        }),
                                    )
                                    .child(
                                        IconButton::new(
                                            ElementId::NamedInteger("rename-branch".into(), i as u64),
                                            IconName::Edit,
                                        )
                                        .size(ButtonSize::Compact)
                                        .color(Color::Muted)
                                        .tooltip("Rename branch")
                                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ = w_rn.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::BranchRename(bn_rn.to_string()));
                                            });
                                        }),
                                    )
                                    .child(
                                        IconButton::new(
                                            ElementId::NamedInteger("delete-branch".into(), i as u64),
                                            IconName::Trash,
                                        )
                                        .size(ButtonSize::Compact)
                                        .color(Color::Deleted)
                                        .tooltip("Delete branch")
                                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ = w_dl.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::BranchDelete(bn_dl.to_string()));
                                            });
                                        }),
                                    );
                            }
                            item = item.child(actions);
                        }

                        item
                    }).collect()
                },
            )
            .with_sizing_behavior(ListSizingBehavior::Infer);
            content = content.child(list);
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
            } else {
                let flattened = self.flattened_remotes.clone();
                nav_idx += flattened.len();
                let remotes_list = self.remotes.clone();
                let colors = colors.clone();
                let w = sidebar_weak.clone();

                content = content.child(
                    uniform_list(
                        "remotes-list",
                        flattened.len(),
                        move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                            let w = w.clone();
                            range
                                .map(|i| {
                                    let remote = &remotes_list[flattened[i]];
                                    let kb_active = keyboard_index == Some(i);
                                    let remote_name: SharedString = remote.name.clone().into();
                                    let url_text = remote
                                        .url
                                        .as_deref()
                                        .or(remote.push_url.as_deref())
                                        .unwrap_or("No URL configured")
                                        .to_string();
                                    let fetch_remote = remote_name.clone();
                                    let pull_remote = remote_name.clone();
                                    let push_remote = remote_name.clone();
                                    let remove_remote = remote_name.clone();

                                    let mut item = div()
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
                                        .active(|s| s.bg(colors.ghost_element_active));

                                    // Icon buttons: fetch, pull, push, remove
                                    let w_fetch = w.clone();
                                    let w_pull = w.clone();
                                    let w_push = w.clone();
                                    let w_remove = w.clone();

                                    item = item.child(
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
                                                Label::new(remote_name.clone())
                                                    .size(LabelSize::XSmall)
                                                    .weight(gpui::FontWeight::SEMIBOLD)
                                                    .truncate(),
                                            )
                                            .child(div().flex_1())
                                            .child(
                                                IconButton::new(
                                                    ElementId::NamedInteger(
                                                        "remote-fetch".into(),
                                                        i as u64,
                                                    ),
                                                    IconName::Refresh,
                                                )
                                                .size(ButtonSize::Compact)
                                                .color(Color::Info)
                                                .tooltip("Fetch from remote")
                                                .on_click(
                                                    move |_: &ClickEvent, _, cx: &mut App| {
                                                        let _ =
                                                            w_fetch.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::RemoteFetch(
                                                                    fetch_remote.to_string(),
                                                                ));
                                                            });
                                                    },
                                                ),
                                            )
                                            .child(
                                                IconButton::new(
                                                    ElementId::NamedInteger(
                                                        "remote-pull".into(),
                                                        i as u64,
                                                    ),
                                                    IconName::ArrowDown,
                                                )
                                                .size(ButtonSize::Compact)
                                                .color(Color::Warning)
                                                .tooltip("Pull from remote")
                                                .on_click(
                                                    move |_: &ClickEvent, _, cx: &mut App| {
                                                        let _ =
                                                            w_pull.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::RemotePull(
                                                                    pull_remote.to_string(),
                                                                ));
                                                            });
                                                    },
                                                ),
                                            )
                                            .child(
                                                IconButton::new(
                                                    ElementId::NamedInteger(
                                                        "remote-push".into(),
                                                        i as u64,
                                                    ),
                                                    IconName::ArrowUp,
                                                )
                                                .size(ButtonSize::Compact)
                                                .color(Color::Success)
                                                .tooltip("Push to remote")
                                                .on_click(
                                                    move |_: &ClickEvent, _, cx: &mut App| {
                                                        let _ =
                                                            w_push.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::RemotePush(
                                                                    push_remote.to_string(),
                                                                ));
                                                            });
                                                    },
                                                ),
                                            )
                                            .child(
                                                IconButton::new(
                                                    ElementId::NamedInteger(
                                                        "remote-remove".into(),
                                                        i as u64,
                                                    ),
                                                    IconName::Trash,
                                                )
                                                .size(ButtonSize::Compact)
                                                .color(Color::Deleted)
                                                .tooltip("Remove remote")
                                                .on_click(
                                                    move |_: &ClickEvent, _, cx: &mut App| {
                                                        let _ =
                                                            w_remove.clone().update(cx, |_, cx| {
                                                                cx.emit(
                                                                    SidebarEvent::RemoteRemove(
                                                                        remove_remote.to_string(),
                                                                    ),
                                                                );
                                                            });
                                                    },
                                                ),
                                            ),
                                    );
                                    item = item.child(
                                        Label::new(SharedString::from(url_text))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    );
                                    item
                                })
                                .collect()
                        },
                    )
                    .with_sizing_behavior(ListSizingBehavior::Infer),
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
            nav_idx += self.flattened_remote_branches.len();
            let flattened = self.flattened_remote_branches.clone();
            let remote_branches = self.remote_branches.clone();
            let w = Rc::new(sidebar_weak.clone());
            let colors = colors.clone();

            let list = uniform_list(
                "remote-branches-list",
                flattened.len(),
                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                    let w = w.clone();
                    range
                        .map(|i| {
                            let branch_idx = flattened[i];
                            let branch = &remote_branches[branch_idx];
                            let kb_active = keyboard_index == Some(i);
                            let name: SharedString = branch.name.clone().into();
                            let remote_branch_name = name.clone();
                            let w_item = w.clone();

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
                                .on_click(
                                    move |event: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        if event.click_count() >= 2 {
                                            let _ = w_item.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::BranchCheckout(
                                                    remote_branch_name.to_string(),
                                                ));
                                            });
                                        } else {
                                            let _ = w_item.clone().update(cx, |_this, cx| {
                                                cx.emit(SidebarEvent::BranchSelected(
                                                    remote_branch_name.to_string(),
                                                ));
                                            });
                                        }
                                    },
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
                                )
                        })
                        .collect()
                },
            )
            .with_sizing_behavior(ListSizingBehavior::Infer);
            content = content.child(list);
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
            // Virtualized tags list
            nav_idx += self.flattened_tags.len();
            if self.flattened_tags.is_empty() {
                // Empty state already rendered above
            } else {
                let flattened = self.flattened_tags.clone();
                let tags = self.tags.clone();
                let selected_tag = self.selected_tag.clone();
                let colors = colors.clone();
                let w = Rc::new(sidebar_weak.clone());

                content =
                    content.child(
                        uniform_list(
                            "tags-list",
                            flattened.len(),
                            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                                let w = w.clone();
                                range
                                    .map(|i| {
                                        let tag_idx = flattened[i];
                                        let tag = &tags[tag_idx];
                                        let kb_active = keyboard_index == Some(i);
                                        let is_selected =
                                            selected_tag.as_ref().is_some_and(|s| s == &tag.name);
                                        let name: SharedString = tag.name.clone().into();
                                        let tag_select = name.clone();
                                        let tag_delete = name.clone();
                                        let tag_checkout = name.clone();

                                        let mut item = div()
                                            .id(ElementId::NamedInteger(
                                                "tag-item".into(),
                                                i as u64,
                                            ))
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
                                            .cursor_pointer();

                                        let w_sel = w.clone();
                                        item = item.on_click(
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                let _ = w_sel.clone().update(
                                                    cx,
                                                    |this: &mut Sidebar, cx| {
                                                        this.selected_tag =
                                                            Some(tag_select.to_string());
                                                        cx.emit(SidebarEvent::TagSelected(
                                                            tag_select.to_string(),
                                                        ));
                                                    },
                                                );
                                            },
                                        );

                                        item = item
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
                                            .child(div().flex_1());

                                        // Checkout button
                                        let w_chk = w.clone();
                                        let tc = tag_checkout.clone();
                                        item = item.child(
                                    IconButton::new(
                                        ElementId::NamedInteger("checkout-tag".into(), i as u64),
                                        IconName::ArrowDown,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Muted)
                                    .tooltip("Checkout tag")
                                    .on_click(
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ =
                                                w_chk.clone().update(cx, |_: &mut Sidebar, cx| {
                                                    cx.emit(SidebarEvent::TagCheckout(
                                                        tc.to_string(),
                                                    ));
                                                });
                                        },
                                    ),
                                );

                                        // Delete button
                                        let w_del = w.clone();
                                        let td = tag_delete.clone();
                                        item.child(
                                    IconButton::new(
                                        ElementId::NamedInteger("delete-tag".into(), i as u64),
                                        IconName::Trash,
                                    )
                                    .size(ButtonSize::Compact)
                                    .color(Color::Deleted)
                                    .tooltip("Delete tag")
                                    .on_click(
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ =
                                                w_del.clone().update(cx, |_: &mut Sidebar, cx| {
                                                    cx.emit(SidebarEvent::TagDelete(
                                                        td.to_string(),
                                                    ));
                                                });
                                        },
                                    ),
                                )
                                    })
                                    .collect()
                            },
                        )
                        .with_sizing_behavior(ListSizingBehavior::Infer),
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
            } else {
                // Virtualized stash list
                nav_idx += self.stashes.len();
                let stashes = self.stashes.clone();
                let selected_stash = self.selected_stash;
                let colors = colors.clone();
                let w = sidebar_weak.clone();

                content = content.child(
                    uniform_list(
                        "stashes-list",
                        stashes.len(),
                        move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                            let w = w.clone();
                            range
                                .map(|i| {
                                    let stash = &stashes[i];
                                    let kb_active = keyboard_index == Some(i);
                                    let is_selected =
                                        selected_stash.as_ref().is_some_and(|s| *s == stash.index);
                                    let msg: SharedString = stash.message.clone().into();
                                    let stash_index = stash.index;

                                    let mut item = div()
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
                                        .active(|s| s.bg(colors.ghost_element_active));

                                    let w_sel = w.clone();
                                    item = item.on_click(
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let _ = w_sel.clone().update(
                                                cx,
                                                |this: &mut Sidebar, cx| {
                                                    this.selected_stash = Some(stash_index);
                                                    cx.emit(SidebarEvent::StashSelected(
                                                        stash_index,
                                                    ));
                                                },
                                            );
                                        },
                                    );

                                    item = item
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
                                                .child({
                                                    let w_apply = w.clone();
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
                                            .on_click(
                                                move |_: &ClickEvent, _: &mut Window,
                                                      cx: &mut App| {
                                                    let _ = w_apply.clone()
                                                        .update(cx, |_: &mut Sidebar, cx| {
                                                        cx.emit(SidebarEvent::StashApply(
                                                            stash_index,
                                                        ));
                                                    });
                                                },
                                            )
                                                })
                                                .child({
                                                    let w_pop = w.clone();
                                                    IconButton::new(
                                                ElementId::NamedInteger(
                                                    "pop-stash".into(),
                                                    i as u64,
                                                ),
                                                IconName::Undo,
                                            )
                                            .size(ButtonSize::Compact)
                                            .color(Color::Info)
                                            .tooltip("Pop stash")
                                            .on_click(
                                                move |_: &ClickEvent, _: &mut Window,
                                                      cx: &mut App| {
                                                    let _ = w_pop.clone()
                                                        .update(cx, |_: &mut Sidebar, cx| {
                                                        cx.emit(SidebarEvent::StashPop(
                                                            stash_index,
                                                        ));
                                                    });
                                                },
                                            )
                                                })
                                                .child({
                                                    let w_branch = w.clone();
                                                    IconButton::new(
                                                ElementId::NamedInteger(
                                                    "branch-stash".into(),
                                                    i as u64,
                                                ),
                                                IconName::GitBranch,
                                            )
                                            .size(ButtonSize::Compact)
                                            .tooltip("Create branch from stash")
                                            .on_click(
                                                move |_: &ClickEvent, _: &mut Window,
                                                      cx: &mut App| {
                                                    let _ = w_branch.clone()
                                                        .update(cx, |_: &mut Sidebar, cx| {
                                                        cx.emit(SidebarEvent::StashBranch(
                                                            stash_index,
                                                        ));
                                                    });
                                                },
                                            )
                                                })
                                                .child({
                                                    let w_drop = w.clone();
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
                                            .on_click(
                                                move |_: &ClickEvent, _: &mut Window,
                                                      cx: &mut App| {
                                                    let _ = w_drop.clone()
                                                        .update(cx, |_: &mut Sidebar, cx| {
                                                        cx.emit(SidebarEvent::StashDrop(
                                                            stash_index,
                                                        ));
                                                    });
                                                },
                                            )
                                                }),
                                        );

                                    item
                                })
                                .collect()
                        },
                    )
                    .with_sizing_behavior(ListSizingBehavior::Infer),
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
                        )
                        .when(!worktree.is_current, |el| {
                            el.child(
                                IconButton::new(
                                    ElementId::NamedInteger("remove-worktree".into(), i as u64),
                                    IconName::Trash,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Deleted)
                                .tooltip("Remove worktree")
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::WorktreeRemove(worktree_index));
                                    },
                                )),
                            )
                        }),
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

        let staged_chevron = if staged_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
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
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_section(SidebarSection::StagedChanges, cx);
            }))
            .child(
                rgitui_ui::Icon::new(staged_chevron)
                    .size(rgitui_ui::IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new("Staged")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
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
                let flattened = self.flattened_staged.clone();
                let files = self.staged.clone();
                let w = Rc::new(sidebar_weak.clone());
                let colors = colors.clone();
                let list = uniform_list(
                    "staged-file-list",
                    flattened.len(),
                    move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                        let w = w.clone();
                        range.map(|i| {
                            let item = &flattened[i];
                            match item {
                                FlatFileItem::File { file_idx, indent } => {
                                    let file = &files[*file_idx];
                                    let file_name: SharedString = file
                                        .path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("")
                                        .to_string()
                                        .into();
                                    let kind_color = Sidebar::file_change_color(file.kind);
                                    let file_path_for_emit: String =
                                        file.path.display().to_string();
                                    let file_path_clone = file_path_for_emit.clone();
                                    let file_path_unstage = file_path_for_emit.clone();
                                    div()
                                        .id(ElementId::NamedInteger("staged-file".into(), i as u64))
                                        .group("sidebar-file-row")
                                        .h_flex()
                                        .w_full()
                                        .h(px(item_h))
                                        .flex_shrink_0()
                                        .px_2()
                                        .pl(*indent)
                                        .gap_1()
                                        .items_center()
                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                        .active(|s| s.bg(colors.ghost_element_selected))
                                        .cursor_pointer()
                                        .on_click({
                                            let w_click = w.clone();
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            if let Some(entity) = (*w_click).upgrade() {
                                                entity.update(cx, |this, cx| {
                                                    this.selected_file =
                                                        Some((file_path_clone.clone(), true));
                                                    cx.emit(SidebarEvent::FileSelected {
                                                        path: file_path_clone.clone(),
                                                        staged: true,
                                                    });
                                                });
                                            }
                                        }})
                                        .child(
                                            Label::new(Sidebar::file_change_symbol(file.kind))
                                                .size(LabelSize::XSmall)
                                                .color(kind_color),
                                        )
                                        .child(
                                            Label::new(file_name)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Default)
                                                .truncate(),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .pr(px(2.))
                                                .child(
                                                    div()
                                                        .id(ElementId::NamedInteger("unstage-action".into(), i as u64))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .w(px(18.))
                                                        .h(px(18.))
                                                        .rounded(px(3.))
                                                        .invisible()
                                                        .group_hover("sidebar-file-row", |s| s.visible())
                                                        .text_xs()
                                                        .text_color(Color::Added.color(cx))
                                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                                        .cursor_pointer()
                                                        .tooltip(Tooltip::text("Unstage file"))
                                                        .on_click({
                                                            let w_unstg = w.clone();
                                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                            w_unstg.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::UnstageFile(
                                                                    file_path_unstage.clone(),
                                                                ));
                                                            }).ok();
                                                        }})
                                                        .child(rgitui_ui::Icon::new(IconName::Minus)),
                                                ),
                                        )
                                        .into_any_element()
                                }
                                FlatFileItem::Dir { dir_key, label, file_count, collapsed, indent } => {
                                    let dir_icon = if *collapsed {
                                        IconName::ChevronRight
                                    } else {
                                        IconName::ChevronDown
                                    };
                                    let dk = dir_key.clone();
                                    div()
                                        .id(SharedString::from(format!("staged-dir-{}", dir_key)))
                                        .h_flex()
                                        .w_full()
                                        .h(px(item_h))
                                        .flex_shrink_0()
                                        .px_2()
                                        .pl(*indent)
                                        .gap_1()
                                        .items_center()
                                        .overflow_hidden()
                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                        .active(|s| s.bg(colors.ghost_element_active))
                                        .cursor_pointer()
                                        .on_click({
                                            let w_dir = w.clone();
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            w_dir.clone().update(cx, |_, cx| {
                                                cx.emit(SidebarEvent::ToggleDir(dk.clone()));
                                            }).ok();
                                        }})
                                        .child(rgitui_ui::Icon::new(dir_icon).size(rgitui_ui::IconSize::XSmall).color(Color::Muted))
                                        .child(rgitui_ui::Icon::new(IconName::Folder).size(rgitui_ui::IconSize::XSmall).color(Color::Muted))
                                        .child(Label::new(label.clone()).size(LabelSize::XSmall).color(Color::Default).truncate())
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .h_flex().h(px(16.)).min_w(px(20.)).px(px(6.))
                                                .rounded(px(8.)).bg(colors.ghost_element_hover)
                                                .items_center().justify_center()
                                                .child(Label::new(SharedString::from(format!("{}", file_count))).size(LabelSize::XSmall).color(Color::Default)),
                                        )
                                        .into_any_element()
                                }
                            }
                        }).collect::<Vec<_>>()
                    },
                )
                .with_sizing_behavior(ListSizingBehavior::Infer);
                content = content.child(list);
            }
        }

        // -- Unstaged Changes --
        let unstaged_expanded = self.is_expanded(SidebarSection::UnstagedChanges);
        let unstaged_count = self.unstaged.len();
        let has_unstaged = unstaged_count > 0;
        let unstaged_kind_counts = Self::file_kind_counts(&self.unstaged);

        let kb_active = keyboard_index == Some(nav_idx);

        let unstaged_chevron = if unstaged_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
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
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_section(SidebarSection::UnstagedChanges, cx);
            }))
            .child(
                rgitui_ui::Icon::new(unstaged_chevron)
                    .size(rgitui_ui::IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new("Unstaged")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
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
                let flattened = self.flattened_unstaged.clone();
                let files = self.unstaged.clone();
                let w = sidebar_weak.clone();
                let colors = colors.clone();
                let list = uniform_list(
                    "unstaged-file-list",
                    flattened.len(),
                    move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                        let w = w.clone();
                        range.map(|i| {
                            let item = &flattened[i];
                            match item {
                                FlatFileItem::File { file_idx, indent } => {
                                    let file = &files[*file_idx];
                                    let file_name: SharedString = file
                                        .path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("")
                                        .to_string()
                                        .into();
                                    let kind_color = Sidebar::file_change_color(file.kind);
                                    let file_path_for_emit: String =
                                        file.path.display().to_string();
                                    let file_path_clone = file_path_for_emit.clone();
                                    let file_path_stage = file_path_for_emit.clone();
                                    div()
                                        .id(ElementId::NamedInteger("unstaged-file".into(), i as u64))
                                        .group("sidebar-file-row")
                                        .h_flex()
                                        .w_full()
                                        .h(px(item_h))
                                        .flex_shrink_0()
                                        .px_2()
                                        .pl(*indent)
                                        .gap_1()
                                        .items_center()
                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                        .active(|s| s.bg(colors.ghost_element_selected))
                                        .cursor_pointer()
                                        .on_click({
                                            let w_sel = w.clone();
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            w_sel.clone().update(cx, |this, cx| {
                                                this.selected_file =
                                                    Some((file_path_clone.clone(), false));
                                                cx.emit(SidebarEvent::FileSelected {
                                                    path: file_path_clone.clone(),
                                                    staged: false,
                                                });
                                            }).ok();
                                        }})
                                        .child(
                                            Label::new(Sidebar::file_change_symbol(file.kind))
                                                .size(LabelSize::XSmall)
                                                .color(kind_color),
                                        )
                                        .child(
                                            Label::new(file_name)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Default)
                                                .truncate(),
                                        )
                                        .child(div().flex_1())
                                        // Action buttons for unstaged files (wrapped for right padding)
                                        .child(
                                            div()
                                                .pr(px(2.))
                                                .h_flex()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .id(ElementId::NamedInteger("discard-action".into(), i as u64))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .w(px(18.))
                                                        .h(px(18.))
                                                        .rounded(px(3.))
                                                        .invisible()
                                                        .group_hover("sidebar-file-row", |s| s.visible())
                                                        .text_xs()
                                                        .text_color(Color::Deleted.color(cx))
                                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                                        .cursor_pointer()
                                                        .tooltip(Tooltip::text("Discard changes (Ctrl+Z)"))
                                                        .on_click({
                                                            let w_dis = w.clone();
                                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                            w_dis.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::DiscardFile(
                                                                    file_path_for_emit.clone(),
                                                                ));
                                                            }).ok();
                                                        }})
                                                        .child("x"),
                                                )
                                                .child(
                                                    div()
                                                        .id(ElementId::NamedInteger("stage-action".into(), i as u64))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .w(px(18.))
                                                        .h(px(18.))
                                                        .rounded(px(3.))
                                                        .invisible()
                                                        .group_hover("sidebar-file-row", |s| s.visible())
                                                        .text_xs()
                                                        .text_color(Color::Added.color(cx))
                                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                                        .cursor_pointer()
                                                        .tooltip(Tooltip::text("Stage file"))
                                                        .on_click({
                                                            let w_stg = w.clone();
                                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                            w_stg.clone().update(cx, |_, cx| {
                                                                cx.emit(SidebarEvent::StageFile(
                                                                    file_path_stage.clone(),
                                                                ));
                                                            }).ok();
                                                        }})
                                                        .child(rgitui_ui::Icon::new(IconName::Plus)),
                                                ),
                                        )
                                        .into_any_element()
                                }
                                FlatFileItem::Dir { dir_key, label, file_count, collapsed, indent } => {
                                    let dir_icon = if *collapsed {
                                        IconName::ChevronRight
                                    } else {
                                        IconName::ChevronDown
                                    };
                                    let dk = dir_key.clone();
                                    div()
                                        .id(SharedString::from(format!("unstaged-dir-{}", dir_key)))
                                        .h_flex()
                                        .w_full()
                                        .h(px(item_h))
                                        .flex_shrink_0()
                                        .px_2()
                                        .pl(*indent)
                                        .gap_1()
                                        .items_center()
                                        .overflow_hidden()
                                        .hover(|s| s.bg(colors.ghost_element_hover))
                                        .active(|s| s.bg(colors.ghost_element_active))
                                        .cursor_pointer()
                                        .on_click({
                                            let w_dir2 = w.clone();
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            w_dir2.clone().update(cx, |_, cx| {
                                                cx.emit(SidebarEvent::ToggleDir(dk.clone()));
                                            }).ok();
                                        }})
                                        .child(rgitui_ui::Icon::new(dir_icon).size(rgitui_ui::IconSize::XSmall).color(Color::Muted))
                                        .child(rgitui_ui::Icon::new(IconName::Folder).size(rgitui_ui::IconSize::XSmall).color(Color::Muted))
                                        .child(Label::new(label.clone()).size(LabelSize::XSmall).color(Color::Default).truncate())
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .h_flex().h(px(16.)).min_w(px(20.)).px(px(6.))
                                                .rounded(px(8.)).bg(colors.ghost_element_hover)
                                                .items_center().justify_center()
                                                .child(Label::new(SharedString::from(format!("{}", file_count))).size(LabelSize::XSmall).color(Color::Default)),
                                        )
                                        .into_any_element()
                                }
                            }
                        }).collect::<Vec<_>>()
                    },
                )
                .with_sizing_behavior(ListSizingBehavior::Infer);
                content = content.child(list);
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
    use rgitui_git::{BranchInfo, FileChangeKind, FileStatus};
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

    // --- filter_local_branch_indices tests ---

    fn make_branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_head: false,
            is_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            tip_oid: None,
            author_email: None,
            last_commit_time: None,
            is_merged_into_main: None,
        }
    }

    #[test]
    fn test_filter_branches_empty_filter_returns_all() {
        let branches = vec![
            make_branch("main"),
            make_branch("feature/login"),
            make_branch("feature/logout"),
            make_branch("bugfix/123"),
        ];
        let result = filter_local_branch_indices(&branches, "", false, None);
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_filter_branches_case_insensitive() {
        let branches = vec![
            make_branch("main"),
            make_branch("feature/login"),
            make_branch("Feature/Logout"),
            make_branch("FEATURE/DASHBOARD"),
        ];
        let result = filter_local_branch_indices(&branches, "feature", false, None);
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_filter_branches_substring_match() {
        let branches = vec![
            make_branch("main"),
            make_branch("feature/login"),
            make_branch("hotfix/urgent"),
            make_branch("release/1.0"),
        ];
        // "log" matches feature/login
        let result = filter_local_branch_indices(&branches, "log", false, None);
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn test_filter_branches_no_match_returns_empty() {
        let branches = vec![make_branch("main"), make_branch("feature/login")];
        let result = filter_local_branch_indices(&branches, "xyz", false, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_branches_empty_branches() {
        let branches: Vec<BranchInfo> = vec![];
        let result = filter_local_branch_indices(&branches, "main", false, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_branches_single_match() {
        let branches = vec![
            make_branch("main"),
            make_branch("develop"),
            make_branch("feature/xyz"),
        ];
        let result = filter_local_branch_indices(&branches, "develop", false, None);
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn test_filter_branches_matches_prefix_and_suffix() {
        let branches = vec![
            make_branch("main"),
            make_branch("feature/main-auth"),
            make_branch("release/main-v2"),
        ];
        // "main" appears in all three
        let result = filter_local_branch_indices(&branches, "main", false, None);
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn test_filter_branches_special_chars() {
        let branches = vec![
            make_branch("feature/login"),
            make_branch("feature/logout"),
            make_branch("release/v1"),
        ];
        // "/" and digits should work as normal filter characters
        let result = filter_local_branch_indices(&branches, "release/v1", false, None);
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn test_filter_branches_exact_match() {
        let branches = vec![
            make_branch("main"),
            make_branch("release/1.0"),
            make_branch("release/1.1"),
        ];
        let result = filter_local_branch_indices(&branches, "release/1.0", false, None);
        assert_eq!(result, vec![1]);
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
