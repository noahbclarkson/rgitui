use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    canvas, div, px, uniform_list, App, Bounds, ClickEvent, Context, ElementId, Entity,
    EventEmitter, FocusHandle, KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Render, ScrollStrategy, SharedString, Size,
    UniformListScrollHandle, WeakEntity, Window,
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
    StagedDir(String),   // collapsed_dirs key for a staged directory row
    UnstagedFile(usize), // index into unstaged files
    UnstagedDir(String), // collapsed_dirs key for an unstaged directory row
}

/// State for the right-click stash context menu.
struct StashContextMenuState {
    /// The index of the stash that was right-clicked.
    stash_index: usize,
    /// Window-relative position where the menu should appear.
    position: Point<Pixels>,
}

/// Minimum width of the stash context menu, in pixels.
const STASH_MENU_WIDTH: f32 = 180.0;
/// Height of a single stash context-menu item row, in pixels.
const STASH_MENU_ITEM_HEIGHT: f32 = 28.0;
/// Vertical padding applied to the top and bottom of the stash context menu.
const STASH_MENU_VERTICAL_PADDING: f32 = 3.0;
/// Number of action rows in the stash context menu (Apply, Pop, Create branch, Drop).
const STASH_MENU_ITEM_COUNT: f32 = 4.0;

/// Expanded sidebar sections remain content-sized up to this many rows. Larger
/// sections get their own bounded scrolling viewport so `uniform_list` can
/// render only the visible window instead of being measured at max-content.
const SIDEBAR_LIST_MAX_VISIBLE_ROWS: usize = 12;

fn bounded_sidebar_list_height(item_count: usize, row_height: f32) -> f32 {
    item_count.min(SIDEBAR_LIST_MAX_VISIBLE_ROWS) as f32 * row_height
}

