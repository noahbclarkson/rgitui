use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    FontWeight, KeyDownEvent, Render, ScrollStrategy, SharedString, UniformListScrollHandle,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

/// Pre-computed git context used for context-sensitive command filtering.
/// Computed from GitProject state and passed to predicates.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandContext {
    /// True when the repository has at least one remote configured.
    pub has_remotes: bool,
    /// True when the worktree has staged or unstaged changes.
    pub has_changes: bool,
    /// True when the worktree is clean (no uncommitted changes).
    pub worktree_clean: bool,
    /// True when the repository is currently bisecting.
    pub is_bisecting: bool,
    /// True when there is at least one stash entry.
    pub has_stashes: bool,
    /// True when there are staged files to commit.
    pub has_staged: bool,
    /// True when a merge, rebase, cherry-pick, or revert is in progress.
    pub in_progress_operation: bool,
    /// True when the user has a GitHub token configured.
    pub has_github_token: bool,
}

impl CommandContext {
    /// No-project state: all context flags are false (commands are hidden if they
    /// require specific conditions).
    pub fn none() -> Self {
        Self {
            has_remotes: false,
            has_changes: false,
            worktree_clean: false,
            is_bisecting: false,
            has_stashes: false,
            has_staged: false,
            in_progress_operation: false,
            has_github_token: false,
        }
    }
}

/// A no-op predicate that always shows the command.
const fn always_show(_: CommandContext) -> bool {
    true
}

/// Show only when the user has a GitHub token configured (for PR creation).
const fn has_github_token(ctx: CommandContext) -> bool {
    ctx.has_github_token
}

/// Show only when the repository has at least one remote configured.
const fn has_remotes(ctx: CommandContext) -> bool {
    ctx.has_remotes
}

/// Show only when there are unstaged and/or staged file changes.
const fn has_changes(ctx: CommandContext) -> bool {
    ctx.has_changes
}

/// Show only when the repository worktree is clean (no uncommitted changes).
const fn worktree_clean(ctx: CommandContext) -> bool {
    ctx.worktree_clean
}

/// Show only when the repository is currently bisecting.
const fn is_bisecting(ctx: CommandContext) -> bool {
    ctx.is_bisecting
}

/// Show only when there is at least one stash entry.
const fn has_stashes(ctx: CommandContext) -> bool {
    ctx.has_stashes
}

/// Show only when there are staged files to commit.
const fn has_staged(ctx: CommandContext) -> bool {
    ctx.has_staged
}

/// Show only when in a merge, rebase, cherry-pick, or revert in-progress state.
const fn in_progress_operation(ctx: CommandContext) -> bool {
    ctx.in_progress_operation
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    Fetch,
    Pull,
    Push,
    ForcePush,
    Commit,
    StageAll,
    UnstageAll,
    StashSave,
    StashPop,
    StashApply,
    StashDrop,
    CreateBranch,
    DeleteBranch,
    RenameBranch,
    MergeBranch,
    CreateTag,
    CreateWorktree,
    CreatePr,
    CherryPick,
    RevertCommit,
    InteractiveRebase,
    DiscardAll,
    ResetHard,
    AbortOperation,
    ContinueMerge,
    ToggleDiffMode,
    Search,
    AiMessage,
    Refresh,
    Settings,
    OpenRepo,
    WorkspaceHome,
    RestoreLastWorkspace,
    Shortcuts,
    SwitchBranch,
    Blame,
    Undo,
    FileHistory,
    Reflog,
    Submodules,
    BisectStart,
    BisectGood,
    BisectBad,
    BisectReset,
    BisectSkip,
    GlobalSearch,
}

