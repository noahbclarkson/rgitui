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
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: CommandId,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub category: &'static str,
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
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let commands = vec![
            PaletteCommand {
                id: CommandId::Fetch,
                label: "Git: Fetch",
                shortcut: Some("Ctrl+Shift+F"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::Pull,
                label: "Git: Pull",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::Push,
                label: "Git: Push",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::ForcePush,
                label: "Git: Force Push",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::Commit,
                label: "Git: Commit",
                shortcut: Some("Ctrl+Enter"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::StageAll,
                label: "Git: Stage All",
                shortcut: Some("Ctrl+S"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::UnstageAll,
                label: "Git: Unstage All",
                shortcut: Some("Ctrl+U"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::StashSave,
                label: "Git: Stash",
                shortcut: Some("Ctrl+Z"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::StashPop,
                label: "Git: Pop Stash",
                shortcut: Some("Ctrl+Shift+Z"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::StashApply,
                label: "Git: Apply Stash (keep)",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::StashDrop,
                label: "Git: Drop Stash",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::CreateBranch,
                label: "Git: Create Branch",
                shortcut: Some("Ctrl+B"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::SwitchBranch,
                label: "Git: Switch Branch",
                shortcut: Some("Ctrl+Shift+B"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::DeleteBranch,
                label: "Git: Delete Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::RenameBranch,
                label: "Git: Rename Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::MergeBranch,
                label: "Git: Merge Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::CreateTag,
                label: "Git: Create Tag",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::CherryPick,
                label: "Git: Cherry-pick Commit",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::RevertCommit,
                label: "Git: Revert Commit",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::InteractiveRebase,
                label: "Git: Interactive Rebase",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::DiscardAll,
                label: "Git: Discard All Changes",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::ResetHard,
                label: "Git: Reset Hard (to HEAD)",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::AbortOperation,
                label: "Git: Abort Merge/Rebase",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::ContinueMerge,
                label: "Git: Continue Merge",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::ToggleDiffMode,
                label: "View: Toggle Diff Mode",
                shortcut: Some("d"),
                category: "View",
            },
            PaletteCommand {
                id: CommandId::Search,
                label: "View: Search Commits",
                shortcut: Some("Ctrl+F"),
                category: "View",
            },
            PaletteCommand {
                id: CommandId::AiMessage,
                label: "AI: Generate Commit Message",
                shortcut: Some("Ctrl+G"),
                category: "AI",
            },
            PaletteCommand {
                id: CommandId::Refresh,
                label: "Git: Refresh",
                shortcut: Some("F5"),
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::Settings,
                label: "Preferences: Open Settings",
                shortcut: Some("Ctrl+,"),
                category: "Preferences",
            },
            PaletteCommand {
                id: CommandId::OpenRepo,
                label: "File: Open Repository",
                shortcut: Some("Ctrl+O"),
                category: "File",
            },
            PaletteCommand {
                id: CommandId::WorkspaceHome,
                label: "Workspace: Home",
                shortcut: None,
                category: "Workspace",
            },
            PaletteCommand {
                id: CommandId::RestoreLastWorkspace,
                label: "Workspace: Restore Last",
                shortcut: None,
                category: "Workspace",
            },
            PaletteCommand {
                id: CommandId::Shortcuts,
                label: "Help: Keyboard Shortcuts",
                shortcut: Some("?"),
                category: "Help",
            },
            PaletteCommand {
                id: CommandId::Blame,
                label: "View: Blame File",
                shortcut: Some("b"),
                category: "View",
            },
            PaletteCommand {
                id: CommandId::FileHistory,
                label: "View: File History",
                shortcut: Some("h"),
                category: "View",
            },
            PaletteCommand {
                id: CommandId::Undo,
                label: "Edit: Undo Last Operation",
                shortcut: None,
                category: "Edit",
            },
            PaletteCommand {
                id: CommandId::BisectStart,
                label: "Git: Bisect Start",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::BisectGood,
                label: "Git: Bisect Good (mark current)",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::BisectBad,
                label: "Git: Bisect Bad (mark current)",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::BisectReset,
                label: "Git: Bisect Reset",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: CommandId::Reflog,
                label: "View: Reflog",
                shortcut: None,
                category: "View",
            },
            PaletteCommand {
                id: CommandId::Submodules,
                label: "View: Submodules",
                shortcut: None,
                category: "View",
            },
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
        if query.is_empty() {
            self.filtered_indices = (0..self.commands.len()).map(|i| (i, 0)).collect();
        } else {
            let mut scored: Vec<(usize, usize)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
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
}