/// Total rendered size of the stash context menu. Derived from the item count and
/// row height so the outside-click hit-test and the edge-clamping math stay in sync
/// with the actual menu content.
fn stash_menu_size() -> Size<Pixels> {
    Size::new(
        px(STASH_MENU_WIDTH),
        px(STASH_MENU_ITEM_COUNT * STASH_MENU_ITEM_HEIGHT + 2.0 * STASH_MENU_VERTICAL_PADDING),
    )
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
    /// Change kinds hidden from the Staged section. Empty means "show all";
    /// each kind in the set is filtered out of the list. Toggled by clicking the
    /// header chips; ephemeral (per-session) state.
    staged_hidden_kinds: HashSet<FileChangeKind>,
    /// Change kinds hidden from the Unstaged section (e.g. clicking the `?` chip
    /// hides untracked files). Empty means "show all". Ephemeral, toggled via
    /// the header chips.
    unstaged_hidden_kinds: HashSet<FileChangeKind>,
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
    /// Right-click context menu for stashes.
    stash_context_menu: Option<StashContextMenuState>,
    /// Cached bounds of the sidebar panel container (updated via on_allocate).
    container_bounds: Bounds<Pixels>,
    local_branches_scroll: UniformListScrollHandle,
    remotes_scroll: UniformListScrollHandle,
    remote_branches_scroll: UniformListScrollHandle,
    tags_scroll: UniformListScrollHandle,
    stashes_scroll: UniformListScrollHandle,
    worktrees_scroll: UniformListScrollHandle,
    staged_scroll: UniformListScrollHandle,
    unstaged_scroll: UniformListScrollHandle,
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
                    this.rebuild_flattened_branches();
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
            staged_hidden_kinds: HashSet::new(),
            unstaged_hidden_kinds: HashSet::new(),
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
            stash_context_menu: None,
            container_bounds: Bounds::new(Point::new(px(0.), px(0.)), Size::new(px(0.), px(0.))),
            local_branches_scroll: UniformListScrollHandle::new(),
            remotes_scroll: UniformListScrollHandle::new(),
            remote_branches_scroll: UniformListScrollHandle::new(),
            tags_scroll: UniformListScrollHandle::new(),
            stashes_scroll: UniformListScrollHandle::new(),
            worktrees_scroll: UniformListScrollHandle::new(),
            staged_scroll: UniformListScrollHandle::new(),
            unstaged_scroll: UniformListScrollHandle::new(),
        }
    }

    /// Show the stash context menu at the given window-relative position.
    fn show_stash_context_menu(
        &mut self,
        stash_index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.stash_context_menu = Some(StashContextMenuState {
            stash_index,
            position,
        });
        cx.notify();
    }

    /// Dismiss the stash context menu if it's open.
    fn dismiss_stash_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.stash_context_menu.is_some() {
            self.stash_context_menu = None;
            cx.notify();
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
            for &i in &self.flattened_local_branches {
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
            for item in &self.flattened_staged {
                match item {
                    FlatFileItem::File { file_idx, .. } => {
                        items.push(SidebarItem::StagedFile(*file_idx));
                    }
                    FlatFileItem::Dir { dir_key, .. } => {
                        items.push(SidebarItem::StagedDir(dir_key.clone()));
                    }
                }
            }
        }

        items.push(SidebarItem::SectionHeader(SidebarSection::UnstagedChanges));
        if is_expanded(SidebarSection::UnstagedChanges, &self.expanded_sections) {
            for item in &self.flattened_unstaged {
                match item {
                    FlatFileItem::File { file_idx, .. } => {
                        items.push(SidebarItem::UnstagedFile(*file_idx));
                    }
                    FlatFileItem::Dir { dir_key, .. } => {
                        items.push(SidebarItem::UnstagedDir(dir_key.clone()));
                    }
                }
            }
        }

        self.cached_nav_items = items;
    }

    /// Keep keyboard movement useful now that large sections have independent
    /// bounded list viewports. The outer sidebar still owns section-to-section
    /// scrolling, while this brings an off-screen row within its section into view.
    fn scroll_keyboard_row_into_view(&self) {
        let Some(selected_index) = self.keyboard_index else {
            return;
        };
        if selected_index >= self.cached_nav_items.len() {
            return;
        }
        let Some((section, row_index)) = self.cached_nav_items[..=selected_index]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, item)| match item {
                SidebarItem::SectionHeader(section) if index < selected_index => {
                    Some((*section, selected_index - index - 1))
                }
                SidebarItem::SectionHeader(_) => None,
                _ => None,
            })
        else {
            return;
        };

        let handle = match section {
            SidebarSection::LocalBranches => &self.local_branches_scroll,
            SidebarSection::Remotes => &self.remotes_scroll,
            SidebarSection::RemoteBranches => &self.remote_branches_scroll,
            SidebarSection::Tags => &self.tags_scroll,
            SidebarSection::Stashes => &self.stashes_scroll,
            SidebarSection::Worktrees => &self.worktrees_scroll,
            SidebarSection::StagedChanges => &self.staged_scroll,
            SidebarSection::UnstagedChanges => &self.unstaged_scroll,
        };
        handle.scroll_to_item(row_index, ScrollStrategy::Nearest);
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
            SidebarItem::StagedDir(dir_key) | SidebarItem::UnstagedDir(dir_key) => {
                cx.emit(SidebarEvent::ToggleDir(dir_key));
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
                    self.rebuild_flattened_branches();
                    self.rebuild_nav_items();
                    cx.notify();
                    cx.stop_propagation();
                }
            }
            // Bounded section lists follow keyboard selection through
            // their own scroll handles; the outer sidebar still owns movement
            // between section viewports.
            "up" | "k" => {
                let new_idx = match self.keyboard_index {
                    Some(i) if i > 0 => i - 1,
                    Some(_) => 0,
                    None => 0,
                };
                self.keyboard_index = Some(new_idx);
                self.scroll_keyboard_row_into_view();
                cx.notify();
            }
            "down" | "j" => {
                let max = self.cached_nav_items.len().saturating_sub(1);
                let new_idx = match self.keyboard_index {
                    Some(i) => (i + 1).min(max),
                    None => 0,
                };
                self.keyboard_index = Some(new_idx);
                self.scroll_keyboard_row_into_view();
                cx.notify();
            }
            "enter" | " " => {
                self.activate_keyboard_item(cx);
            }
            "home" => {
                self.keyboard_index = Some(0);
                self.scroll_keyboard_row_into_view();
                cx.notify();
            }
            "end" => {
                self.keyboard_index = Some(self.cached_nav_items.len().saturating_sub(1));
                self.scroll_keyboard_row_into_view();
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
        self.staged = staged;
        self.unstaged = unstaged;
        self.rebuild_change_trees();

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

        // Re-flatten to reflect the new collapse state. The cached trees are
        // unchanged by a collapse toggle, so only the flattened lists rebuild.
        self.reflatten_change_trees();

        // Keyboard navigation items mirror the flattened file lists, so rebuild
        // them to keep the cursor index aligned with the new collapse state.
        self.rebuild_nav_items();

        cx.notify();
    }

    /// Rebuild the cached staged/unstaged file trees from the current file
    /// lists (honoring the active per-section kind filters) and re-derive their
    /// flattened representations. Call when the underlying file data or a kind
    /// filter changes.
    fn rebuild_change_trees(&mut self) {
        let started = Instant::now();
        self.prune_hidden_kinds();
        self.cached_staged_tree = Self::build_file_tree(&self.staged, &self.staged_hidden_kinds);
        self.cached_unstaged_tree =
            Self::build_file_tree(&self.unstaged, &self.unstaged_hidden_kinds);
        self.reflatten_change_trees();
        let elapsed = started.elapsed();
        if elapsed >= Duration::from_millis(8) {
            log::debug!(
                "Sidebar change-tree preparation took {:?} ({} staged, {} unstaged, {} visible rows)",
                elapsed,
                self.staged.len(),
                self.unstaged.len(),
                self.flattened_staged.len() + self.flattened_unstaged.len()
            );
        }
    }

    /// Drop any hidden kind that no longer has a file in its section. Without
    /// this, hiding a kind and then staging away its last file would leave the
    /// kind pinned as hidden with no chip left to restore it — so if those files
    /// reappear later they would stay invisible with no way to unhide them.
    fn prune_hidden_kinds(&mut self) {
        if !self.staged_hidden_kinds.is_empty() {
            let present: HashSet<FileChangeKind> = self.staged.iter().map(|f| f.kind).collect();
            self.staged_hidden_kinds
                .retain(|kind| present.contains(kind));
        }
        if !self.unstaged_hidden_kinds.is_empty() {
            let present: HashSet<FileChangeKind> = self.unstaged.iter().map(|f| f.kind).collect();
            self.unstaged_hidden_kinds
                .retain(|kind| present.contains(kind));
        }
    }

    /// Re-derive the flattened staged/unstaged lists from the cached trees,
    /// honoring the current directory collapse state. The cached trees are left
    /// untouched, so this is the cheap path for collapse toggles.
    fn reflatten_change_trees(&mut self) {
        self.flattened_staged.clear();
        Self::flatten_tree(
            &self.cached_staged_tree,
            "staged",
            "",
            0,
            &mut self.flattened_staged,
            &self.collapsed_dirs,
        );
        self.flattened_unstaged.clear();
        Self::flatten_tree(
            &self.cached_unstaged_tree,
            "unstaged",
            "",
            0,
            &mut self.flattened_unstaged,
            &self.collapsed_dirs,
        );
    }

    /// Toggle the visibility of a change kind in the given changes section.
    /// Every kind is shown by default; clicking its chip hides that kind, and
    /// clicking again restores it. Hidden kinds combine, so e.g. hiding `?`
    /// removes untracked files while leaving everything else visible. State is
    /// per-session only.
    fn toggle_kind_visibility(
        &mut self,
        section: SidebarSection,
        kind: FileChangeKind,
        cx: &mut Context<Self>,
    ) {
        let hidden = match section {
            SidebarSection::StagedChanges => &mut self.staged_hidden_kinds,
            SidebarSection::UnstagedChanges => &mut self.unstaged_hidden_kinds,
            _ => return,
        };
        if !hidden.remove(&kind) {
            hidden.insert(kind);
        }
        self.rebuild_change_trees();
        self.rebuild_nav_items();
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

    /// Color for a file row's name label. Untracked files recede while tracked
    /// changes use the same high-contrast semantic color as their status icon.
    fn file_name_color(kind: FileChangeKind) -> Color {
        match kind {
            FileChangeKind::Untracked => Color::Muted,
            tracked => Self::file_change_color(tracked),
        }
    }

    /// Human-readable name for a change kind, used in chip tooltips.
    fn kind_display_name(kind: FileChangeKind) -> &'static str {
        match kind {
            FileChangeKind::Added => "Added",
            FileChangeKind::Modified => "Modified",
            FileChangeKind::Deleted => "Deleted",
            FileChangeKind::Renamed => "Renamed",
            FileChangeKind::Copied => "Copied",
            FileChangeKind::TypeChange => "Type-changed",
            FileChangeKind::Untracked => "Untracked",
            FileChangeKind::Conflicted => "Conflicted",
        }
    }

    /// Render one interactive change-kind chip for a changes section header.
    /// The chip doubles as a count badge (`~7`, `?1`, …) and a visibility
    /// toggle: every kind is shown by default, and clicking a chip hides that
    /// kind from the list. Shown chips carry the kind's color and a neutral
    /// outline; hidden chips are dimmed and struck through.
    fn kind_chip(
        &self,
        section: SidebarSection,
        kind: FileChangeKind,
        count: usize,
        hidden: bool,
        colors: &rgitui_theme::ThemeColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kind_color = Self::file_change_color(kind);
        let text: SharedString = format!("{}{}", Self::file_change_symbol(kind), count).into();
        let prefix = match section {
            SidebarSection::StagedChanges => "staged",
            _ => "unstaged",
        };
        let id = SharedString::from(format!("{}-kind-chip-{}", prefix, kind.short_code()));
        let name = Self::kind_display_name(kind);
        let tooltip: SharedString = if hidden {
            format!("{} hidden — click to show", name).into()
        } else {
            format!("Hide {}", name).into()
        };

        div()
            .id(id)
            .h_flex()
            .items_center()
            .h(px(16.))
            .px(px(5.))
            .rounded(px(4.))
            .border_1()
            .border_color(colors.border_variant)
            .when(hidden, |el| el.opacity(0.45))
            .hover(|s| s.bg(colors.ghost_element_hover))
            .cursor_pointer()
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_kind_visibility(section, kind, cx);
            }))
            .child({
                let mut label = Label::new(text).size(LabelSize::XSmall).color(if hidden {
                    Color::Muted
                } else {
                    kind_color
                });
                if hidden {
                    label = label.strikethrough();
                }
                label
            })
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

    /// Build the directory tree for a set of file changes. Files whose kind is
    /// in `hidden` are skipped; the original index into `files` is preserved for
    /// every retained file, so downstream `file_idx` lookups into the unfiltered
    /// `staged`/`unstaged` slices stay valid.
    fn build_file_tree(files: &[FileStatus], hidden: &HashSet<FileChangeKind>) -> FileTreeNode {
        let mut root = FileTreeNode::default();
        for (idx, file) in files.iter().enumerate() {
            if hidden.contains(&file.kind) {
                continue;
            }
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

    #[cfg(test)]
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
    // TODO(audit): QUAL-03 — this render method is ~2400 lines with several
    // near-duplicate uniform_list blocks. Extract per-section row renderers and a
    // shared section/list builder so render becomes a thin orchestrator (ref Zed's
    // git_panel.rs render_* helpers). Deferred: large structural refactor.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let item_h = compactness.spacing(24.0);
        let header_h = compactness.spacing(26.0);
        let sidebar_weak: WeakEntity<Sidebar> = cx.weak_entity();

        // Compute navigable items for keyboard highlight matching
        let keyboard_index = self.keyboard_index;

        // Bounds tracker — updates container_bounds whenever the panel resizes.
        let sidebar_bounds = cx.weak_entity();
        let bounds_tracker = canvas(
            {
                let sidebar_bounds = sidebar_bounds.clone();
                move |bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                    sidebar_bounds
                        .update(cx, |this: &mut Sidebar, _| {
                            this.container_bounds = bounds;
                        })
                        .ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        // Dismiss context menu on left-click outside the menu bounds.
        let stash_dismiss = cx.weak_entity();
        let has_stash_menu = self.stash_context_menu.is_some();

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
            .border_color(colors.border_variant)
            .when(has_stash_menu, |el| {
                el.on_mouse_down(MouseButton::Left, {
                    let dismiss = stash_dismiss.clone();
                    move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        dismiss
                            .update(cx, |this: &mut Sidebar, cx| {
                                let click_inside_menu =
                                    this.stash_context_menu.as_ref().is_some_and(|cm| {
                                        let menu_size = stash_menu_size();
                                        let x = event.position.x;
                                        let y = event.position.y;
                                        x >= cm.position.x
                                            && x < cm.position.x + menu_size.width
                                            && y >= cm.position.y
                                            && y < cm.position.y + menu_size.height
                                    });
                                if !click_inside_menu {
                                    this.dismiss_stash_context_menu(cx);
                                }
                            })
                            .ok();
                    }
                })
            })
            .child(bounds_tracker);

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
        // `flattened_local_branches` already holds the filtered indices (kept in
        // sync with the filter text and "My Branches" toggle), so use its length
        // directly rather than re-running the per-branch filter every frame.
        let filtered_count = self.flattened_local_branches.len();

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
            if self.flattened_local_branches.is_empty() {
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
            let nav_base = nav_idx;
            nav_idx += self.flattened_local_branches.len();
            let flattened = self.flattened_local_branches.clone();
            let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
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
                        let kb_active = keyboard_index == Some(nav_base + i);
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
                                    div().flex_1().min_w_0().child(
                                        Label::new(name.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Accent)
                                            .weight(gpui::FontWeight::BOLD)
                                            .truncate(),
                                    ),
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
                                    div().flex_1().min_w_0().child(
                                        Label::new(name.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    ),
                                );
                        }

                        if !is_head && branch.is_merged_into_head == Some(true) {
                            item = item.child(
                                div()
                                    .id(SharedString::from(format!("merged-branch-{i}")))
                                    .flex_shrink_0()
                                    .tooltip(Tooltip::text("Merged into current branch"))
                                    .child(
                                        rgitui_ui::Icon::new(IconName::GitMerge)
                                            .size(rgitui_ui::IconSize::XSmall)
                                            .color(Color::Success),
                                    ),
                            );
                        }

                        if branch.ahead > 0 || branch.behind > 0 {
                            item = item.child(
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
                                .flex_shrink_0()
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
            .h(px(list_height))
            .with_sizing_behavior(ListSizingBehavior::Auto)
            .track_scroll(&self.local_branches_scroll);
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
                let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
                let nav_base = nav_idx;
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
                                    let kb_active = keyboard_index == Some(nav_base + i);
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
                    .h(px(list_height))
                    .with_sizing_behavior(ListSizingBehavior::Auto)
                    .track_scroll(&self.remotes_scroll),
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
            let nav_base = nav_idx;
            nav_idx += self.flattened_remote_branches.len();
            let flattened = self.flattened_remote_branches.clone();
            let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
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
                            let kb_active = keyboard_index == Some(nav_base + i);
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
            .h(px(list_height))
            .with_sizing_behavior(ListSizingBehavior::Auto)
            .track_scroll(&self.remote_branches_scroll);
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
            let nav_base = nav_idx;
            nav_idx += self.flattened_tags.len();
            if self.flattened_tags.is_empty() {
                // Empty state already rendered above
            } else {
                let flattened = self.flattened_tags.clone();
                let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
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
                                        let kb_active = keyboard_index == Some(nav_base + i);
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
                        .h(px(list_height))
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .track_scroll(&self.tags_scroll),
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
                let nav_base = nav_idx;
                nav_idx += self.stashes.len();
                let stashes = self.stashes.clone();
                let list_height = bounded_sidebar_list_height(stashes.len(), item_h);
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
                                    let kb_active = keyboard_index == Some(nav_base + i);
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
                                        .active(|s| s.bg(colors.ghost_element_active))
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            {
                                                let w = w.clone();
                                                move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                                                    let _ = w.update(cx, |this: &mut Sidebar, cx| {
                                                        this.show_stash_context_menu(stash_index, event.position, cx);
                                                    });
                                                }
                                            },
                                        );

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
                    .h(px(list_height))
                    .with_sizing_behavior(ListSizingBehavior::Auto)
                    .track_scroll(&self.stashes_scroll),
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
                .overflow_hidden()
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
                    div()
                        .id("new-worktree-btn")
                        .min_w_0()
                        .overflow_hidden()
                        .child(
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
                        .flex_shrink_0()
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
            if !self.worktrees.is_empty() {
                let nav_base = nav_idx;
                nav_idx += self.flattened_worktrees.len();
                let flattened = self.flattened_worktrees.clone();
                let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
                let worktrees = self.worktrees.clone();
                let selected_worktree = self.selected_worktree;
                let colors = colors.clone();
                let w = Rc::new(sidebar_weak.clone());

                let list = uniform_list(
                    "worktrees-list",
                    flattened.len(),
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        let w = w.clone();
                        range
                            .map(|i| {
                                let worktree_index = flattened[i];
                                let worktree = &worktrees[worktree_index];
                                let kb_active = keyboard_index == Some(nav_base + i);
                                let is_selected = selected_worktree == Some(worktree_index);
                                let name: SharedString = worktree.name.clone().into();
                                let path: SharedString =
                                    worktree.path.display().to_string().into();
                                let w_sel = w.clone();
                                let w_rm = w.clone();

                                div()
                                    .id(ElementId::NamedInteger(
                                        "worktree-item".into(),
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
                                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        let _ = w_sel.clone().update(cx, |this: &mut Sidebar, cx| {
                                            this.selected_worktree = Some(worktree_index);
                                            cx.emit(SidebarEvent::WorktreeSelected(worktree_index));
                                        });
                                    })
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
                                                ElementId::NamedInteger(
                                                    "remove-worktree".into(),
                                                    i as u64,
                                                ),
                                                IconName::Trash,
                                            )
                                            .size(ButtonSize::Compact)
                                            .color(Color::Deleted)
                                            .tooltip("Remove worktree")
                                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                let _ = w_rm.clone().update(cx, |_: &mut Sidebar, cx| {
                                                    cx.emit(SidebarEvent::WorktreeRemove(
                                                        worktree_index,
                                                    ));
                                                });
                                            }),
                                        )
                                    })
                            })
                            .collect()
                    },
                )
                .h(px(list_height))
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .track_scroll(&self.worktrees_scroll);
                content = content.child(list);
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
            let mut breakdown = div().h_flex().gap(px(4.));
            for kind in &kind_order {
                if let Some(&count) = staged_kind_counts.get(kind) {
                    if count > 0 {
                        breakdown = breakdown.child(
                            Label::new(SharedString::from(format!(
                                "{}{}",
                                Self::file_change_symbol(*kind),
                                count
                            )))
                            .size(LabelSize::XSmall)
                            .color(Self::file_change_color(*kind)),
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
                let nav_base = nav_idx;
                nav_idx += self.flattened_staged.len();
                let flattened = self.flattened_staged.clone();
                let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
                let files = self.staged.clone();
                let selected_file = self.selected_file.clone();
                let w = Rc::new(sidebar_weak.clone());
                let colors = colors.clone();
                let list = uniform_list(
                    "staged-file-list",
                    flattened.len(),
                    move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                        let w = w.clone();
                        range.map(|i| {
                            let item = &flattened[i];
                            let kb_active = keyboard_index == Some(nav_base + i);
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
                                    let is_selected = selected_file.as_ref().is_some_and(
                                        |(p, staged)| *staged && p == &file_path_for_emit,
                                    );
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
                                                .color(Sidebar::file_name_color(file.kind))
                                                .truncate(),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .pr(px(4.))
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
                                        .when(kb_active, |el| {
                                            el.bg(colors.ghost_element_hover)
                                                .border_l_2()
                                                .border_color(kb_accent)
                                        })
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
                .h(px(list_height))
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .track_scroll(&self.staged_scroll);
                content = content.child(list);
            }
        }

        // -- Unstaged Changes --
        let unstaged_expanded = self.is_expanded(SidebarSection::UnstagedChanges);
        let unstaged_count = self.unstaged.len();
        let has_unstaged = unstaged_count > 0;
        let unstaged_kind_counts = Self::file_kind_counts(&self.unstaged);

        let kb_active = keyboard_index == Some(nav_idx);
        nav_idx += 1;
        // Unstaged is the final navigable section, so `nav_idx` is not advanced
        // past its rows; this base anchors the list rows in the shared flat
        // keyboard-navigation index space.
        let unstaged_nav_base = nav_idx;

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
            let mut breakdown = div().h_flex().gap(px(4.));
            for kind in &kind_order {
                if let Some(&count) = unstaged_kind_counts.get(kind) {
                    if count > 0 {
                        if *kind == FileChangeKind::Untracked {
                            let hidden = self.unstaged_hidden_kinds.contains(kind);
                            breakdown = breakdown.child(self.kind_chip(
                                SidebarSection::UnstagedChanges,
                                *kind,
                                count,
                                hidden,
                                &colors,
                                cx,
                            ));
                        } else {
                            breakdown = breakdown.child(
                                Label::new(SharedString::from(format!(
                                    "{}{}",
                                    Self::file_change_symbol(*kind),
                                    count
                                )))
                                .size(LabelSize::XSmall)
                                .color(Self::file_change_color(*kind)),
                            );
                        }
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
                let nav_base = unstaged_nav_base;
                let flattened = self.flattened_unstaged.clone();
                let list_height = bounded_sidebar_list_height(flattened.len(), item_h);
                let files = self.unstaged.clone();
                let selected_file = self.selected_file.clone();
                let w = sidebar_weak.clone();
                let colors = colors.clone();
                let list = uniform_list(
                    "unstaged-file-list",
                    flattened.len(),
                    move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                        let w = w.clone();
                        range.map(|i| {
                            let item = &flattened[i];
                            let kb_active = keyboard_index == Some(nav_base + i);
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
                                    let is_selected = selected_file.as_ref().is_some_and(
                                        |(p, staged)| !*staged && p == &file_path_for_emit,
                                    );
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
                                                .color(Sidebar::file_name_color(file.kind))
                                                .truncate(),
                                        )
                                        .child(div().flex_1())
                                        // Action buttons for unstaged files (wrapped for right padding)
                                        .child(
                                            div()
                                                .pr(px(4.))
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
                                        .when(kb_active, |el| {
                                            el.bg(colors.ghost_element_hover)
                                                .border_l_2()
                                                .border_color(kb_accent)
                                        })
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
                .h(px(list_height))
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .track_scroll(&self.unstaged_scroll);
                content = content.child(list);
            }
        }

        // Build scroll div and context menu (if open), then return panel.
        let scroll_div = div()
            .id("sidebar-scroll")
            .v_flex()
            .w_full()
            .flex_1()
            .overflow_y_scroll()
            .child(content);

        if let Some(ref menu_state) = self.stash_context_menu {
            let menu_bg = colors.elevated_surface_background;
            let menu_border = colors.border;
            let menu_hover = colors.ghost_element_hover;
            let menu_active = colors.ghost_element_active;
            let pos = menu_state.position;
            let container_bounds = self.container_bounds;
            let weak = cx.weak_entity();

            // Convert window-relative click position to container-relative coordinates.
            let menu_size = stash_menu_size();
            let rel_x = pos.x - container_bounds.origin.x;
            let rel_y = pos.y - container_bounds.origin.y;
            let max_x = container_bounds.size.width - menu_size.width;
            let max_y = container_bounds.size.height - menu_size.height;
            let clamped_x = rel_x.max(px(0.)).min(max_x);
            let clamped_y = rel_y.max(px(0.)).min(max_y);

            let mut menu = div()
                .id("stash-context-menu")
                .absolute()
                .left(clamped_x)
                .top(clamped_y)
                .v_flex()
                .min_w(menu_size.width)
                .py(px(STASH_MENU_VERTICAL_PADDING))
                .bg(menu_bg)
                .border_1()
                .border_color(menu_border)
                .rounded(px(6.))
                .elevation_3(cx)
                .on_mouse_down(
                    MouseButton::Left,
                    |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    },
                )
                .on_mouse_down(
                    MouseButton::Right,
                    |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    },
                )
                .on_mouse_move(|_: &MouseMoveEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                });

            // Apply
            {
                let w = weak.clone();
                menu = menu.child(
                    div()
                        .id("stash-menu-apply")
                        .h_flex()
                        .w_full()
                        .h(px(STASH_MENU_ITEM_HEIGHT))
                        .px(px(8.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(|s| {
                            s.bg(menu_hover)
                                .border_l_2()
                                .border_color(colors.text_accent)
                        })
                        .active(|s| s.bg(menu_active))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            w.update(cx, |this: &mut Sidebar, cx| {
                                let idx = this.stash_context_menu.as_ref().map(|m| m.stash_index);
                                this.stash_context_menu = None;
                                cx.notify();
                                if let Some(i) = idx {
                                    cx.emit(SidebarEvent::StashApply(i));
                                }
                            })
                            .ok();
                        })
                        .child(
                            rgitui_ui::Icon::new(IconName::Check)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Apply stash")
                                .size(LabelSize::XSmall)
                                .color(Color::Default),
                        ),
                );
            }
            // Pop
            {
                let w = weak.clone();
                menu = menu.child(
                    div()
                        .id("stash-menu-pop")
                        .h_flex()
                        .w_full()
                        .h(px(STASH_MENU_ITEM_HEIGHT))
                        .px(px(8.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(|s| {
                            s.bg(menu_hover)
                                .border_l_2()
                                .border_color(colors.text_accent)
                        })
                        .active(|s| s.bg(menu_active))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            w.update(cx, |this: &mut Sidebar, cx| {
                                let idx = this.stash_context_menu.as_ref().map(|m| m.stash_index);
                                this.stash_context_menu = None;
                                cx.notify();
                                if let Some(i) = idx {
                                    cx.emit(SidebarEvent::StashPop(i));
                                }
                            })
                            .ok();
                        })
                        .child(
                            rgitui_ui::Icon::new(IconName::Undo)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Pop stash")
                                .size(LabelSize::XSmall)
                                .color(Color::Default),
                        ),
                );
            }
            // Create branch
            {
                let w = weak.clone();
                menu = menu.child(
                    div()
                        .id("stash-menu-create-branch")
                        .h_flex()
                        .w_full()
                        .h(px(STASH_MENU_ITEM_HEIGHT))
                        .px(px(8.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(|s| {
                            s.bg(menu_hover)
                                .border_l_2()
                                .border_color(colors.text_accent)
                        })
                        .active(|s| s.bg(menu_active))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            w.update(cx, |this: &mut Sidebar, cx| {
                                let idx = this.stash_context_menu.as_ref().map(|m| m.stash_index);
                                this.stash_context_menu = None;
                                cx.notify();
                                if let Some(i) = idx {
                                    cx.emit(SidebarEvent::StashBranch(i));
                                }
                            })
                            .ok();
                        })
                        .child(
                            rgitui_ui::Icon::new(IconName::GitBranch)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Create branch")
                                .size(LabelSize::XSmall)
                                .color(Color::Default),
                        ),
                );
            }
            // Drop
            {
                let w = weak.clone();
                menu = menu.child(
                    div()
                        .id("stash-menu-drop")
                        .h_flex()
                        .w_full()
                        .h(px(STASH_MENU_ITEM_HEIGHT))
                        .px(px(8.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(|s| {
                            s.bg(menu_hover)
                                .border_l_2()
                                .border_color(colors.text_accent)
                        })
                        .active(|s| s.bg(menu_active))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            w.update(cx, |this: &mut Sidebar, cx| {
                                let idx = this.stash_context_menu.as_ref().map(|m| m.stash_index);
                                this.stash_context_menu = None;
                                cx.notify();
                                if let Some(i) = idx {
                                    cx.emit(SidebarEvent::StashDrop(i));
                                }
                            })
                            .ok();
                        })
                        .child(
                            rgitui_ui::Icon::new(IconName::Trash)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Deleted),
                        )
                        .child(
                            Label::new("Drop stash")
                                .size(LabelSize::XSmall)
                                .color(Color::Default),
                        ),
                );
            }

            panel.child(scroll_div).child(menu)
        } else {
            panel.child(scroll_div)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgitui_git::{BranchInfo, FileChangeKind, FileStatus};
    use std::path::PathBuf;

    #[test]
    fn bounded_list_height_sizes_short_lists_to_content() {
        assert_eq!(bounded_sidebar_list_height(3, 24.0), 72.0);
    }

    #[test]
    fn bounded_list_height_caps_large_lists() {
        assert_eq!(
            bounded_sidebar_list_height(SIDEBAR_LIST_MAX_VISIBLE_ROWS + 50, 24.0),
            SIDEBAR_LIST_MAX_VISIBLE_ROWS as f32 * 24.0
        );
    }

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
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
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
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
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
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
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
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
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

    #[test]
    fn test_build_file_tree_no_hidden_includes_all() {
        let files = vec![
            make_file("a.rs", FileChangeKind::Modified),
            make_file("b.rs", FileChangeKind::Untracked),
        ];
        // An empty hidden set means "show all" — every file is retained.
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
        assert_eq!(tree.file_indices.len(), 2);
    }

    #[test]
    fn test_build_file_tree_hidden_kind_is_excluded() {
        // Hiding the untracked kind (clicking the `?` chip) removes untracked
        // files and keeps everything else.
        let files = vec![
            make_file("a.rs", FileChangeKind::Modified),
            make_file("b.rs", FileChangeKind::Untracked),
            make_file("c.rs", FileChangeKind::Modified),
        ];
        let hidden = HashSet::from([FileChangeKind::Untracked]);
        let tree = Sidebar::build_file_tree(&files, &hidden);
        // The untracked file (original index 1) is dropped, and the retained
        // files keep their original indices into the unfiltered slice.
        assert_eq!(tree.file_indices, vec![0, 2]);
        assert_eq!(Sidebar::file_tree_file_count(&tree), 2);
    }

    #[test]
    fn test_build_file_tree_multiple_hidden_kinds_combine() {
        // Hiding several kinds removes all of them, leaving the rest.
        let files = vec![
            make_file("a.rs", FileChangeKind::Added),
            make_file("b.rs", FileChangeKind::Modified),
            make_file("c.rs", FileChangeKind::Untracked),
        ];
        let hidden = HashSet::from([FileChangeKind::Added, FileChangeKind::Modified]);
        let tree = Sidebar::build_file_tree(&files, &hidden);
        assert_eq!(tree.file_indices, vec![2]);
    }

    // --- file_name_color tests ---

    #[test]
    fn test_file_name_color_untracked_is_dimmed() {
        assert_eq!(
            Sidebar::file_name_color(FileChangeKind::Untracked),
            Color::Muted
        );
    }

    #[test]
    fn test_file_name_color_tracked_kinds_use_status_color() {
        for kind in [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
            FileChangeKind::Copied,
            FileChangeKind::TypeChange,
            FileChangeKind::Conflicted,
        ] {
            assert_eq!(
                Sidebar::file_name_color(kind),
                Sidebar::file_change_color(kind)
            );
        }
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
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
        assert_eq!(Sidebar::file_tree_file_count(&tree), 2);
    }

    #[test]
    fn test_file_tree_file_count_nested() {
        let files = vec![
            make_file("src/main.rs", FileChangeKind::Modified),
            make_file("src/lib.rs", FileChangeKind::Added),
            make_file("tests/test.rs", FileChangeKind::Modified),
        ];
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
        assert_eq!(Sidebar::file_tree_file_count(&tree), 3);
    }

    #[test]
    fn test_file_tree_file_count_deep() {
        let files = vec![
            make_file("a/b/c.rs", FileChangeKind::Added),
            make_file("a/d.rs", FileChangeKind::Modified),
        ];
        let tree = Sidebar::build_file_tree(&files, &HashSet::new());
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
            is_merged_into_head: None,
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