impl CommandId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push => "push",
            Self::ForcePush => "force_push",
            Self::Commit => "commit",
            Self::StageAll => "stage_all",
            Self::UnstageAll => "unstage_all",
            Self::StashSave => "stash_save",
            Self::StashPop => "stash_pop",
            Self::StashApply => "stash_apply",
            Self::StashDrop => "stash_drop",
            Self::CreateBranch => "create_branch",
            Self::DeleteBranch => "delete_branch",
            Self::RenameBranch => "rename_branch",
            Self::MergeBranch => "merge_branch",
            Self::CreateTag => "create_tag",
            Self::CreateWorktree => "create_worktree",
            Self::CreatePr => "create_pr",
            Self::CherryPick => "cherry_pick",
            Self::RevertCommit => "revert_commit",
            Self::InteractiveRebase => "interactive_rebase",
            Self::DiscardAll => "discard_all",
            Self::ResetHard => "reset_hard",
            Self::AbortOperation => "abort_operation",
            Self::ContinueMerge => "continue_merge",
            Self::ToggleDiffMode => "toggle_diff_mode",
            Self::Search => "search",
            Self::AiMessage => "ai_message",
            Self::Refresh => "refresh",
            Self::Settings => "settings",
            Self::OpenRepo => "open_repo",
            Self::WorkspaceHome => "workspace_home",
            Self::RestoreLastWorkspace => "restore_last_workspace",
            Self::Shortcuts => "shortcuts",
            Self::SwitchBranch => "switch_branch",
            Self::Blame => "blame",
            Self::Undo => "undo",
            Self::FileHistory => "file_history",
            Self::Reflog => "reflog",
            Self::Submodules => "submodules",
            Self::BisectStart => "bisect_start",
            Self::BisectGood => "bisect_good",
            Self::BisectBad => "bisect_bad",
            Self::BisectReset => "bisect_reset",
            Self::BisectSkip => "bisect_skip",
            Self::GlobalSearch => "global_search",
        }
    }

    pub fn display_label(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push => "push",
            Self::ForcePush => "force push",
            Self::Commit => "commit",
            Self::StageAll => "stage all",
            Self::UnstageAll => "unstage all",
            Self::StashSave => "stash save",
            Self::StashPop => "stash pop",
            Self::StashApply => "stash apply",
            Self::StashDrop => "stash drop",
            Self::CreateBranch => "create branch",
            Self::DeleteBranch => "delete branch",
            Self::RenameBranch => "rename branch",
            Self::MergeBranch => "merge branch",
            Self::CreateTag => "create tag",
            Self::CreateWorktree => "create worktree",
            Self::CreatePr => "create pull request",
            Self::CherryPick => "cherry pick",
            Self::RevertCommit => "revert commit",
            Self::InteractiveRebase => "interactive rebase",
            Self::DiscardAll => "discard all",
            Self::ResetHard => "reset hard",
            Self::AbortOperation => "abort operation",
            Self::ContinueMerge => "continue merge",
            Self::ToggleDiffMode => "toggle diff mode",
            Self::Search => "search",
            Self::AiMessage => "ai message",
            Self::Refresh => "refresh",
            Self::Settings => "settings",
            Self::OpenRepo => "open repo",
            Self::WorkspaceHome => "workspace home",
            Self::RestoreLastWorkspace => "restore last workspace",
            Self::Shortcuts => "shortcuts",
            Self::SwitchBranch => "switch branch",
            Self::Blame => "blame file",
            Self::Undo => "undo last operation",
            Self::FileHistory => "file history",
            Self::Reflog => "reflog",
            Self::Submodules => "submodules",
            Self::BisectStart => "bisect start",
            Self::BisectGood => "bisect good (current)",
            Self::BisectBad => "bisect bad (current)",
            Self::BisectReset => "bisect reset",
            Self::BisectSkip => "bisect skip (current)",
            Self::GlobalSearch => "global search",
        }
    }
}

