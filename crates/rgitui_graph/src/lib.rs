use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, point, px, uniform_list, Animation, AnimationExt, App, Bounds, ClickEvent,
    Context, CursorStyle, ElementId, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    ListSizingBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, PathBuilder,
    Pixels, Point, Render, ScrollStrategy, SharedString, Size, UniformListScrollHandle, WeakEntity,
    Window,
};
use rgitui_git::{compute_graph, CommitInfo, FileChangeKind, GraphEdge, GraphRow, RefLabel};
use rgitui_settings::{GraphStyle, SettingsState};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    AvatarCache, Badge, CheckState, Checkbox, Icon, IconName, IconSize, Label, LabelSize, Tooltip,
};

#[derive(Clone)]
pub struct AuthorColumnResize;
impl Render for AuthorColumnResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
pub struct DateColumnResize;
impl Render for DateColumnResize {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Pre-computed unit circle vertex offsets (cos, sin) for 36-step circles.
/// Computed once and reused across all frames to avoid per-frame trig calls.
fn unit_circle_offsets() -> &'static [(f32, f32)] {
    use std::sync::OnceLock;
    static OFFSETS: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    OFFSETS.get_or_init(|| {
        let steps = 36_usize;
        (0..=steps)
            .map(|s| {
                let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                (angle.cos(), angle.sin())
            })
            .collect()
    })
}

/// Build a filled circle path from pre-computed unit circle offsets.
fn build_filled_circle(cx_x: Pixels, cy_y: Pixels, radius: f32) -> Option<gpui::Path<Pixels>> {
    let offsets = unit_circle_offsets();
    let mut path = PathBuilder::fill();
    for (i, &(cos, sin)) in offsets.iter().enumerate() {
        // Skip the last (duplicate of first, only needed for stroke closure)
        if i >= 36 {
            break;
        }
        let x = cx_x + px(radius * cos);
        let y = cy_y + px(radius * sin);
        if i == 0 {
            path.move_to(point(x, y));
        } else {
            path.line_to(point(x, y));
        }
    }
    path.close();
    path.build().ok()
}

/// Build a stroked circle path from pre-computed unit circle offsets.
fn build_stroked_circle(
    cx_x: Pixels,
    cy_y: Pixels,
    radius: f32,
    stroke_width: Pixels,
) -> Option<gpui::Path<Pixels>> {
    let offsets = unit_circle_offsets();
    let mut path = PathBuilder::stroke(stroke_width);
    for (i, &(cos, sin)) in offsets.iter().enumerate() {
        let x = cx_x + px(radius * cos);
        let y = cy_y + px(radius * sin);
        if i == 0 {
            path.move_to(point(x, y));
        } else {
            path.line_to(point(x, y));
        }
    }
    path.build().ok()
}

/// Pre-computed quarter-circle arc offsets (sin, 1-cos) for 12-segment arcs.
fn quarter_arc_offsets() -> &'static [(f32, f32)] {
    use std::sync::OnceLock;
    static OFFSETS: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    OFFSETS.get_or_init(|| {
        let segments = 12_usize;
        (1..=segments)
            .map(|s| {
                let t = s as f32 / segments as f32;
                let angle = t * std::f32::consts::FRAC_PI_2;
                (angle.sin(), 1.0 - angle.cos())
            })
            .collect()
    })
}

/// Events emitted by the graph view.
#[derive(Debug, Clone)]
pub enum GraphViewEvent {
    CommitSelected(git2::Oid),
    CherryPick(git2::Oid),
    RevertCommit(git2::Oid),
    CreateBranchAtCommit(git2::Oid),
    CheckoutCommit(git2::Oid),
    CopyCommitSha(String),
    CopyCommitMessage(String),
    CopyAuthorName(String),
    CopyDate(String),
    /// Open this commit on GitHub (or compatible host).
    ViewOnGithub(git2::Oid),
    CreateTagAtCommit(git2::Oid),
    ResetToCommit(git2::Oid, String),
    /// Request to load more commits beyond the current set.
    LoadMoreCommits,
    /// Toggle "My Commits" filter — show only commits authored by the current user.
    ToggleMyCommits,
    /// A worktree node row was selected.
    WorktreeNodeSelected {
        worktree_path: PathBuf,
        name: String,
    },
    /// Mark commit as "good" during bisect.
    BisectGood(git2::Oid),
    /// Mark commit as "bad" during bisect.
    BisectBad(git2::Oid),
    /// Open interactive rebase starting from (not including) the selected commit.
    /// All commits from HEAD down to the selected commit will be shown in the
    /// interactive rebase editor, allowing the user to reorder, squash, fixup,
    /// reword, or drop them.
    InteractiveRebase(git2::Oid),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorktreeGraphInfo {
    pub name: String,
    pub is_current: bool,
    pub head_oid: Option<git2::Oid>,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub combined_breakdown: HashMap<FileChangeKind, usize>,
    pub worktree_path: PathBuf,
    pub branch: Option<String>,
}

#[derive(Clone, Debug)]
struct WorktreeRowPosition {
    worktree_idx: usize,
    list_index: usize,
    commit_index: Option<usize>,
    node_lane: usize,
    color_index: usize,
}

/// State for the right-click context menu.
struct ContextMenuState {
    /// The index of the commit that was right-clicked.
    commit_index: usize,
    /// Screen position where the menu should appear.
    position: Point<Pixels>,
}

/// The commit graph panel.
pub struct GraphView {
    commits: Arc<Vec<CommitInfo>>,
    graph_rows: Arc<Vec<GraphRow>>,
    global_max_lane: usize,
    selected_index: Option<usize>,
    selected_oid: Option<git2::Oid>,
    row_height: f32,
    scroll_handle: UniformListScrollHandle,
    context_menu: Option<ContextMenuState>,
    show_search: bool,
    filter_matches: Vec<usize>,
    filter_match_set: HashSet<usize>,
    filter_match_set_arc: Arc<HashSet<usize>>,
    current_match: usize,
    search_editor: Entity<rgitui_ui::TextInput>,
    graph_focus: FocusHandle,
    all_commits_loaded: bool,
    /// OID to scroll to once the commit list is refreshed and the OID is present.
    /// Used when scroll_to_commit is called for a commit not yet in the loaded list.
    pending_scroll_oid: Option<git2::Oid>,
    /// Search query to re-run once more commits have been loaded.
    /// Used when a search returns no matches but more commits are available.
    pending_search_query: Option<SharedString>,
    cached_graph_hash: u64,
    search_debounce_task: Option<gpui::Task<()>>,
    worktree_infos: Vec<WorktreeGraphInfo>,
    worktree_row_positions: Vec<WorktreeRowPosition>,
    worktree_row_set: HashSet<usize>,
    virtual_rows_prefix: Vec<usize>,
    show_settings_popover: bool,
    /// SHA display length: 0 = default short (7), or specific length (7/8/10/12/40).
    sha_display_length: u8,
    show_author_column: bool,
    show_date_column: bool,
    show_absolute_dates: bool,
    show_avatars: bool,
    show_graph_lanes: bool,
    show_ref_badges: bool,
    show_author_email: bool,
    /// Whether "My Commits" filter is active — show only commits by the current user.
    my_commits_active: bool,
    /// Cached bounds of the graph container div, used to convert window-relative
    /// click positions to container-relative coordinates for context menu placement.
    container_bounds: Bounds<Pixels>,
    /// OID of the commit currently being dragged (for drag-to-rebase).
    dragging_oid: Option<git2::Oid>,
    /// Whether a drag-to-rebase was just completed (suppresses click-to-select).
    suppress_next_click: bool,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl GraphView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_editor = cx.new(|cx| {
            let mut ti = rgitui_ui::TextInput::new(cx);
            ti.set_placeholder("Search commits...");
            ti
        });

        cx.subscribe(
            &search_editor,
            |this: &mut Self, _, event: &rgitui_ui::TextInputEvent, cx| match event {
                rgitui_ui::TextInputEvent::Changed(_) => {
                    this.schedule_search_filter(cx);
                }
                rgitui_ui::TextInputEvent::Submit => {
                    this.jump_to_next_match(cx);
                }
            },
        )
        .detach();

        Self {
            commits: Arc::new(Vec::new()),
            graph_rows: Arc::new(Vec::new()),
            global_max_lane: 0,
            selected_index: None,
            selected_oid: None,
            row_height: 32.0,
            scroll_handle: UniformListScrollHandle::new(),
            context_menu: None,
            show_search: false,
            filter_matches: Vec::new(),
            filter_match_set: HashSet::new(),
            filter_match_set_arc: Arc::new(HashSet::new()),
            current_match: 0,
            search_editor,
            graph_focus: cx.focus_handle(),
            all_commits_loaded: false,
            pending_scroll_oid: None,
            pending_search_query: None,
            cached_graph_hash: 0,
            search_debounce_task: None,
            worktree_infos: Vec::new(),
            worktree_row_positions: Vec::new(),
            worktree_row_set: HashSet::new(),
            virtual_rows_prefix: Vec::new(),
            show_settings_popover: false,
            sha_display_length: 0,
            show_author_column: true,
            show_date_column: true,
            show_absolute_dates: false,
            show_avatars: true,
            show_graph_lanes: true,
            show_ref_badges: true,
            show_author_email: false,
            my_commits_active: false,
            container_bounds: Bounds::new(Point::new(px(0.), px(0.)), Size::new(px(0.), px(0.))),
            dragging_oid: None,
            suppress_next_click: false,
        }
    }

