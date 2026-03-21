use gpui::{Bounds, Entity, Pixels};
use rgitui_git::GitOperationUpdate;

use crate::{
    BranchDialog, CommandPalette, ConfirmDialog, InteractiveRebase, RenameDialog, RepoOpener,
    SettingsModal, ShortcutsHelp, TagDialog,
};

use super::{ActiveOperation, FocusedPanel, OperationOutput};

/// Layout dimensions for resizable panels.
pub(crate) struct LayoutState {
    pub sidebar_width: f32,
    pub detail_panel_width: f32,
    pub diff_viewer_height: f32,
    pub commit_input_height: f32,
    pub content_bounds: Bounds<Pixels>,
    pub right_panel_bounds: Bounds<Pixels>,
}

/// Modal dialog entities.
pub(crate) struct DialogState {
    pub branch_dialog: Entity<BranchDialog>,
    pub tag_dialog: Entity<TagDialog>,
    pub rename_dialog: Entity<RenameDialog>,
    pub confirm_dialog: Entity<ConfirmDialog>,
}

/// Overlay entities (command palette, settings, etc).
pub(crate) struct OverlayState {
    pub command_palette: Entity<CommandPalette>,
    pub interactive_rebase: Entity<InteractiveRebase>,
    pub settings_modal: Entity<SettingsModal>,
    pub repo_opener: Entity<RepoOpener>,
    pub shortcuts_help: Entity<ShortcutsHelp>,
}

/// Git operation tracking state.
pub(crate) struct OperationState {
    pub active_git_operation: Option<GitOperationUpdate>,
    pub last_failed_git_operation: Option<GitOperationUpdate>,
    pub active_operations: Vec<ActiveOperation>,
    pub last_operation_output: Option<OperationOutput>,
    pub is_loading: bool,
    pub loading_message: Option<String>,
}

/// Focus management state.
pub(crate) struct FocusState {
    pub last_focused_panel: Option<FocusedPanel>,
    pub pending_focus_restore: bool,
    pub crash_recovery_available: bool,
    pub crash_recovery_shown: bool,
}