impl std::fmt::Display for CommandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for CommandId {
    type Error = ();

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "fetch" => Ok(Self::Fetch),
            "pull" => Ok(Self::Pull),
            "push" => Ok(Self::Push),
            "force_push" => Ok(Self::ForcePush),
            "commit" => Ok(Self::Commit),
            "stage_all" => Ok(Self::StageAll),
            "unstage_all" => Ok(Self::UnstageAll),
            "stash_save" => Ok(Self::StashSave),
            "stash_pop" => Ok(Self::StashPop),
            "stash_apply" => Ok(Self::StashApply),
            "stash_drop" => Ok(Self::StashDrop),
            "create_branch" => Ok(Self::CreateBranch),
            "delete_branch" => Ok(Self::DeleteBranch),
            "rename_branch" => Ok(Self::RenameBranch),
            "merge_branch" => Ok(Self::MergeBranch),
            "create_tag" => Ok(Self::CreateTag),
            "create_worktree" => Ok(Self::CreateWorktree),
            "create_pr" => Ok(Self::CreatePr),
            "cherry_pick" => Ok(Self::CherryPick),
            "revert_commit" => Ok(Self::RevertCommit),
            "interactive_rebase" => Ok(Self::InteractiveRebase),
            "discard_all" => Ok(Self::DiscardAll),
            "reset_hard" => Ok(Self::ResetHard),
            "abort_operation" => Ok(Self::AbortOperation),
            "continue_merge" => Ok(Self::ContinueMerge),
            "toggle_diff_mode" => Ok(Self::ToggleDiffMode),
            "search" => Ok(Self::Search),
            "ai_message" => Ok(Self::AiMessage),
            "refresh" => Ok(Self::Refresh),
            "settings" => Ok(Self::Settings),
            "open_repo" => Ok(Self::OpenRepo),
            "workspace_home" => Ok(Self::WorkspaceHome),
            "restore_last_workspace" => Ok(Self::RestoreLastWorkspace),
            "shortcuts" => Ok(Self::Shortcuts),
            "switch_branch" => Ok(Self::SwitchBranch),
            "blame" => Ok(Self::Blame),
            "undo" => Ok(Self::Undo),
            "file_history" => Ok(Self::FileHistory),
            "reflog" => Ok(Self::Reflog),
            "submodules" => Ok(Self::Submodules),
            "bisect_start" => Ok(Self::BisectStart),
            "bisect_good" => Ok(Self::BisectGood),
            "bisect_bad" => Ok(Self::BisectBad),
            "bisect_reset" => Ok(Self::BisectReset),
            "bisect_skip" => Ok(Self::BisectSkip),
            "global_search" => Ok(Self::GlobalSearch),
            _ => Err(()),
        }
    }
}

#[derive(Clone)]
pub struct PaletteCommand {
    pub id: CommandId,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub category: &'static str,
    /// Context predicate — evaluated at filter-time to determine visibility.
    predicate: fn(CommandContext) -> bool,
}

impl PaletteCommand {
    fn new(
        id: CommandId,
        label: &'static str,
        shortcut: Option<&'static str>,
        category: &'static str,
    ) -> Self {
        Self {
            id,
            label,
            shortcut,
            category,
            predicate: always_show,
        }
    }

    fn with_predicate(mut self, pred: fn(CommandContext) -> bool) -> Self {
        self.predicate = pred;
        self
    }
}

#[derive(Debug, Clone)]
pub enum CommandPaletteEvent {
    CommandSelected(CommandId),
    Dismissed,
}