    /// Focus the graph view for keyboard navigation.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.graph_focus.focus(window, cx);
        cx.notify();
    }

    /// Check if the graph view is currently focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.graph_focus.is_focused(window)
    }

    /// Whether "My Commits" filter is currently active.
    pub fn my_commits_active(&self) -> bool {
        self.my_commits_active
    }

    pub fn set_commits(&mut self, commits: Arc<Vec<CommitInfo>>, cx: &mut Context<Self>) {
        // Compute a simple hash to detect if commits actually changed
        let graph_style = cx.global::<SettingsState>().settings().graph_style;
        let new_hash = Self::compute_commits_hash(&commits, &graph_style);
        if new_hash == self.cached_graph_hash && !self.commits.is_empty() {
            log::debug!(
                "GraphView::set_commits: hash unchanged ({:#x}), skipping ({} commits)",
                new_hash,
                commits.len()
            );
            return;
        }
        self.cached_graph_hash = new_hash;
        log::debug!(
            "GraphView::set_commits: hash changed -> {:#x}, spawning graph compute for {} commits",
            new_hash,
            commits.len()
        );

        // Preserve selection by OID across refreshes
        let prev_selected_oid = self.selected_oid;
        let prev_selected_index = self.selected_index;

        // Store commits immediately so other state (search, working tree row) stays in sync.
        // The old graph_rows remain in place until the background computation finishes,
        // so the UI renders the previous graph during computation rather than going blank.
        self.commits = commits.clone();
        self.recompute_worktree_positions();

        // Spawn graph computation on the background executor.
        let commits_for_bg = commits;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let graph_rows = cx
                .background_executor()
                .spawn(async move { compute_graph(&commits_for_bg) })
                .await;

            this.update(cx, |this: &mut GraphView, cx| {
                this.global_max_lane = graph_rows
                    .iter()
                    .map(|r| r.lane_count)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                this.graph_rows = Arc::new(graph_rows);
                log::debug!(
                    "GraphView: graph rows applied: {} rows, max_lane={}",
                    this.graph_rows.len(),
                    this.global_max_lane
                );
                this.recompute_worktree_positions();

                // Restore selection if the previously selected commit still exists
                if let Some(prev_oid) = prev_selected_oid {
                    if let Some(new_index) = this.commits.iter().position(|c| c.oid == prev_oid) {
                        this.selected_index = this.list_index_for_commit_index(new_index);
                    } else {
                        this.selected_index = None;
                        this.selected_oid = None;
                    }
                } else if prev_selected_index
                    .is_some_and(|index| this.worktree_row_set.contains(&index))
                {
                    if !prev_selected_index.is_some_and(|index| index < this.total_list_items()) {
                        this.selected_index = None;
                    } else {
                        this.selected_index = prev_selected_index;
                    }
                } else {
                    this.selected_index = None;
                }

                if this.show_search && !this.search_editor.read(cx).is_empty() {
                    this.update_search_filter(cx);
                }

                // Check if a pending scroll target has just been loaded.
                if let Some(pending_oid) = this.pending_scroll_oid {
                    if let Some(index) = this.commits.iter().position(|c| c.oid == pending_oid) {
                        if let Some(list_index) = this.list_index_for_commit_index(index) {
                            this.select_list_index(list_index, cx);
                            this.scroll_handle
                                .scroll_to_item(list_index, ScrollStrategy::Top);
                        }
                    }
                    if this.all_commits_loaded {
                        this.pending_scroll_oid = None;
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Mark that all available commits have been loaded (disables "load more").
    pub fn set_all_loaded(&mut self, loaded: bool) {
        self.all_commits_loaded = loaded;
    }

    /// Update the worktree node data shown in the graph.
    pub fn set_worktree_statuses(&mut self, infos: Vec<WorktreeGraphInfo>, cx: &mut Context<Self>) {
        let filtered: Vec<WorktreeGraphInfo> = infos
            .into_iter()
            .filter(|info| info.staged_count > 0 || info.unstaged_count > 0)
            .collect();
        self.worktree_infos = filtered;
        self.recompute_worktree_positions();
        cx.notify();
    }

    fn recompute_worktree_positions(&mut self) {
        self.worktree_row_positions.clear();
        self.worktree_row_set.clear();
        self.virtual_rows_prefix.clear();

        let visible_commit_count = self.commits.len().min(self.graph_rows.len());
        let mut anchored = Vec::new();
        let mut orphan_indices = Vec::new();

        for (worktree_idx, info) in self.worktree_infos.iter().enumerate() {
            match info.head_oid {
                Some(oid) => {
                    if let Some(commit_index) = self.commits.iter().position(|c| c.oid == oid) {
                        if let Some(graph_row) = self.graph_rows.get(commit_index) {
                            anchored.push((
                                commit_index,
                                WorktreeRowPosition {
                                    worktree_idx,
                                    list_index: 0,
                                    commit_index: Some(commit_index),
                                    node_lane: graph_row.node_lane,
                                    color_index: graph_row.node_color,
                                },
                            ));
                        } else if commit_index < visible_commit_count {
                            anchored.push((
                                commit_index,
                                WorktreeRowPosition {
                                    worktree_idx,
                                    list_index: 0,
                                    commit_index: Some(commit_index),
                                    node_lane: 0,
                                    color_index: 0,
                                },
                            ));
                        }
                    }
                }
                None => orphan_indices.push(worktree_idx),
            }
        }

        // Sort anchored worktrees: current/main worktree first (so it gets
        // list_index 0 and wins lane collisions), then by commit_index.
        anchored.sort_by(|(ci_a, pos_a), (ci_b, pos_b)| {
            let a_current = self.worktree_infos[pos_a.worktree_idx].is_current;
            let b_current = self.worktree_infos[pos_b.worktree_idx].is_current;
            b_current
                .cmp(&a_current)
                .then_with(|| ci_a.cmp(ci_b))
        });

        // Detect lane collisions among anchored worktree rows and assign
        // unique lanes so that two worktrees whose HEAD commits share a lane
        // don't overlap visually. The current worktree is first so it keeps
        // its natural lane (lane 0 for main).
        let mut used_lanes: HashSet<usize> = HashSet::new();
        let mut next_extra_lane = self.global_max_lane + 1;
        for (_commit_index, position) in anchored.iter_mut() {
            if !used_lanes.insert(position.node_lane) {
                // Collision — assign a fresh lane beyond the current max.
                position.node_lane = next_extra_lane;
                position.color_index = next_extra_lane;
                next_extra_lane += 1;
            }
        }

        for (list_index, worktree_idx) in orphan_indices.into_iter().enumerate() {
            self.worktree_row_positions.push(WorktreeRowPosition {
                worktree_idx,
                list_index,
                commit_index: None,
                node_lane: next_extra_lane,
                color_index: 0,
            });
            next_extra_lane += 1;
        }

        // Re-sort by commit_index ascending for list_index placement.
        // (The earlier sort was by is_current-first for lane collision priority,
        // but placement must follow commit order to avoid list_index collisions.)
        anchored.sort_by_key(|(commit_index, _)| *commit_index);

        // Place virtual rows immediately above their HEAD commit.
        let mut virtual_rows_before = self.worktree_row_positions.len(); // orphans already placed
        for (commit_index, mut position) in anchored {
            position.list_index = commit_index + virtual_rows_before;
            self.worktree_row_positions.push(position);
            virtual_rows_before += 1;
        }

        self.worktree_row_positions
            .sort_by_key(|position| position.list_index);
        for position in &self.worktree_row_positions {
            self.worktree_row_set.insert(position.list_index);
        }

        let total_items = visible_commit_count + self.worktree_row_positions.len();
        self.virtual_rows_prefix = vec![0; total_items];
        let mut virtual_count = 0;
        for list_index in 0..total_items {
            if self.worktree_row_set.contains(&list_index) {
                virtual_count += 1;
            }
            self.virtual_rows_prefix[list_index] = virtual_count;
        }
    }

    fn worktree_row_at_list_index(&self, list_index: usize) -> Option<&WorktreeRowPosition> {
        self.worktree_row_positions
            .iter()
            .find(|position| position.list_index == list_index)
    }

    fn commit_index_for_list_index(&self, list_index: usize) -> Option<usize> {
        if self.worktree_row_set.contains(&list_index) {
            return None;
        }
        self.virtual_rows_prefix
            .get(list_index)
            .map(|virtual_count| list_index - *virtual_count)
    }

    fn list_index_for_commit_index(&self, commit_index: usize) -> Option<usize> {
        if commit_index >= self.commits.len().min(self.graph_rows.len()) {
            return None;
        }
        let virtual_rows_before = self
            .worktree_row_positions
            .iter()
            .enumerate()
            .filter(|(ordinal, position)| {
                position.list_index.saturating_sub(*ordinal) <= commit_index
            })
            .count();
        Some(commit_index + virtual_rows_before)
    }

    /// Total number of list items (virtual worktree rows + commits).
    /// Uses the minimum of commits and graph_rows to avoid index-out-of-bounds
    /// while graph computation is still running on the background thread.
    fn total_list_items(&self) -> usize {
        self.commits.len().min(self.graph_rows.len()) + self.worktree_row_positions.len()
    }

    pub fn selected_commit(&self) -> Option<&CommitInfo> {
        self.selected_index.and_then(|i| {
            self.commit_index_for_list_index(i)
                .and_then(|commit_index| self.commits.get(commit_index))
        })
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Total number of rows in the list (includes the virtual working tree row).
    pub fn row_count(&self) -> usize {
        self.total_list_items()
    }

    fn dismiss_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.is_some() {
            self.context_menu = None;
            cx.notify();
        }
    }

    fn show_context_menu(&mut self, index: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some(ContextMenuState {
            commit_index: index,
            position,
        });
        cx.notify();
    }

    // ── Drag-to-rebase ────────────────────────────────────────────────

    /// Begin dragging a commit (called on mousedown of the grip handle).
    fn start_drag(&mut self, oid: git2::Oid, cx: &mut Context<Self>) {
        self.dragging_oid = Some(oid);
        self.suppress_next_click = true;
        cx.notify();
    }

    /// Complete the drag and emit InteractiveRebase for the dragged commit.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(oid) = self.dragging_oid else {
            return;
        };
        self.dragging_oid = None;
        cx.emit(GraphViewEvent::InteractiveRebase(oid));
        cx.notify();
    }

    /// Scroll to the commit with the given OID, selecting it and emitting CommitSelected.
    pub fn scroll_to_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) {
        if let Some(index) = self.commits.iter().position(|c| c.oid == oid) {
            if let Some(list_index) = self.list_index_for_commit_index(index) {
                self.select_list_index(list_index, cx);
                self.scroll_handle
                    .scroll_to_item(list_index, ScrollStrategy::Top);
            }
        } else if !self.all_commits_loaded {
            // Commit not in loaded list — set as pending and trigger loading.
            self.pending_scroll_oid = Some(oid);
            cx.emit(GraphViewEvent::LoadMoreCommits);
        }
    }

    /// Select an item by its index in the uniform list.
    /// This is the public API used by external callers (workspace vim-key navigation).
    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        self.select_list_index(index, cx);
    }

    /// Select an item by its index in the uniform list (accounts for working tree row).
    fn select_list_index(&mut self, list_index: usize, cx: &mut Context<Self>) {
        let total = self.total_list_items();
        if list_index >= total {
            return;
        }
        self.selected_index = Some(list_index);
        if let Some(worktree_idx) = self
            .worktree_row_at_list_index(list_index)
            .map(|position| position.worktree_idx)
        {
            self.selected_oid = None;
            if let Some(worktree_info) = self.worktree_infos.get(worktree_idx) {
                cx.emit(GraphViewEvent::WorktreeNodeSelected {
                    worktree_path: worktree_info.worktree_path.clone(),
                    name: worktree_info.name.clone(),
                });
            }
        } else if let Some(commit_index) = self.commit_index_for_list_index(list_index) {
            if let Some(commit) = self.commits.get(commit_index) {
                self.selected_oid = Some(commit.oid);
                cx.emit(GraphViewEvent::CommitSelected(commit.oid));
            }
        }
        cx.notify();
    }

    /// Toggle the search bar visibility. Clears query when hiding.
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if !self.show_search {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
            self.filter_match_set_arc = Arc::new(HashSet::new());
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Toggle search with window access (focuses the search input).
    pub fn toggle_search_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if self.show_search {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
        } else {
            self.search_editor
                .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
            self.filter_matches.clear();
            self.filter_match_set.clear();
            self.filter_match_set_arc = Arc::new(HashSet::new());
            self.current_match = 0;
        }
        cx.notify();
    }

    /// Returns whether the search bar is currently visible.
    pub fn is_search_visible(&self) -> bool {
        self.show_search
    }

    /// Compute a simple hash of the commit list for change detection.
    fn compute_commits_hash(
        commits: &[CommitInfo],
        graph_style: &rgitui_settings::GraphStyle,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        commits.len().hash(&mut hasher);
        for c in commits {
            c.oid.as_bytes().hash(&mut hasher);
            c.refs.len().hash(&mut hasher);
            for r in &c.refs {
                r.display_name().hash(&mut hasher);
            }
        }
        graph_style.hash(&mut hasher);
        hasher.finish()
    }

    /// Update the filter_matches list based on the current search_query.
    /// Matches case-insensitively against commit message, author name, author email, and short SHA.
    fn schedule_search_filter(&mut self, cx: &mut Context<Self>) {
        self.search_debounce_task = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            smol::Timer::after(std::time::Duration::from_millis(150)).await;
            cx.update(|cx| {
                let _ = this.update(cx, |this, cx| {
                    this.update_search_filter(cx);
                    this.jump_to_first_match(cx);
                    cx.notify();
                });
            });
        }));
    }

    fn update_search_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_matches.clear();
        self.filter_match_set.clear();
        self.current_match = 0;

        if self.search_editor.read(cx).is_empty() {
            self.filter_match_set_arc = Arc::new(HashSet::new());
            self.pending_search_query = None;
            return;
        }

        let query = self.search_editor.read(cx).text().to_lowercase();
        for (i, commit) in self.commits.iter().enumerate() {
            if commit.summary.to_lowercase().contains(&query)
                || commit.message.to_lowercase().contains(&query)
                || commit.author.name.to_lowercase().contains(&query)
                || commit.author.email.to_lowercase().contains(&query)
                || commit.short_id.to_lowercase().contains(&query)
            {
                self.filter_matches.push(i);
                self.filter_match_set.insert(i);
            }
        }
        self.filter_match_set_arc = Arc::new(self.filter_match_set.clone());
        log::debug!(
            "GraphView::search: query={:?} matches={}",
            query,
            self.filter_matches.len()
        );

        // If no matches but more commits are available, auto-load more.
        if self.filter_matches.is_empty() && !self.all_commits_loaded {
            self.pending_search_query = Some(query.into());
            cx.emit(GraphViewEvent::LoadMoreCommits);
        } else {
            // Matches found, or no more commits to load — clear any pending search.
            self.pending_search_query = None;
        }
    }

    /// Jump to the next search match, selecting and scrolling to it.
    fn jump_to_next_match(&mut self, cx: &mut Context<Self>) {
        if self.filter_matches.is_empty() {
            return;
        }
        self.current_match = (self.current_match + 1) % self.filter_matches.len();
        let commit_index = self.filter_matches[self.current_match];
        if let Some(list_index) = self.list_index_for_commit_index(commit_index) {
            self.select_list_index(list_index, cx);
            self.scroll_handle
                .scroll_to_item(list_index, ScrollStrategy::Top);
        }
    }

    /// Jump to the previous search match.
    fn jump_to_prev_match(&mut self, cx: &mut Context<Self>) {
        if self.filter_matches.is_empty() {
            return;
        }
        if self.current_match == 0 {
            self.current_match = self.filter_matches.len() - 1;
        } else {
            self.current_match -= 1;
        }
        let commit_index = self.filter_matches[self.current_match];
        if let Some(list_index) = self.list_index_for_commit_index(commit_index) {
            self.select_list_index(list_index, cx);
            self.scroll_handle
                .scroll_to_item(list_index, ScrollStrategy::Top);
        }
    }

    /// Jump to first match after updating the search filter.
    fn jump_to_first_match(&mut self, cx: &mut Context<Self>) {
        if !self.filter_matches.is_empty() {
            self.current_match = 0;
            let commit_index = self.filter_matches[0];
            if let Some(list_index) = self.list_index_for_commit_index(commit_index) {
                self.select_list_index(list_index, cx);
                self.scroll_handle
                    .scroll_to_item(list_index, ScrollStrategy::Top);
            }
        }
    }

    /// Handle key events on the focused search input.
    fn handle_graph_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .search_editor
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            if event.keystroke.key.as_str() == "escape" {
                // Cancel any in-progress drag-to-rebase.
                self.dragging_oid = None;
                self.suppress_next_click = false;
                self.show_search = false;
                self.search_editor
                    .update(cx, |e: &mut rgitui_ui::TextInput, cx| e.clear(cx));
                self.filter_matches.clear();
                self.filter_match_set.clear();
                self.filter_match_set_arc = Arc::new(HashSet::new());
                self.current_match = 0;
                self.graph_focus.focus(window, cx);
                cx.notify();
            }
            return;
        }
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        let total = self.total_list_items();
        match key {
            "j" | "down" if !ctrl => {
                let next = match self.selected_index {
                    Some(i) if i + 1 < total => i + 1,
                    None if total > 0 => 0,
                    _ => return,
                };
                self.select_list_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "k" | "up" if !ctrl => {
                let next = match self.selected_index {
                    Some(i) if i > 0 => i - 1,
                    None if total > 0 => 0,
                    _ => return,
                };
                self.select_list_index(next, cx);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Center);
            }
            "g" if !ctrl && !keystroke.modifiers.shift
                && total > 0 => {
                    self.select_list_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            "g" if keystroke.modifiers.shift
                && total > 0 => {
                    let last = total - 1;
                    self.select_list_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            "end"
                if total > 0 => {
                    let last = total - 1;
                    self.select_list_index(last, cx);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Center);
                }
            "home"
                if total > 0 => {
                    self.select_list_index(0, cx);
                    self.scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                }
            "/" if !ctrl => {
                self.show_search = true;
                self.search_editor.update(cx, |e: &mut rgitui_ui::TextInput, cx| e.focus(window, cx));
                cx.notify();
            }
            "escape"
                // Dismiss context menu or deselect
                if self.context_menu.is_some() => {
                    self.dismiss_context_menu(cx);
                }
            "y" | "Y" if !ctrl && !keystroke.modifiers.shift => {
                // Copy SHA of selected commit (standard GitKraken shortcut)
                if let Some(commit) = self.selected_commit() {
                    let sha = format!("{}", commit.oid);
                    cx.emit(GraphViewEvent::CopyCommitSha(sha));
                }
            }
            _ => {}
        }
    }
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        log::trace!(
            "GraphView::render: commits={} graph_rows={} selected={:?}",
            self.commits.len(),
            self.graph_rows.len(),
            self.selected_index
        );
        let colors = cx.colors();

        if self.total_list_items() == 0 {
            return div()
                .id("graph-view")
                .v_flex()
                .size_full()
                .bg(colors.panel_background)
                // Header bar — consistent with other panels
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .pl(px(10.))
                        .pr(px(8.))
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
                            Label::new("Graph")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    div().flex_1().flex().items_center().justify_center().child(
                        div()
                            .v_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::GitCommit)
                                    .size(IconSize::Medium)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("No commits to display")
                                    .color(Color::Muted)
                                    .size(LabelSize::Small),
                            ),
                    ),
                )
                .into_any_element();
        }

        // Extract colors before closure (can't call cx inside uniform_list closure)
        let selected_bg = colors.element_selected;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;
        let panel_bg = colors.panel_background;
        let border_color = colors.border_variant;
        let accent_border = colors.border_focused;
        let selected_border = colors.border_focused;

        // Search highlight colors (derived from theme accent with adjusted alpha)
        let search_match_bg = gpui::Hsla {
            a: 0.12,
            ..colors.text_accent
        };
        let search_current_bg = gpui::Hsla {
            a: 0.28,
            ..colors.text_accent
        };

        // HEAD emphasis: accent-tinted background for the HEAD row
        let head_row_bg = gpui::Hsla {
            a: 0.08,
            ..colors.text_accent
        };

        // Working tree row color (warning/yellow tint)
        let status_colors = cx.status();
        let working_tree_bg = gpui::Hsla {
            a: 0.06,
            ..status_colors.warning
        };
        let working_tree_border_color = gpui::Hsla {
            a: 0.6,
            ..status_colors.warning
        };
        let working_tree_node_color = status_colors.warning;

        // Cheap Arc clones
        let commits = self.commits.clone();
        let graph_rows = self.graph_rows.clone();
        let selected_index = self.selected_index;
        let worktree_infos = self.worktree_infos.clone();
        let worktree_row_positions = self.worktree_row_positions.clone();
        let worktree_row_set = self.worktree_row_set.clone();
        let virtual_rows_prefix = self.virtual_rows_prefix.clone();
        let view: WeakEntity<GraphView> = cx.weak_entity();

        let total_list_items = self.total_list_items();

        // Search state for the render closure — use pre-computed Arc for O(1) clone
        let filter_match_set = Arc::clone(&self.filter_match_set_arc);
        let current_match_index = if self.filter_matches.is_empty() {
            None
        } else {
            Some(self.filter_matches[self.current_match])
        };
        let has_search_query = self.show_search && !self.search_editor.read(cx).is_empty();
        let has_context_menu = self.context_menu.is_some();

        let lane_width: f32 = 20.0;
        let graph_padding_left: f32 = 10.0;
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let graph_style = cx.global::<SettingsState>().settings().graph_style;
        let compact_mul = compactness.multiplier();
        let row_height = compactness.spacing(self.row_height);
        let worktree_max_lane = worktree_row_positions
            .iter()
            .map(|position| position.node_lane)
            .max()
            .unwrap_or(0);
        let graph_lane_count = self.global_max_lane.max(worktree_max_lane + 1);

        let graph_col_width =
            ((graph_lane_count as f32 + 1.0) * lane_width + graph_padding_left).max(80.0);

        // Column visibility settings
        let show_author_column = self.show_author_column;
        let show_date_column = self.show_date_column;
        let show_absolute_dates = self.show_absolute_dates;
        let sha_display_length = self.sha_display_length;
        let show_avatars = self.show_avatars;
        let show_graph_lanes = self.show_graph_lanes;
        let show_ref_badges = self.show_ref_badges;
        let show_subject_column = cx.global::<SettingsState>().settings().show_subject_column;
        let author_col_width = cx.global::<SettingsState>().settings().author_column_width;
        let date_col_width = cx.global::<SettingsState>().settings().date_column_width;
        let show_author_email = self.show_author_email;
        let my_commits_active = self.my_commits_active;

        // Helper to get global position
        let entity = cx.entity();

        // Header row (not virtualized — always visible)
        let view_settings_toggle = cx.weak_entity();
        let view_my_commits_toggle = cx.weak_entity();
        let mut header = div()
            .h_flex()
            .items_center()
            .w_full()
            .h(px(26.))
            .pl(px(8.))
            .pr(px(8.))
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(border_color)
            .child(
                canvas(
                    {
                        let entity = entity.clone();
                        move |bounds, _, cx| {
                            entity.update(cx, |this, _| this.container_bounds = bounds);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_drag_move::<AuthorColumnResize>(cx.listener(
                move |this, e: &gpui::DragMoveEvent<AuthorColumnResize>, _, cx| {
                    let settings = cx.global_mut::<SettingsState>();
                    let right_edge = this.container_bounds.right()
                        - px(22.0 + 8.0 + 12.0)
                        - px(date_col_width)
                        - px(12.0);
                    let new_w = f32::from(e.event.position.x - (right_edge - px(author_col_width)))
                        .clamp(50., 600.);
                    settings.settings_mut().author_column_width = new_w;
                    cx.notify();
                },
            ))
            .on_drag_move::<DateColumnResize>(cx.listener(
                move |this, e: &gpui::DragMoveEvent<DateColumnResize>, _, cx| {
                    let settings = cx.global_mut::<SettingsState>();
                    let right_edge = this.container_bounds.right() - px(22.0 + 8.0);
                    let new_w = f32::from(e.event.position.x - (right_edge - px(date_col_width)))
                        .clamp(50., 300.);
                    settings.settings_mut().date_column_width = new_w;
                    cx.notify();
                },
            ))
            .when(show_graph_lanes, |el| {
                el.child(
                    div()
                        .w(px(graph_col_width))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .h_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Graph")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
            })
            .child(
                div().w(px(80.)).flex_shrink_0().child(
                    Label::new("Hash")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .weight(gpui::FontWeight::SEMIBOLD),
                ),
            )
            .when(show_subject_column, |el| {
                el.child(
                    div().flex_1().child(
                        Label::new("Message")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
                )
            });

        if show_author_column {
            let author_width = if self.show_author_email {
                author_col_width.max(180.0)
            } else {
                author_col_width
            };
            header = header.child(
                div()
                    .relative()
                    .w(px(author_width))
                    .flex_shrink_0()
                    .ml(px(12.))
                    .px(px(4.))
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        Icon::new(IconName::User)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Author")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        div()
                            .id("author-resize-handle")
                            .absolute()
                            .right(px(-4.))
                            .w(px(8.))
                            .h_full()
                            .cursor_col_resize()
                            .on_drag(AuthorColumnResize, |val, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| val.clone())
                            })
                            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                cx.stop_propagation()
                            }),
                    ),
            );
        }

        if show_date_column {
            header = header.child(
                div()
                    .relative()
                    .w(px(date_col_width))
                    .flex_shrink_0()
                    .ml(px(12.))
                    .mr(px(8.))
                    .px(px(4.))
                    .h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        Icon::new(IconName::Clock)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Date")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        div()
                            .id("date-resize-handle")
                            .absolute()
                            .right(px(-4.))
                            .w(px(8.))
                            .h_full()
                            .cursor_col_resize()
                            .on_drag(DateColumnResize, |val, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| val.clone())
                            })
                            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                cx.stop_propagation()
                            }),
                    ),
            );
        }

        let my_commits_icon_color = if my_commits_active {
            Color::Accent
        } else {
            Color::Muted
        };

        let my_commits_tooltip: SharedString = if my_commits_active {
            "Showing only your commits. Click to show all commits.".into()
        } else {
            "Show only your commits. Click to filter by current user.".into()
        };

        header = header
            .child(
                div()
                    .id("my-commits-btn")
                    .flex_shrink_0()
                    .w(px(22.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(hover_bg))
                    .active(|s| s.bg(active_bg))
                    .tooltip(Tooltip::text(my_commits_tooltip))
                    .on_mouse_down(
                        MouseButton::Left,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                        view_my_commits_toggle
                            .update(cx, |this: &mut GraphView, cx| {
                                this.my_commits_active = !this.my_commits_active;
                                cx.emit(GraphViewEvent::ToggleMyCommits);
                                cx.notify();
                            })
                            .ok();
                    })
                    .child(
                        Icon::new(IconName::User)
                            .size(IconSize::XSmall)
                            .color(my_commits_icon_color),
                    ),
            )
            .child(
                div()
                    .id("settings-gear-btn")
                    .flex_shrink_0()
                    .w(px(22.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(hover_bg))
                    .active(|s| s.bg(active_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                        view_settings_toggle
                            .update(cx, |this: &mut GraphView, cx| {
                                this.show_settings_popover = !this.show_settings_popover;
                                cx.notify();
                            })
                            .ok();
                    })
                    .child(
                        Icon::new(IconName::Settings)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        let now_utc = chrono::Utc::now();

        // The virtualized list
        // Capture drag state for drag-to-rebase grip rendering.
        let dragging_oid = self.dragging_oid;
        let list = uniform_list(
            "graph-commit-list",
            total_list_items,
            move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                range
                    .map(|i| {
                        if let Some(position) = worktree_row_positions
                            .iter()
                            .find(|position| position.list_index == i)
                        {
                            let info = &worktree_infos[position.worktree_idx];
                            let is_orphan = info.head_oid.is_none();
                            let row_node_color = if info.is_current || is_orphan {
                                working_tree_node_color
                            } else {
                                rgitui_theme::lane_color(position.color_index)
                            };
                            let row_bg = if info.is_current || is_orphan {
                                working_tree_bg
                            } else {
                                gpui::Hsla {
                                    a: 0.06,
                                    ..row_node_color
                                }
                            };
                            let row_border_color = if info.is_current || is_orphan {
                                working_tree_border_color
                            } else {
                                gpui::Hsla {
                                    a: 0.6,
                                    ..row_node_color
                                }
                            };
                            // The worktree virtual row sits immediately above its
                            // HEAD commit. We need to draw pass-through lines for
                            // other branches' lanes that cross this row vertically,
                            // otherwise there's a visible gap in those lines.
                            // We use the HEAD commit's straight pass-through edges,
                            // excluding the worktree's own node lane.
                            let wt_node_lane = position.node_lane;
                            let (has_head_incoming, pass_through_edges) = position
                                .commit_index
                                .and_then(|ci| graph_rows.get(ci))
                                .map(|gr| {
                                    (
                                        gr.has_incoming,
                                        gr.edges
                                            .iter()
                                            .filter(|edge| {
                                                edge.from_lane == edge.to_lane
                                                    && edge.from_lane != gr.node_lane
                                                    && edge.from_lane != wt_node_lane
                                            })
                                            .cloned()
                                            .collect(),
                                    )
                                })
                                .unwrap_or_else(|| (false, Vec::new()));

                            return render_working_tree_row(WorkingTreeRowParams {
                                list_index: i,
                                selected: selected_index == Some(i),
                                staged_count: info.staged_count,
                                unstaged_count: info.unstaged_count,
                                combined_breakdown: info.combined_breakdown.clone(),
                                worktree_name: info.name.clone(),
                                branch_name: info.branch.clone(),
                                is_current_worktree: info.is_current,
                                is_orphan_worktree: is_orphan,
                                has_head_incoming,
                                pass_through_edges,
                                row_height,
                                lane_width,
                                graph_padding_left,
                                graph_col_width,
                                working_tree_bg: row_bg,
                                working_tree_border_color: row_border_color,
                                node_color: row_node_color,
                                selected_bg,
                                hover_bg,
                                active_bg,
                                panel_bg,
                                selected_border,
                                view: view.clone(),
                                show_author_column,
                                show_date_column,
                                author_col_width,
                                date_col_width,
                                show_absolute_dates,
                                show_graph_lanes,
                                compact_mul,
                                has_context_menu,
                                head_node_lane: position.node_lane,
                            });
                        }

                        let vr_count = virtual_rows_prefix.get(i).copied().unwrap_or(0);
                        let commit_idx = i - vr_count;
                        if commit_idx >= commits.len() || commit_idx >= graph_rows.len() {
                            return div().into_any_element();
                        }
                        let commit = &commits[commit_idx];
                        let oid = commit.oid;
                        let graph_row = &graph_rows[commit_idx];
                        let selected = selected_index == Some(i);
                        let is_current_match = current_match_index == Some(commit_idx);
                        let is_any_match = has_search_query && filter_match_set.contains(&commit_idx);
                        let is_head_row = graph_row.is_head;
                        let is_merge_commit = graph_row.is_merge;

                        let row_base_bg = if is_head_row {
                            head_row_bg
                        } else {
                            panel_bg
                        };
                        let bg = if selected {
                            selected_bg
                        } else if is_current_match {
                            search_current_bg
                        } else if is_any_match {
                            search_match_bg
                        } else {
                            row_base_bg
                        };
                        let row_hover_bg = if selected { selected_bg } else { hover_bg };
                        let row_active_bg = if selected { selected_bg } else { active_bg };

                        let graph_width = graph_col_width;
                        let node_lane = graph_row.node_lane;
                        let node_x = node_lane as f32 * lane_width + lane_width / 2.0 + graph_padding_left;

                        let node_color = rgitui_theme::lane_color(graph_row.node_color);
                        let has_virtual_row_above =
                            i > 0 && worktree_row_set.contains(&(i - 1));
                        let has_incoming = graph_row.has_incoming || has_virtual_row_above;

                        // Author initials for avatar (extract small strings, not the whole CommitInfo)
                        let initials: SharedString = commit
                            .author
                            .name
                            .split_whitespace()
                            .take(2)
                            .filter_map(|w| w.chars().next())
                            .collect::<String>()
                            .to_uppercase()
                            .into();

                        // Ref badges — distinct styling per ref type
                        let mut ref_badges: Vec<Badge> = Vec::new();
                        for r in &commit.refs {
                            let badge = match r {
                                RefLabel::Head => Badge::new("HEAD").color(Color::Warning).bold(),
                                RefLabel::LocalBranch(name) => {
                                    Badge::new(name.clone()).color(Color::Success)
                                }
                                RefLabel::RemoteBranch(name) => {
                                    Badge::new(name.clone()).color(Color::Info).italic()
                                }
                                RefLabel::Tag(name) => {
                                    Badge::new(name.clone()).color(Color::Accent).prefix("tag")
                                }
                            };
                            ref_badges.push(badge);
                        }

                        let hash_display: SharedString = match sha_display_length {
                            0 => commit.short_id.clone().into(),
                            40 => format!("{}", commit.oid).into(),
                            n => {
                                let full = format!("{}", commit.oid);
                                full[..((n as usize).min(full.len()))].to_string().into()
                            }
                        };
                        let summary: SharedString = commit.summary.clone().into();
                        let author: SharedString = commit.author.name.clone().into();
                        let author_email = commit.author.email.clone();
                        let time_str: SharedString = if show_absolute_dates {
                            commit.time.format("%b %d").to_string().into()
                        } else {
                            format_relative_time(&commit.time, now_utc).into()
                        };

                        // Clone only the edges vec for canvas closure (not the entire GraphRow)
                        let edges: Vec<GraphEdge> = graph_row.edges.clone();
                        let row_bg_for_canvas = bg;

                        let view_clone = view.clone();
                        let view_ctx_menu = view.clone();

                        let left_tab_color = if selected {
                            selected_border
                        } else if is_head_row {
                            gpui::Hsla {
                                a: 0.8,
                                ..accent_border
                            }
                        } else {
                            gpui::Hsla {
                                a: 0.4,
                                ..node_color
                            }
                        };

                        let mut row = div()
                            .id(ElementId::NamedInteger("commit-row".into(), i as u64))
                            .h_flex()
                            .items_center()
                            .h(px(row_height))
                            .w_full()
                            .bg(bg)
                            .cursor(CursorStyle::PointingHand)
                            .when(!has_context_menu, |el| {
                                el.hover(move |s| s.bg(row_hover_bg))
                            })
                            .active(move |s| s.bg(row_active_bg))
                            .on_click(
                                move |event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view_clone
                                        .update(cx, |this, cx| {
                                            // Suppress click if a drag-to-rebase was just completed.
                                            if this.suppress_next_click {
                                                this.suppress_next_click = false;
                                                cx.notify();
                                                return;
                                            }
                                            this.dismiss_context_menu(cx);
                                            if event.click_count() >= 2 {
                                                // Double-click: checkout this commit
                                                cx.emit(GraphViewEvent::CheckoutCommit(oid));
                                            } else {
                                                this.select_list_index(i, cx);
                                            }
                                        })
                                        .ok();
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                move |event: &MouseDownEvent,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    view_ctx_menu
                                        .update(cx, |this, cx| {
                                            this.show_context_menu(commit_idx, event.position, cx);
                                        })
                                        .ok();
                                },
                            )
                            .child(
                                div()
                                    .w(px(3.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .bg(left_tab_color),
                            )
                            .child(div().w(px(5.)).flex_shrink_0());

                        // Graph column with canvas + avatar overlay
                        if show_graph_lanes {
                        row = row.child(
                            div()
                                .relative()
                                .w(px(graph_width))
                                .flex_shrink_0()
                                .h_full()
                                .child(
                                    canvas(
                                        |_bounds: Bounds<Pixels>,
                                         _window: &mut Window,
                                         _cx: &mut App| {},
                                        move |bounds: Bounds<Pixels>,
                                              _: (),
                                              window: &mut Window,
                                              _cx: &mut App| {
                                            let origin = bounds.origin;
                                            let h = bounds.size.height;
                                            let mid_y = px(row_height / 2.0);
                                            let node_x_px = px(node_x);

                                            // 1. Approach segment: incoming line from row above → dot center.
                                            if has_incoming {
                                                // When a worktree row sits immediately above this commit,
                                                // start at the row top so the branch line doesn't paint
                                                // over the worktree connector above it.
                                                let approach_top = if has_virtual_row_above {
                                                    origin.y
                                                } else {
                                                    origin.y - px(4.0)
                                                };
                                                let mut approach = PathBuilder::stroke(px(2.0));
                                                approach.move_to(point(
                                                    origin.x + node_x_px,
                                                    approach_top,
                                                ));
                                                approach.line_to(point(
                                                    origin.x + node_x_px,
                                                    origin.y + mid_y,
                                                ));
                                                if let Ok(built) = approach.build() {
                                                    window.paint_path(built, node_color);
                                                }
                                            }

                                            // 2. Draw all edges (pass-throughs + outgoing from dot).
                                            for edge in &edges {
                                                let from_x = px(
                                                    edge.from_lane as f32 * lane_width
                                                        + lane_width / 2.0
                                                        + graph_padding_left,
                                                );
                                                let to_x = px(
                                                    edge.to_lane as f32 * lane_width
                                                        + lane_width / 2.0
                                                        + graph_padding_left,
                                                );
                                                let color =
                                                    rgitui_theme::lane_color(edge.color_index);

                                                let start_y = if edge.from_lane == node_lane {
                                                    origin.y + mid_y
                                                } else {
                                                    origin.y - px(4.0)
                                                };
                                                let end_y = origin.y + h + px(4.0);

                                                let stroke_width = px(2.0);

                                                let edge_color = if edge.is_merge {
                                                    gpui::Hsla { a: 0.85, ..color }
                                                } else {
                                                    color
                                                };

                                                let mut path = PathBuilder::stroke(stroke_width);

                                                if from_x == to_x {
                                                    path.move_to(point(
                                                        origin.x + from_x,
                                                        start_y,
                                                    ));
                                                    path.line_to(point(
                                                        origin.x + to_x,
                                                        end_y,
                                                    ));
                                                } else if edge.from_lane != node_lane {
                                                    let fx = origin.x + from_x;
                                                    let tx = origin.x + to_x;
                                                    let horiz_y = origin.y + mid_y;

                                                    match graph_style {
                                                        GraphStyle::Curved => {
                                                            let ry = (horiz_y - start_y).max(px(1.0));
                                                            let rx = (from_x - to_x).abs().max(px(1.0));
                                                            let bend_y = horiz_y - ry;
                                                            let dir = if tx < fx { -1.0_f32 } else { 1.0_f32 };

                                                            path.move_to(point(fx, start_y));

                                                            for &(sin_a, one_minus_cos) in quarter_arc_offsets() {
                                                                let px_x = fx + rx * dir * sin_a;
                                                                let px_y = bend_y + ry * one_minus_cos;
                                                                path.line_to(point(px_x, px_y));
                                                            }
                                                        }
                                                        GraphStyle::Rails => {
                                                            path.move_to(point(fx, start_y));
                                                            path.line_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                        }
                                                        GraphStyle::Angular => {
                                                            path.move_to(point(fx, start_y));
                                                            path.line_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                        }
                                                    }
                                                } else {
                                                    let fx = origin.x + from_x;
                                                    let tx = origin.x + to_x;
                                                    let horiz_y = origin.y + mid_y;

                                                    match graph_style {
                                                        GraphStyle::Curved => {
                                                            let avail = end_y - horiz_y;
                                                            let r = avail.min(px(16.0)).max(px(1.0));
                                                            let dir = if tx < fx { -1.0_f32 } else { 1.0_f32 };

                                                            path.move_to(point(fx, horiz_y));
                                                            let horiz_end_x = tx - r * dir;
                                                            path.line_to(point(horiz_end_x, horiz_y));

                                                            for &(sin_a, one_minus_cos) in quarter_arc_offsets() {
                                                                let arc_x = horiz_end_x + r * dir * sin_a;
                                                                let arc_y = horiz_y + r * one_minus_cos;
                                                                path.line_to(point(arc_x, arc_y));
                                                            }

                                                            path.line_to(point(tx, end_y));
                                                        }
                                                        GraphStyle::Rails => {
                                                            path.move_to(point(fx, horiz_y));
                                                            let mid_point_y = horiz_y + (end_y - horiz_y) / 2.0;
                                                            path.line_to(point(fx, mid_point_y));
                                                            path.line_to(point(tx, mid_point_y));
                                                            path.line_to(point(tx, end_y));
                                                        }
                                                        GraphStyle::Angular => {
                                                            path.move_to(point(fx, horiz_y));
                                                            path.line_to(point(tx, horiz_y));
                                                            path.line_to(point(tx, end_y));
                                                        }
                                                    }
                                                }

                                                if let Ok(built_path) = path.build() {
                                                    window.paint_path(built_path, edge_color);
                                                }
                                            }

                                            // Node dot: HEAD=larger filled, merge=filled+ring, normal=filled
                                            let dot_radius = if is_head_row {
                                                5.5_f32 * compact_mul
                                            } else if is_merge_commit {
                                                5.0_f32 * compact_mul
                                            } else {
                                                4.0_f32 * compact_mul
                                            };
                                            let cx_x = origin.x + node_x_px;
                                            let cy_y = origin.y + mid_y;
                                            // Background ring to occlude lines passing behind the dot
                                            let ring_r = 14.0_f32 * compact_mul;
                                            if let Some(built_ring) = build_filled_circle(cx_x, cy_y, ring_r) {
                                                window.paint_path(built_ring, row_bg_for_canvas);
                                            }

                                            // HEAD commit: glow ring + filled circle
                                            if is_head_row {
                                                let glow_r = dot_radius + 4.0 * compact_mul;
                                                if let Some(built_glow) = build_stroked_circle(cx_x, cy_y, glow_r, px(2.5)) {
                                                    let glow_color = gpui::Hsla {
                                                        a: 0.35,
                                                        ..node_color
                                                    };
                                                    window.paint_path(built_glow, glow_color);
                                                }

                                                // Filled circle for HEAD
                                                if let Some(built_head) = build_filled_circle(cx_x, cy_y, dot_radius) {
                                                    window.paint_path(built_head, node_color);
                                                }
                                            } else if is_merge_commit {
                                                // Merge commit: filled circle with outer ring
                                                if let Some(built_merge) = build_filled_circle(cx_x, cy_y, dot_radius) {
                                                    window.paint_path(built_merge, node_color);
                                                }

                                                let outer_r = dot_radius + 2.5;
                                                if let Some(built_merge) = build_stroked_circle(cx_x, cy_y, outer_r, px(1.5)) {
                                                    let merge_ring_color = gpui::Hsla {
                                                        a: 0.6,
                                                        ..node_color
                                                    };
                                                    window.paint_path(
                                                        built_merge,
                                                        merge_ring_color,
                                                    );
                                                }
                                            } else {
                                                // Normal commit: filled circle
                                                if let Some(built_normal) = build_filled_circle(cx_x, cy_y, dot_radius) {
                                                    window.paint_path(built_normal, node_color);
                                                }
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                // Avatar overlay: resolved image or initials fallback
                                .when(show_avatars, |el| el.child({
                                    let avatar_url = cx
                                        .try_global::<AvatarCache>()
                                        .and_then(|cache| cache.avatar_url(&author_email))
                                        .map(|s| s.to_string());
                                    let initials_fallback = initials.clone();
                                    let fallback_color = node_color;

                                    let avatar_size = 24.0_f32 * compact_mul;
                                    let mut avatar_container = div()
                                        .absolute()
                                        .left(px(node_x - avatar_size / 2.0))
                                        .top(px((row_height - avatar_size) / 2.0))
                                        .w(px(avatar_size))
                                        .h(px(avatar_size))
                                        .rounded_full()
                                        .overflow_hidden()
                                        .bg(panel_bg)
                                        .flex()
                                        .items_center()
                                        .justify_center();

                                    if let Some(url) = avatar_url {
                                        let fb_initials = initials_fallback.clone();
                                        let fb_color = fallback_color;
                                        avatar_container = avatar_container.child(
                                            img(url)
                                                .rounded_full()
                                                .size_full()
                                                .object_fit(ObjectFit::Cover)
                                                .with_fallback(move || {
                                                    div()
                                                        .size_full()
                                                        .rounded_full()
                                                        .border_1()
                                                        .border_color(fb_color)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .text_color(fb_color)
                                                                .text_size(px(8.0))
                                                                .font_weight(
                                                                    gpui::FontWeight::BOLD,
                                                                )
                                                                .child(fb_initials.clone()),
                                                        )
                                                        .into_any_element()
                                                }),
                                        );
                                    } else {
                                        avatar_container = avatar_container
                                            .border_1()
                                            .border_color(fallback_color)
                                            .child(
                                                div()
                                                    .text_color(fallback_color)
                                                    .text_size(px(8.0))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .child(initials_fallback),
                                            );
                                    }

                                    avatar_container
                                })),
                        );
                        }

                        // ── Drag-to-rebase grip ──────────────────────────────────
                        let is_dragging_this = dragging_oid == Some(oid);
                        // Opacity: dim when dragging this row; subtle when not dragging; hidden when not.
                        let grip_opacity = if is_dragging_this { 0.3 } else { 0.0 };
                        let grip_tooltip = Tooltip::text("Drag to rebase");
                        let entity_for_grip = view.clone();
                        let oid_for_grip = oid;
                        row = row.child(
                            div()
                                .id(ElementId::NamedInteger("rebase-grip".into(), i as u64))
                                .w(px(16.))
                                .h_flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .cursor(CursorStyle::PointingHand)
                                .opacity(grip_opacity)
                                .hover(|s| s.opacity(0.7))
                                .tooltip(grip_tooltip)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                                        entity_for_grip
                                            .update(cx, |this, cx| {
                                                this.start_drag(oid_for_grip, cx);
                                            })
                                            .ok();
                                    },
                                ),
                        );

                        row = row.child(
                            div().w(px(80.)).flex_shrink_0().child(
                                Label::new(hash_display)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .weight(gpui::FontWeight::MEDIUM)
                                    .truncate(),
                            ),
                        );

                        if show_subject_column {
                            let mut message_col = div()
                                .flex_1()
                                .min_w_0()
                                .h_flex()
                                .items_center()
                                .gap(px(4.))
                                .overflow_x_hidden();

                            if show_ref_badges {
                                for badge in ref_badges {
                                    message_col = message_col.child(
                                        div().flex_shrink_0().child(badge),
                                    );
                                }
                            }

                            // GPG signed commit badge
                            if commit.is_signed {
                                message_col = message_col.child(
                                    div()
                                        .flex_shrink_0()
                                        .child(Badge::new("✓ Signed").color(Color::Success).bold()),
                                );
                            }

                            let summary_label = if is_head_row {
                                Label::new(summary)
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate()
                            } else {
                                Label::new(summary).size(LabelSize::Small).truncate()
                            };
                            message_col = message_col.child(summary_label);

                            row = row.child(message_col);
                        }

                        // Author column (conditional)
                        if show_author_column {
                            let author_display: SharedString = if show_author_email {
                                format!("{} <{}>", author, author_email).into()
                            } else {
                                author.clone()
                            };
                            let author_tooltip: SharedString = format!("{} <{}>", author, author_email).into();
                            row = row.child(
                                div()
                                    .id(ElementId::NamedInteger("graph-author".into(), i as u64))
                                    .w(px(if show_author_email { author_col_width.max(180.) } else { author_col_width }))
                                    .flex_shrink_0()
                                    .ml(px(12.))
                                    .px(px(4.))
                                    .h_flex()
                                    .items_center()
                                    .overflow_x_hidden()
                                    .tooltip(Tooltip::text(author_tooltip))
                                    .child(
                                        Label::new(author_display)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    ),
                            );
                        }

                        // Date column (conditional)
                        if show_date_column {
                            row = row.child(
                                div()
                                    .w(px(date_col_width))
                                    .flex_shrink_0()
                                    .ml(px(12.))
                                    .mr(px(8.))
                                    .px(px(4.))
                                    .h_flex()
                                    .items_center()
                                    .overflow_x_hidden()
                                    .child(
                                        Label::new(time_str)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    ),
                            );
                        }

                        // Spacer matching the gear icon width in the header
                        // so the flex-1 message column is the same width in
                        // both header and rows, keeping Author/Date aligned.
                        row = row.child(div().w(px(26.)).flex_shrink_0());

                        row.into_any_element()
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        // Track container bounds so we can convert window-relative click positions
        // to container-relative coordinates for context menu placement.
        let view_bounds = cx.weak_entity();
        let bounds_tracker = canvas(
            {
                let view_bounds = view_bounds.clone();
                move |bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                    view_bounds
                        .update(cx, |this: &mut GraphView, _| {
                            this.container_bounds = bounds;
                        })
                        .ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let mut container = div()
            .id("graph-view")
            .track_focus(&self.graph_focus)
            .key_context("GraphView")
            .on_key_down(cx.listener(Self::handle_graph_key_down))
            .relative()
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(panel_bg)
            .child(bounds_tracker)
            .on_mouse_down(MouseButton::Left, {
                let view_dismiss = cx.weak_entity();
                move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    view_dismiss
                        .update(cx, |this: &mut GraphView, cx| {
                            // Only dismiss if click is outside the context menu bounds.
                            // Menu dimensions: 200px wide, 280px tall (12 items), anchored at clamped_x/clamped_y.
                            let click_inside_menu = this.context_menu.as_ref().is_some_and(|cm| {
                                let menu_w: Pixels = px(200.);
                                let menu_h: Pixels = px(280.);
                                let x = event.position.x;
                                let y = event.position.y;
                                x >= cm.position.x
                                    && x < cm.position.x + menu_w
                                    && y >= cm.position.y
                                    && y < cm.position.y + menu_h
                            });
                            if !click_inside_menu {
                                this.dismiss_context_menu(cx);
                            }
                            if this.show_settings_popover {
                                this.show_settings_popover = false;
                                cx.notify();
                            }
                        })
                        .ok();
                }
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                // Focus the graph when clicked (for keyboard navigation)
                this.graph_focus.focus(window, cx);
                cx.notify();
            }))
            // Drag-to-rebase: track mouse moves while a commit drag is in progress.
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _cx| {
                // Drag hover tracking would require scroll offset — skip for now.
                // The drag still works: mousedown on grip → mouseup fires InteractiveRebase.
                let _ = this;
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                    if this.dragging_oid.is_some() {
                        this.end_drag(cx);
                    }
                }),
            )
            .child(header);

        // Search bar (shown when search is active)
        if self.show_search {
            let no_matches =
                !self.search_editor.read(cx).is_empty() && self.filter_matches.is_empty();
            let match_count_text: SharedString = if self.search_editor.read(cx).is_empty() {
                String::new().into()
            } else if no_matches {
                "No matches".into()
            } else {
                format!("{}/{}", self.current_match + 1, self.filter_matches.len()).into()
            };
            let match_count_color = if no_matches {
                Color::Warning
            } else {
                Color::Muted
            };

            let view_prev = cx.weak_entity();
            let view_next = cx.weak_entity();
            let has_matches = !self.filter_matches.is_empty();

            let search_bar = div()
                .id("search-bar-container")
                .on_click(|_: &ClickEvent, _, cx: &mut App| {
                    cx.stop_propagation();
                })
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx: &mut App| {
                    cx.stop_propagation();
                })
                .h_flex()
                .items_center()
                .w_full()
                .h(px(40.))
                .px_2()
                .gap_2()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(div().flex_1().child(self.search_editor.clone()))
                .child(
                    Label::new(match_count_text)
                        .size(LabelSize::XSmall)
                        .color(match_count_color),
                )
                .child(
                    div()
                        .id("search-prev")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .when(has_matches, |el| el.hover(move |s| s.bg(hover_bg)))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_prev
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.jump_to_prev_match(cx);
                                })
                                .ok();
                        })
                        .child(Icon::new(IconName::ChevronUp).size(IconSize::Small).color(
                            if has_matches {
                                Color::Default
                            } else {
                                Color::Muted
                            },
                        )),
                )
                .child(
                    div()
                        .id("search-next")
                        .cursor_pointer()
                        .rounded_sm()
                        .p(px(2.))
                        .when(has_matches, |el| el.hover(move |s| s.bg(hover_bg)))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_next
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.jump_to_next_match(cx);
                                })
                                .ok();
                        })
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(IconSize::Small)
                                .color(if has_matches {
                                    Color::Default
                                } else {
                                    Color::Muted
                                }),
                        ),
                );

            container = container.child(search_bar);
        }

        container = container.child(list);

        if !self.all_commits_loaded && !self.commits.is_empty() {
            let view_load = cx.weak_entity();
            container = container.child(
                div()
                    .id("load-more-row")
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .mt(px(4.))
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .bg(colors.surface_background)
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view_load
                            .update(cx, |_this, cx| {
                                cx.emit(GraphViewEvent::LoadMoreCommits);
                            })
                            .ok();
                    })
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .bg(colors.ghost_element_hover)
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .size(IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("Load more commits")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    ),
            );
        }

        // Context menu overlay
        if let Some(ref menu_state) = self.context_menu {
            if let Some(commit) = self.commits.get(menu_state.commit_index) {
                let oid = commit.oid;
                let sha = format!("{}", oid);
                let msg_clone = commit.message.clone();
                let author_name_clone = commit.author.name.clone();
                let date_clone = commit.time.format("%Y-%m-%d %H:%M:%S").to_string();
                let pos = menu_state.position;
                let weak = cx.weak_entity();
                let sha_clone = sha.clone();

                let menu_bg = colors.elevated_surface_background;
                let menu_border = colors.border;
                let menu_hover = colors.ghost_element_hover;
                let menu_active = colors.ghost_element_active;
                let menu_accent = colors.text_accent;

                let menu_items: Vec<(&str, IconName)> = vec![
                    ("Cherry-pick commit", IconName::GitCommit),
                    ("Revert commit", IconName::Undo),
                    ("Checkout commit", IconName::Check),
                    ("Create branch here", IconName::GitBranch),
                    ("Create tag here", IconName::Tag),
                    ("Mark as good (bisect)", IconName::Check),
                    ("Mark as bad (bisect)", IconName::X),
                    ("Reset to here", IconName::Trash),
                    ("Interactive Rebase", IconName::GitMerge),
                    ("Copy SHA", IconName::Copy),
                    ("Copy commit message", IconName::Edit),
                    ("Copy author name", IconName::User),
                    ("Copy date", IconName::Clock),
                    ("View on GitHub", IconName::ExternalLink),
                ];

                // Convert window-relative click position to container-relative coordinates,
                // then clamp to keep the menu within the container bounds.
                let menu_w = px(200.);
                let menu_h = px(330.);
                let container_bounds = self.container_bounds;
                // Convert click position from window coordinates to container-relative.
                let rel_x = pos.x - container_bounds.origin.x;
                let rel_y = pos.y - container_bounds.origin.y;
                // Clamp so menu stays within container.
                let max_x = container_bounds.size.width - menu_w;
                let max_y = container_bounds.size.height - menu_h;
                let clamped_x = rel_x.max(px(0.)).min(max_x);
                let clamped_y = rel_y.max(px(0.)).min(max_y);

                let mut menu = div()
                    .id("graph-context-menu")
                    .absolute()
                    .left(clamped_x)
                    .top(clamped_y)
                    .v_flex()
                    .min_w(px(200.))
                    .py(px(3.))
                    .bg(menu_bg)
                    .border_1()
                    .border_color(menu_border)
                    .rounded(px(6.))
                    .elevation_3(cx)
                    // Prevent left-click on menu from dismissing via container handler
                    .on_mouse_down(
                        MouseButton::Left,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    // Prevent right-click on menu from opening another menu
                    .on_mouse_down(
                        MouseButton::Right,
                        |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    // Prevent hover from leaking through to graph rows
                    .on_mouse_move(|_: &MouseMoveEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    });

                for (idx, (label_text, icon_name)) in menu_items.iter().enumerate() {
                    let label: SharedString = (*label_text).into();
                    let icon = *icon_name;

                    // Add separator before bisect options, before destructive "Reset",
                    // before Interactive Rebase, and before clipboard ops
                    if idx == 5 || idx == 7 || idx == 8 || idx == 12 {
                        menu = menu.child(
                            div()
                                .w_full()
                                .h(px(1.))
                                .my(px(2.))
                                .mx(px(8.))
                                .bg(colors.border_variant),
                        );
                    }

                    let mut item = div()
                        .id(ElementId::NamedInteger("ctx-action".into(), idx as u64))
                        .h_flex()
                        .w_full()
                        .h(px(26.))
                        .px(px(8.))
                        .mx(px(4.))
                        .gap(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(move |s| s.bg(menu_hover).border_l_2().border_color(menu_accent))
                        .active(move |s| s.bg(menu_active));

                    match idx {
                        0 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CherryPick(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        1 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::RevertCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        2 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CheckoutCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        3 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CreateBranchAtCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        4 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CreateTagAtCommit(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        5 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::BisectGood(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        6 => {
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::BisectBad(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        7 => {
                            let w = weak.clone();
                            let sha_for_reset = sha_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let sha_val = sha_for_reset.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::ResetToCommit(oid, sha_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        8 => {
                            // Interactive Rebase — emit event with the right-clicked commit's OID.
                            // The workspace handler will build the commit list from HEAD down
                            // to (including) this commit and open the rebase editor.
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::InteractiveRebase(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        9 => {
                            let w = weak.clone();
                            let sha_for_click = sha_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let sha_val = sha_for_click.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CopyCommitSha(sha_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        10 => {
                            let w = weak.clone();
                            let msg_for_click = msg_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let msg_val = msg_for_click.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CopyCommitMessage(msg_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        11 => {
                            let w = weak.clone();
                            let author_for_click = author_name_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let author_val = author_for_click.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CopyAuthorName(author_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        12 => {
                            let w = weak.clone();
                            let date_for_click = date_clone.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let date_val = date_for_click.clone();
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::CopyDate(date_val));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        13 => {
                            // View on GitHub — emit OID; workspace handler constructs the URL.
                            let w = weak.clone();
                            item = item.on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    w.update(cx, |this: &mut GraphView, cx| {
                                        this.context_menu = None;
                                        cx.emit(GraphViewEvent::ViewOnGithub(oid));
                                        cx.notify();
                                    })
                                    .ok();
                                },
                            );
                        }
                        _ => {}
                    }

                    // Destructive actions render in error color (Bad and Reset)
                    let item_color = if idx == 6 || idx == 7 {
                        Color::Error
                    } else {
                        Color::Muted
                    };
                    let label_color = if idx == 6 || idx == 7 {
                        Color::Error
                    } else {
                        Color::Default
                    };

                    item = item
                        .child(Icon::new(icon).size(IconSize::XSmall).color(item_color))
                        .child(Label::new(label).size(LabelSize::XSmall).color(label_color));

                    menu = menu.child(item);
                }

                let menu = menu.with_animation(
                    "graph-context-menu-entrance",
                    Animation::new(Duration::from_millis(100))
                        .with_easing(|t| 1.0 - (1.0 - t).powi(5)),
                    |el, delta| el.opacity(delta),
                );
                container = container.child(menu);
            }
        }

        // Settings popover overlay
        if self.show_settings_popover {
            let popover_bg = colors.elevated_surface_background;
            let popover_border = colors.border;
            let popover_hover = colors.ghost_element_hover;

            let view_sha_length = cx.weak_entity();
            let view_subject = cx.weak_entity();
            let view_author = cx.weak_entity();
            let view_author_email = cx.weak_entity();
            let view_date = cx.weak_entity();
            let view_absolute_dates = cx.weak_entity();
            let view_avatars = cx.weak_entity();
            let view_graph_lanes = cx.weak_entity();
            let view_ref_badges = cx.weak_entity();

            let sha_length_label: SharedString = match self.sha_display_length {
                0 => "Short (7)".into(),
                40 => "Full (40)".into(),
                n => format!("{} chars", n).into(),
            };
            let subject_state = if cx.global::<SettingsState>().settings().show_subject_column {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let author_state = if self.show_author_column {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let author_email_state = if self.show_author_email {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let date_state = if self.show_date_column {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let absolute_dates_state = if self.show_absolute_dates {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let avatars_state = if self.show_avatars {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let graph_lanes_state = if self.show_graph_lanes {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let ref_badges_state = if self.show_ref_badges {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };

            let popover = div()
                .id("graph-settings-popover")
                .absolute()
                .right(px(8.))
                .top(px(28.))
                .v_flex()
                .min_w(px(180.))
                .py(px(4.))
                .bg(popover_bg)
                .border_1()
                .border_color(popover_border)
                .rounded(px(6.))
                .elevation_3(cx)
                .on_mouse_down(
                    MouseButton::Left,
                    |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation();
                    },
                )
                .child(
                    div().px(px(10.)).py(px(4.)).child(
                        Label::new("Display Settings")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .my(px(2.))
                        .bg(colors.border_variant),
                )
                .child(
                    div()
                        .id("toggle-sha-length")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_sha_length
                                .update(cx, |this: &mut GraphView, cx| {
                                    // Cycle: 0(short/7) → 8 → 10 → 12 → 40(full) → 0
                                    this.sha_display_length = match this.sha_display_length {
                                        0 => 8,
                                        8 => 10,
                                        10 => 12,
                                        12 => 40,
                                        _ => 0,
                                    };
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(Label::new("SHA length:").size(LabelSize::XSmall))
                        .child(
                            Label::new(sha_length_label)
                                .size(LabelSize::XSmall)
                                .color(Color::Accent)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    div()
                        .id("toggle-subject-col")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_subject
                                .update(cx, |_this: &mut GraphView, cx| {
                                    cx.update_global::<SettingsState, _>(|state, _cx| {
                                        state.settings_mut().show_subject_column =
                                            !state.settings().show_subject_column;
                                    });
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-subject-col", subject_state))
                        .child(Label::new("Show subject column").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-author-col")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_author
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_author_column = !this.show_author_column;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-author-col", author_state))
                        .child(Label::new("Show author column").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-author-email")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .pl(px(26.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_author_email
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_author_email = !this.show_author_email;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-author-email", author_email_state))
                        .child(Label::new("Show author email").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-date-col")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_date
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_date_column = !this.show_date_column;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-date-col", date_state))
                        .child(Label::new("Show date column").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-absolute-dates")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_absolute_dates
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_absolute_dates = !this.show_absolute_dates;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-absolute-dates", absolute_dates_state))
                        .child(Label::new("Absolute dates").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-avatars")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_avatars
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_avatars = !this.show_avatars;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-avatars", avatars_state))
                        .child(Label::new("Show avatars").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .my(px(2.))
                        .bg(colors.border_variant),
                )
                .child(
                    div()
                        .id("toggle-graph-lanes")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_graph_lanes
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_graph_lanes = !this.show_graph_lanes;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-graph-lanes", graph_lanes_state))
                        .child(Label::new("Show graph lanes").size(LabelSize::XSmall)),
                )
                .child(
                    div()
                        .id("toggle-ref-badges")
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(8.))
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .rounded(px(3.))
                        .hover(move |s| s.bg(popover_hover))
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_ref_badges
                                .update(cx, |this: &mut GraphView, cx| {
                                    this.show_ref_badges = !this.show_ref_badges;
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(Checkbox::new("cb-ref-badges", ref_badges_state))
                        .child(Label::new("Show branch/tag badges").size(LabelSize::XSmall)),
                );

            container = container.child(popover);
        }

        container.into_any_element()
    }
}

/// Parameters for rendering the virtual working tree row.
#[derive(Clone)]
struct WorkingTreeRowParams {
    list_index: usize,
    selected: bool,
    staged_count: usize,
    unstaged_count: usize,
    combined_breakdown: HashMap<FileChangeKind, usize>,
    worktree_name: String,
    branch_name: Option<String>,
    is_current_worktree: bool,
    is_orphan_worktree: bool,
    has_head_incoming: bool,
    pass_through_edges: Vec<GraphEdge>,
    row_height: f32,
    lane_width: f32,
    graph_padding_left: f32,
    graph_col_width: f32,
    working_tree_bg: gpui::Hsla,
    working_tree_border_color: gpui::Hsla,
    node_color: gpui::Hsla,
    selected_bg: gpui::Hsla,
    hover_bg: gpui::Hsla,
    active_bg: gpui::Hsla,
    panel_bg: gpui::Hsla,
    selected_border: gpui::Hsla,
    view: WeakEntity<GraphView>,
    show_author_column: bool,
    show_date_column: bool,
    author_col_width: f32,
    date_col_width: f32,
    #[allow(dead_code)]
    show_absolute_dates: bool,
    show_graph_lanes: bool,
    compact_mul: f32,
    has_context_menu: bool,
    /// The lane HEAD's commit sits on (so the working tree node connects to it).
    head_node_lane: usize,
}

/// Render the virtual "Working Tree" row that appears at the top of the graph.
fn render_working_tree_row(params: WorkingTreeRowParams) -> gpui::AnyElement {
    let WorkingTreeRowParams {
        list_index,
        selected,
        staged_count,
        unstaged_count,
        combined_breakdown,
        worktree_name,
        branch_name,
        is_current_worktree,
        is_orphan_worktree,
        has_head_incoming,
        pass_through_edges,
        row_height,
        lane_width,
        graph_padding_left,
        graph_col_width,
        working_tree_bg,
        working_tree_border_color,
        node_color,
        selected_bg,
        hover_bg,
        active_bg,
        panel_bg,
        selected_border,
        view,
        show_author_column,
        show_date_column,
        author_col_width,
        date_col_width,
        show_absolute_dates: _,
        show_graph_lanes,
        compact_mul,
        has_context_menu,
        head_node_lane,
    } = params;
    let bg = if selected {
        selected_bg
    } else {
        working_tree_bg
    };
    let row_hover_bg = if selected { selected_bg } else { hover_bg };
    let row_active_bg = if selected { selected_bg } else { active_bg };
    let left_tab_color = if selected {
        selected_border
    } else {
        working_tree_border_color
    };

    let has_changes = staged_count > 0 || unstaged_count > 0;

    let added_count = combined_breakdown
        .get(&FileChangeKind::Added)
        .copied()
        .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Untracked)
            .copied()
            .unwrap_or(0);
    let modified_count = combined_breakdown
        .get(&FileChangeKind::Modified)
        .copied()
        .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Renamed)
            .copied()
            .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::Copied)
            .copied()
            .unwrap_or(0)
        + combined_breakdown
            .get(&FileChangeKind::TypeChange)
            .copied()
            .unwrap_or(0);
    let deleted_count = combined_breakdown
        .get(&FileChangeKind::Deleted)
        .copied()
        .unwrap_or(0);
    let conflicted_count = combined_breakdown
        .get(&FileChangeKind::Conflicted)
        .copied()
        .unwrap_or(0);

    let graph_width = graph_col_width;
    let node_x = head_node_lane as f32 * lane_width + lane_width / 2.0 + graph_padding_left;
    let display_branch = branch_name.filter(|branch| !branch.is_empty());
    let row_title = if is_orphan_worktree {
        display_branch
            .clone()
            .map(|branch| format!("{branch} (no commits)"))
            .unwrap_or_else(|| "No commits yet".to_string())
    } else if let Some(branch) = display_branch.clone() {
        format!("Pending changes on {branch}")
    } else {
        "Pending changes".to_string()
    };
    let row_badge_color = if is_current_worktree || is_orphan_worktree {
        Color::Warning
    } else {
        Color::Accent
    };
    let hash_label = if is_orphan_worktree {
        "new".to_string()
    } else {
        "working".to_string()
    };

    let view_click = view.clone();

    div()
        .id(ElementId::NamedInteger(
            "commit-row".into(),
            list_index as u64,
        ))
        .h_flex()
        .items_center()
        .h(px(row_height))
        .w_full()
        .bg(bg)
        .border_b_1()
        .border_color(gpui::Hsla {
            a: 0.15,
            ..working_tree_border_color
        })
        .cursor(CursorStyle::PointingHand)
        .when(!has_context_menu, |el| {
            el.hover(move |s| s.bg(row_hover_bg))
        })
        .active(move |s| s.bg(row_active_bg))
        .on_click(
            move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
                view_click
                    .update(cx, |this, cx| {
                        this.dismiss_context_menu(cx);
                        this.select_list_index(list_index, cx);
                    })
                    .ok();
            },
        )
        .child(div().w(px(3.)).h_full().flex_shrink_0().bg(left_tab_color))
        .child(div().w(px(5.)).flex_shrink_0())
        // Graph column with canvas (hollow circle node + connecting line down)
        .when(show_graph_lanes, |el| {
            el.child(
                div()
                    .relative()
                    .w(px(graph_width))
                    .flex_shrink_0()
                    .h_full()
                    .child(
                        canvas(
                            |_bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut App| {},
                            move |bounds: Bounds<Pixels>,
                                  _: (),
                                  window: &mut Window,
                                  _cx: &mut App| {
                                let origin = bounds.origin;
                                let h = bounds.size.height;
                                let mid_y = px(row_height / 2.0);
                                let node_x_px = px(node_x);
                                let cx_x = origin.x + node_x_px;
                                let cy_y = origin.y + mid_y;

                                if has_head_incoming {
                                    let mut line_up = PathBuilder::stroke(px(2.0));
                                    line_up.move_to(point(cx_x, origin.y - px(4.0)));
                                    line_up.line_to(point(cx_x, cy_y));
                                    if let Ok(built) = line_up.build() {
                                        window.paint_path(built, node_color);
                                    }
                                }

                                for edge in &pass_through_edges {
                                    let lane_x = px(edge.from_lane as f32 * lane_width
                                        + lane_width / 2.0
                                        + graph_padding_left);
                                    let mut pass = PathBuilder::stroke(px(2.0));
                                    pass.move_to(point(origin.x + lane_x, origin.y - px(4.0)));
                                    pass.line_to(point(origin.x + lane_x, origin.y + h + px(4.0)));
                                    if let Ok(built) = pass.build() {
                                        window.paint_path(
                                            built,
                                            rgitui_theme::lane_color(edge.color_index),
                                        );
                                    }
                                }

                                // Vertical line from node center to bottom (connects to HEAD row below)
                                let mut line_down = PathBuilder::stroke(px(2.0));
                                line_down.move_to(point(cx_x, cy_y));
                                line_down.line_to(point(cx_x, origin.y + h + px(4.0)));
                                if let Ok(built) = line_down.build() {
                                    window.paint_path(built, node_color);
                                }

                                // Background ring to occlude lines behind the dot
                                let ring_r = 13.0_f32 * compact_mul;
                                let steps = 32_usize;
                                let mut ring = PathBuilder::fill();
                                for s in 0..steps {
                                    let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                                    let x = cx_x + px(ring_r * angle.cos());
                                    let y = cy_y + px(ring_r * angle.sin());
                                    if s == 0 {
                                        ring.move_to(point(x, y));
                                    } else {
                                        ring.line_to(point(x, y));
                                    }
                                }
                                ring.close();
                                if let Ok(built_ring) = ring.build() {
                                    window.paint_path(built_ring, panel_bg);
                                }

                                // Hollow circle (stroke only, no fill) to distinguish from commits
                                let dot_radius = 5.0_f32 * compact_mul;
                                let mut circle = PathBuilder::stroke(px(2.0));
                                for s in 0..=steps {
                                    let angle = (s as f32) * std::f32::consts::TAU / (steps as f32);
                                    let x = cx_x + px(dot_radius * angle.cos());
                                    let y = cy_y + px(dot_radius * angle.sin());
                                    if s == 0 {
                                        circle.move_to(point(x, y));
                                    } else {
                                        circle.line_to(point(x, y));
                                    }
                                }
                                if let Ok(built_circle) = circle.build() {
                                    window.paint_path(built_circle, node_color);
                                }

                                // Dashed outer ring to further distinguish
                                let outer_r = dot_radius + 3.0 * compact_mul;
                                let dash_count = 8_usize;
                                let arc_per_dash =
                                    std::f32::consts::TAU / (dash_count as f32 * 2.0);
                                for d in 0..dash_count {
                                    let start_angle =
                                        d as f32 * std::f32::consts::TAU / dash_count as f32;
                                    let end_angle = start_angle + arc_per_dash;
                                    let mut dash = PathBuilder::stroke(px(1.5));
                                    let sx = cx_x + px(outer_r * start_angle.cos());
                                    let sy = cy_y + px(outer_r * start_angle.sin());
                                    dash.move_to(point(sx, sy));
                                    let ex = cx_x + px(outer_r * end_angle.cos());
                                    let ey = cy_y + px(outer_r * end_angle.sin());
                                    dash.line_to(point(ex, ey));
                                    if let Ok(built_dash) = dash.build() {
                                        let dash_color = gpui::Hsla {
                                            a: 0.6,
                                            ..node_color
                                        };
                                        window.paint_path(built_dash, dash_color);
                                    }
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
        })
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .h_flex()
                .items_center()
                .gap(px(4.))
                .child(
                    Icon::new(IconName::Edit)
                        .size(IconSize::XSmall)
                        .color(Color::Warning),
                )
                .child(
                    Label::new(hash_label)
                        .size(LabelSize::XSmall)
                        .color(if is_current_worktree || is_orphan_worktree {
                            Color::Warning
                        } else {
                            Color::Accent
                        })
                        .weight(gpui::FontWeight::MEDIUM),
                ),
        )
        .child({
            let mut message_col = div()
                .flex_1()
                .min_w_0()
                .h_flex()
                .items_center()
                .gap(px(4.))
                .overflow_x_hidden();

            let badge_color = if has_changes {
                row_badge_color
            } else {
                Color::Muted
            };
            message_col = message_col.child(
                div()
                    .flex_shrink_0()
                    .child(Badge::new(worktree_name).color(badge_color).bold()),
            );
            if is_orphan_worktree {
                message_col = message_col.child(
                    div()
                        .flex_shrink_0()
                        .child(Badge::new("New branch").color(Color::Warning)),
                );
            }

            if has_changes {
                message_col = message_col.child(
                    Label::new(row_title)
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                );
                if staged_count > 0 {
                    message_col = message_col.child(
                        div().flex_shrink_0().child(
                            Badge::new(SharedString::from(format!("{} staged", staged_count)))
                                .color(Color::Success),
                        ),
                    );
                }
                if unstaged_count > 0 {
                    message_col = message_col.child(
                        div().flex_shrink_0().child(
                            Badge::new(SharedString::from(format!("{} unstaged", unstaged_count)))
                                .color(Color::Warning),
                        ),
                    );
                }

                let mut indicators = div().h_flex().gap(px(4.)).flex_shrink_0();

                if added_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("+{added_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Success)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if modified_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("~{modified_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Accent)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if deleted_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("-{deleted_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }
                if conflicted_count > 0 {
                    indicators = indicators.child(
                        Label::new(SharedString::from(format!("!{conflicted_count}")))
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    );
                }

                message_col = message_col.child(indicators);
            } else {
                message_col = message_col.child(
                    Label::new(row_title)
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                );
            }

            message_col
        })
        .when(show_author_column, |el| {
            el.child(div().w(px(author_col_width)).flex_shrink_0())
        })
        .when(show_date_column, |el| {
            el.child(div().w(px(date_col_width)).flex_shrink_0())
        })
        .into_any_element()
}

/// Compute a breakdown of file change kinds from a list of `FileStatus` entries.
pub fn compute_breakdown(files: &[rgitui_git::FileStatus]) -> HashMap<FileChangeKind, usize> {
    let mut map = HashMap::new();
    for f in files {
        *map.entry(f.kind).or_insert(0) += 1;
    }
    map
}

fn format_relative_time(
    time: &chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let duration = now.signed_duration_since(*time);

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_weeks() < 4 {
        format!("{}w ago", duration.num_weeks())
    } else if duration.num_days() < 365 {
        format!("{}mo ago", duration.num_days() / 30)
    } else {
        format!("{}y ago", duration.num_days() / 365)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use rgitui_git::{FileChangeKind, FileStatus};
    use std::path::PathBuf;

    fn make_file_status(kind: FileChangeKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from("file.txt"),
            kind,
            old_path: None,
            additions: 0,
            deletions: 0,
        }
    }

    // --- compute_breakdown ---

    #[test]
    fn compute_breakdown_empty() {
        let result = compute_breakdown(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_breakdown_single_kind() {
        let files = vec![
            make_file_status(FileChangeKind::Modified),
            make_file_status(FileChangeKind::Modified),
        ];
        let result = compute_breakdown(&files);
        assert_eq!(result.get(&FileChangeKind::Modified), Some(&2));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn compute_breakdown_mixed_kinds() {
        let files = vec![
            make_file_status(FileChangeKind::Added),
            make_file_status(FileChangeKind::Modified),
            make_file_status(FileChangeKind::Deleted),
            make_file_status(FileChangeKind::Added),
        ];
        let result = compute_breakdown(&files);
        assert_eq!(result.get(&FileChangeKind::Added), Some(&2));
        assert_eq!(result.get(&FileChangeKind::Modified), Some(&1));
        assert_eq!(result.get(&FileChangeKind::Deleted), Some(&1));
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn compute_breakdown_all_kinds() {
        let kinds = [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
            FileChangeKind::Copied,
            FileChangeKind::TypeChange,
            FileChangeKind::Untracked,
            FileChangeKind::Conflicted,
        ];
        let files: Vec<FileStatus> = kinds.iter().copied().map(make_file_status).collect();
        let result = compute_breakdown(&files);
        assert_eq!(result.len(), 8);
        for kind in &kinds {
            assert_eq!(result.get(kind), Some(&1));
        }
    }

    // --- format_relative_time ---

    #[test]
    fn format_relative_time_just_now() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&now, now), "just now");
    }

    #[test]
    fn format_relative_time_minutes() {
        let now = Utc::now();
        let t = now - Duration::minutes(30);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "30m ago");
    }

    #[test]
    fn format_relative_time_hours() {
        let now = Utc::now();
        let t = now - Duration::hours(5);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "5h ago");
    }

    #[test]
    fn format_relative_time_days() {
        let now = Utc::now();
        let t = now - Duration::days(3);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "3d ago");
    }

    #[test]
    fn format_relative_time_weeks() {
        let now = Utc::now();
        let t = now - Duration::weeks(2);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "2w ago");
    }

    #[test]
    fn format_relative_time_months() {
        let now = Utc::now();
        let t = now - Duration::days(60);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "2mo ago");
    }

    #[test]
    fn format_relative_time_years() {
        let now = Utc::now();
        let t = now - Duration::days(400);
        let result = format_relative_time(&t, now);
        assert_eq!(result, "1y ago");
    }
}