pub struct CommandPalette {
    visible: bool,
    query_editor: Entity<TextInput>,
    commands: Vec<PaletteCommand>,
    /// Each entry is `(command_index, fuzzy_score)`, sorted by score descending.
    filtered_indices: Vec<(usize, usize)>,
    selected_index: usize,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// Pre-computed git context for context-sensitive command filtering.
    context: CommandContext,
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Context-sensitive predicates:
        //   has_remotes  — only when remotes are configured
        //   has_staged   — only when files are staged
        //   has_changes  — only when worktree has unstaged/staged changes
        //   has_stashes  — only when stash entries exist
        //   worktree_clean — only when no uncommitted changes
        //   is_bisecting — only when a bisect is in progress
        //   in_progress_operation — only during merge/rebase/cherry-pick
        //   always_show  — no context restriction
        let commands: Vec<PaletteCommand> = vec![
            PaletteCommand::new(CommandId::Fetch, "Git: Fetch", Some("Ctrl+Shift+F"), "Git")
                .with_predicate(has_remotes),
            PaletteCommand::new(CommandId::Pull, "Git: Pull", None, "Git")
                .with_predicate(has_remotes),
            PaletteCommand::new(CommandId::Push, "Git: Push", None, "Git")
                .with_predicate(has_remotes),
            PaletteCommand::new(CommandId::ForcePush, "Git: Force Push", None, "Git")
                .with_predicate(has_remotes),
            PaletteCommand::new(CommandId::Commit, "Git: Commit", Some("Ctrl+Enter"), "Git")
                .with_predicate(has_staged),
            PaletteCommand::new(CommandId::StageAll, "Git: Stage All", Some("Ctrl+S"), "Git")
                .with_predicate(has_changes),
            PaletteCommand::new(
                CommandId::UnstageAll,
                "Git: Unstage All",
                Some("Ctrl+U"),
                "Git",
            )
            .with_predicate(has_changes),
            PaletteCommand::new(CommandId::StashSave, "Git: Stash", Some("Ctrl+Z"), "Git")
                .with_predicate(has_changes),
            PaletteCommand::new(
                CommandId::StashPop,
                "Git: Pop Stash",
                Some("Ctrl+Shift+Z"),
                "Git",
            )
            .with_predicate(has_stashes),
            PaletteCommand::new(
                CommandId::StashApply,
                "Git: Apply Stash (keep)",
                None,
                "Git",
            )
            .with_predicate(has_stashes),
            PaletteCommand::new(CommandId::StashDrop, "Git: Drop Stash", None, "Git")
                .with_predicate(has_stashes),
            PaletteCommand::new(
                CommandId::CreateBranch,
                "Git: Create Branch",
                Some("Ctrl+B"),
                "Git",
            ),
            PaletteCommand::new(
                CommandId::SwitchBranch,
                "Git: Switch Branch",
                Some("Ctrl+Shift+B"),
                "Git",
            ),
            PaletteCommand::new(CommandId::DeleteBranch, "Git: Delete Branch", None, "Git"),
            PaletteCommand::new(CommandId::RenameBranch, "Git: Rename Branch", None, "Git"),
            PaletteCommand::new(CommandId::MergeBranch, "Git: Merge Branch", None, "Git")
                .with_predicate(worktree_clean),
            PaletteCommand::new(CommandId::CreateTag, "Git: Create Tag", None, "Git"),
            PaletteCommand::new(
                CommandId::CreateWorktree,
                "Git: Create Worktree",
                None,
                "Git",
            ),
            PaletteCommand::new(CommandId::CreatePr, "Git: Create Pull Request", None, "Git")
                .with_predicate(has_github_token),
            PaletteCommand::new(
                CommandId::CherryPick,
                "Git: Cherry-pick Commit",
                None,
                "Git",
            )
            .with_predicate(worktree_clean),
            PaletteCommand::new(CommandId::RevertCommit, "Git: Revert Commit", None, "Git")
                .with_predicate(worktree_clean),
            PaletteCommand::new(
                CommandId::InteractiveRebase,
                "Git: Interactive Rebase",
                None,
                "Git",
            )
            .with_predicate(worktree_clean),
            PaletteCommand::new(
                CommandId::DiscardAll,
                "Git: Discard All Changes",
                None,
                "Git",
            )
            .with_predicate(has_changes),
            PaletteCommand::new(
                CommandId::ResetHard,
                "Git: Reset Hard (to HEAD)",
                None,
                "Git",
            )
            .with_predicate(has_changes),
            PaletteCommand::new(
                CommandId::AbortOperation,
                "Git: Abort Merge/Rebase",
                None,
                "Git",
            )
            .with_predicate(in_progress_operation),
            PaletteCommand::new(CommandId::ContinueMerge, "Git: Continue Merge", None, "Git")
                .with_predicate(in_progress_operation),
            PaletteCommand::new(
                CommandId::ToggleDiffMode,
                "View: Toggle Diff Mode",
                Some("d"),
                "View",
            ),
            PaletteCommand::new(
                CommandId::Search,
                "View: Search Commits",
                Some("Ctrl+F"),
                "View",
            ),
            PaletteCommand::new(
                CommandId::AiMessage,
                "AI: Generate Commit Message",
                Some("Ctrl+G"),
                "AI",
            )
            .with_predicate(has_staged),
            PaletteCommand::new(CommandId::Refresh, "Git: Refresh", Some("F5"), "Git"),
            PaletteCommand::new(
                CommandId::Settings,
                "Preferences: Open Settings",
                Some("Ctrl+,"),
                "Preferences",
            ),
            PaletteCommand::new(
                CommandId::OpenRepo,
                "File: Open Repository",
                Some("Ctrl+O"),
                "File",
            ),
            PaletteCommand::new(
                CommandId::WorkspaceHome,
                "Workspace: Home",
                None,
                "Workspace",
            ),
            PaletteCommand::new(
                CommandId::RestoreLastWorkspace,
                "Workspace: Restore Last",
                None,
                "Workspace",
            ),
            PaletteCommand::new(
                CommandId::Shortcuts,
                "Help: Keyboard Shortcuts",
                Some("?"),
                "Help",
            ),
            PaletteCommand::new(CommandId::Blame, "View: Blame File", Some("b"), "View"),
            PaletteCommand::new(
                CommandId::FileHistory,
                "View: File History",
                Some("h"),
                "View",
            ),
            PaletteCommand::new(CommandId::Undo, "Edit: Undo Last Operation", None, "Edit"),
            PaletteCommand::new(CommandId::BisectStart, "Git: Bisect Start", None, "Git")
                .with_predicate(worktree_clean),
            PaletteCommand::new(
                CommandId::BisectGood,
                "Git: Bisect Good (mark current)",
                None,
                "Git",
            )
            .with_predicate(is_bisecting),
            PaletteCommand::new(
                CommandId::BisectBad,
                "Git: Bisect Bad (mark current)",
                None,
                "Git",
            )
            .with_predicate(is_bisecting),
            PaletteCommand::new(CommandId::BisectReset, "Git: Bisect Reset", None, "Git")
                .with_predicate(is_bisecting),
            PaletteCommand::new(
                CommandId::BisectSkip,
                "Git: Bisect Skip (skip this commit)",
                None,
                "Git",
            )
            .with_predicate(is_bisecting),
            PaletteCommand::new(CommandId::Reflog, "View: Reflog", None, "View"),
            PaletteCommand::new(CommandId::Submodules, "View: Submodules", None, "View"),
            PaletteCommand::new(
                CommandId::GlobalSearch,
                "Search: Global Search",
                Some("Ctrl+Shift+F"),
                "Search",
            ),
        ];

        let filtered_indices: Vec<(usize, usize)> = (0..commands.len()).map(|i| (i, 0)).collect();

        let query_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Type a command...");
            ti
        });

        cx.subscribe(
            &query_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(_) => {
                    this.update_filter(cx);
                    cx.notify();
                }
                TextInputEvent::Submit => {
                    this.select_current(cx);
                }
            },
        )
        .detach();

        Self {
            visible: false,
            query_editor,
            commands,
            filtered_indices,
            selected_index: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            context: CommandContext::none(),
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.query_editor.update(cx, |editor, cx| {
                editor.clear(cx);
            });
            self.selected_index = 0;
            self.update_filter(cx);
            self.query_editor.update(cx, |editor, cx| {
                editor.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(CommandPaletteEvent::Dismissed);
        cx.notify();
    }

    /// Update the git context used for context-sensitive command filtering.
    /// Called by the workspace whenever the project state changes.
    pub fn set_context(&mut self, context: CommandContext) {
        self.context = context;
    }

    /// Fuzzy subsequence match. Returns a score (higher = better) or None if
    /// query chars don't all appear in target in order.
    pub(crate) fn fuzzy_score(query: &str, target: &str) -> Option<usize> {
        if query.is_empty() {
            return Some(0);
        }
        let target_len = target.len();
        let mut score: usize = 0;
        let mut t_chars = target.char_indices().peekable();
        // query is already lowercased by caller (update_filter); targets are also lowercased by caller.
        // We still do case-insensitive for safety in direct calls.
        let query_lc = query.to_lowercase();
        for q_char in query_lc.chars() {
            loop {
                match t_chars.next() {
                    Some((pos, t_char)) => {
                        if t_char.to_ascii_lowercase() == q_char {
                            // Prefer matches at earlier positions → higher score
                            score += target_len.saturating_sub(pos);
                            break;
                        }
                    }
                    None => return None, // query char not found
                }
            }
        }
        Some(score)
    }

    fn update_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query_editor.read(cx).text().to_lowercase();

        // Collect command indices whose predicate passes based on current context.
        let ctx = self.context;
        let valid_indices: Vec<usize> = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| (cmd.predicate)(ctx))
            .map(|(i, _)| i)
            .collect();

        if query.is_empty() {
            self.filtered_indices = valid_indices.into_iter().map(|i| (i, 0)).collect();
        } else {
            let mut scored: Vec<(usize, usize)> = valid_indices
                .iter()
                .filter_map(|&i| {
                    let cmd = &self.commands[i];
                    // Best score across label, id, and category
                    let label_lc = cmd.label.to_lowercase();
                    let id_lc = cmd.id.as_str().to_lowercase();
                    let cat_lc = cmd.category.to_lowercase();
                    let score = [label_lc.as_str(), id_lc.as_str(), cat_lc.as_str()]
                        .iter()
                        .filter_map(|target| Self::fuzzy_score(&query, target))
                        .max();
                    score.map(|s| (i, s))
                })
                .collect();
            // Sort by score descending (best first)
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered_indices = scored;
        }
        self.selected_index = 0;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn select_current(&mut self, cx: &mut Context<Self>) {
        if let Some(&(idx, _)) = self.filtered_indices.get(self.selected_index) {
            let cmd_id = self.commands[idx].id;
            self.visible = false;
            cx.emit(CommandPaletteEvent::CommandSelected(cmd_id));
            cx.notify();
        }
    }

    fn category_icon(category: &str) -> IconName {
        match category {
            "Git" => IconName::GitBranch,
            "View" => IconName::Eye,
            "AI" => IconName::Sparkle,
            "Preferences" => IconName::Settings,
            "File" => IconName::Folder,
            "Workspace" => IconName::Menu,
            "Help" => IconName::Star,
            "Edit" => IconName::Undo,
            _ => IconName::Terminal,
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        match key {
            "escape" => {
                self.dismiss(cx);
                cx.stop_propagation();
            }
            "up" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.scroll_handle
                        .scroll_to_item(self.selected_index, ScrollStrategy::Nearest);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "down" => {
                if self.selected_index + 1 < self.filtered_indices.len() {
                    self.selected_index += 1;
                    self.scroll_handle
                        .scroll_to_item(self.selected_index, ScrollStrategy::Nearest);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("command-palette").into_any_element();
        }

        let colors = cx.colors();
        let query_is_empty = self.query_editor.read(cx).is_empty();
        let filtered_count = self.filtered_indices.len();

        let mut modal = div()
            .id("command-palette-modal")
            .track_focus(&self.focus_handle)
            .key_context("CommandPalette")
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(480.))
            .max_h(px(440.))
            .elevation_3(cx)
            .rounded(px(10.))
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_move(|_, _, cx| {
                cx.stop_propagation();
            });

        let focused_border = colors.border_focused;
        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .h(px(44.))
                .px(px(14.))
                .gap(px(10.))
                .items_center()
                .border_b_1()
                .border_color(focused_border)
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(div().flex_1().child(self.query_editor.clone()))
                .when(!query_is_empty, |el| {
                    let count_text: SharedString = format!("{filtered_count} results").into();
                    el.child(
                        Label::new(count_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                }),
        );

        if filtered_count > 0 {
            let selected_index = self.selected_index;
            let selected_bg = colors.element_selected;
            let hover_bg = colors.ghost_element_hover;
            let hint_bg = colors.hint_background;

            let commands: Arc<Vec<PaletteCommand>> = Arc::new(self.commands.clone());
            let filtered: Arc<Vec<(usize, usize)>> = Arc::new(self.filtered_indices.clone());
            let view = cx.weak_entity();

            let visible_items = filtered_count.min(10);
            let list_height = visible_items as f32 * 36.0 + 4.0;

            let list = uniform_list(
                "palette-results",
                filtered_count,
                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                    range
                        .map(|display_idx| {
                            let (cmd_idx, _score) = filtered[display_idx];
                            let cmd = &commands[cmd_idx];
                            let is_selected = display_idx == selected_index;

                            let label: SharedString = cmd.label.into();
                            let cmd_id = cmd.id;
                            let icon = CommandPalette::category_icon(cmd.category);
                            let shortcut = cmd.shortcut;

                            let view_click = view.clone();

                            let mut row = div()
                                .id(ElementId::NamedInteger(
                                    "palette-cmd".into(),
                                    display_idx as u64,
                                ))
                                .h_flex()
                                .w_full()
                                .h(px(36.))
                                .px(px(14.))
                                .mx(px(4.))
                                .gap(px(10.))
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .when(is_selected, move |el| el.bg(selected_bg))
                                .hover(move |s| s.bg(hover_bg))
                                .on_click(move |_: &ClickEvent, _, cx| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.visible = false;
                                            cx.emit(CommandPaletteEvent::CommandSelected(cmd_id));
                                            cx.notify();
                                        })
                                        .ok();
                                });

                            row = row.child(Icon::new(icon).size(IconSize::Small).color(
                                if is_selected {
                                    Color::Accent
                                } else {
                                    Color::Muted
                                },
                            ));

                            row = row.child(
                                Label::new(label)
                                    .size(LabelSize::Small)
                                    .when(is_selected, |l| l.weight(FontWeight::MEDIUM)),
                            );

                            row = row.child(div().flex_1());

                            if let Some(shortcut_text) = shortcut {
                                row = row.child(
                                    div()
                                        .h_flex()
                                        .h(px(22.))
                                        .px(px(8.))
                                        .rounded(px(4.))
                                        .bg(hint_bg)
                                        .items_center()
                                        .child(
                                            Label::new(SharedString::from(shortcut_text))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted)
                                                .weight(FontWeight::MEDIUM),
                                        ),
                                );
                            }

                            row.into_any_element()
                        })
                        .collect()
                },
            )
            .h(px(list_height))
            .max_h(px(360.))
            .track_scroll(&self.scroll_handle);

            modal = modal.child(div().py(px(4.)).child(list));
        } else {
            modal = modal.child(
                div()
                    .id("palette-empty")
                    .w_full()
                    .py(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .v_flex()
                    .gap(px(8.))
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new("No matching commands")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        }

        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .h(px(32.))
                .px(px(14.))
                .gap(px(16.))
                .items_center()
                .border_t_1()
                .border_color(colors.border_variant)
                .bg(colors.surface_background)
                .child(
                    Label::new("\u{2191}\u{2193} navigate")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    Label::new("\u{23ce} select")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    Label::new("esc dismiss")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                ),
        );

        div()
            .id("command-palette-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(80.))
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.4,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::CommandPalette;

    #[test]
    fn fuzzy_score_exact_match_returns_score() {
        assert!(CommandPalette::fuzzy_score("push", "Push to Remote").is_some());
    }

    #[test]
    fn fuzzy_score_case_insensitive() {
        assert!(CommandPalette::fuzzy_score("push", "PUSH").is_some());
        assert!(CommandPalette::fuzzy_score("PUSH", "push").is_some());
    }

    #[test]
    fn fuzzy_score_missing_char_returns_none() {
        assert_eq!(CommandPalette::fuzzy_score("xyz", "Push"), None);
    }

    #[test]
    fn fuzzy_score_empty_query_returns_zero() {
        assert_eq!(CommandPalette::fuzzy_score("", "Push to Remote"), Some(0));
    }

    #[test]
    fn fuzzy_score_earlier_match_higher_score() {
        let score_early = CommandPalette::fuzzy_score("sh", "Show").unwrap();
        let score_late = CommandPalette::fuzzy_score("sh", "Fish").unwrap();
        assert!(
            score_early > score_late,
            "earlier match should score higher: {score_early} vs {score_late}"
        );
    }

    #[test]
    fn fuzzy_score_subsequence_in_order() {
        assert!(CommandPalette::fuzzy_score("pd", "Push and Delete").is_some());
        assert_eq!(CommandPalette::fuzzy_score("dp", "Push and Delete"), None);
    }

    #[test]
    fn fuzzy_score_longer_target_scores_higher_when_same_prefix() {
        // Same query "co", same positions, longer target gives higher score
        // because score = sum(target_len - matched_pos)
        let score_short = CommandPalette::fuzzy_score("co", "Commit").unwrap();
        let score_long = CommandPalette::fuzzy_score("co", "Commit Message").unwrap();
        assert!(
            score_long > score_short,
            "longer matching target should score higher: {score_long} vs {score_short}"
        );
    }

    #[test]
    fn fuzzy_score_repeated_chars() {
        assert_eq!(CommandPalette::fuzzy_score("pp", "Push"), None);
        assert!(CommandPalette::fuzzy_score("ps", "Push").is_some());
    }

    #[test]
    fn fuzzy_score_single_char_query() {
        // Single char should match first occurrence
        assert!(CommandPalette::fuzzy_score("a", "Push").is_none());
        assert!(CommandPalette::fuzzy_score("p", "Push").is_some());
        assert!(CommandPalette::fuzzy_score("u", "Push").is_some());
        assert!(CommandPalette::fuzzy_score("s", "Push").is_some());
    }

    #[test]
    fn fuzzy_score_query_longer_than_target() {
        // Query longer than target: should fail
        assert_eq!(CommandPalette::fuzzy_score("pushit", "Push"), None);
    }

    #[test]
    fn fuzzy_score_empty_target() {
        // Non-empty query with empty target should fail
        assert_eq!(CommandPalette::fuzzy_score("abc", ""), None);
    }

    #[test]
    fn fuzzy_score_numbers_and_special_chars() {
        // Numbers in query and target
        assert!(CommandPalette::fuzzy_score("42", "Answer 42").is_some());
        assert!(CommandPalette::fuzzy_score("v2", "version2").is_some());
        // Special characters
        assert!(CommandPalette::fuzzy_score("rmrf", "rm -rf").is_some());
    }

    #[test]
    fn fuzzy_score_unicode() {
        // Unicode characters
        assert!(CommandPalette::fuzzy_score("caf", "Café").is_some());
        assert!(CommandPalette::fuzzy_score("日本語", "日本語テスト").is_some());
    }
}
