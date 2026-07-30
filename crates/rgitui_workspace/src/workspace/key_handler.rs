//! How the workspace window resolves keystrokes.
//!
//! There is no keyboard handler here any more. Every shortcut is declared by
//! `commands!` in [`crate::keymap::registry`], bound to a gpui action scoped to a
//! key context, and handled by an `on_action` listener on the element that owns
//! the behaviour. Two mechanisms replace what used to be hand-rolled here:
//!
//! * **Esc and Enter.** `menu::Cancel` and `menu::Confirm` are each bound once.
//!   gpui dispatches an action outwards from the focused element and the first
//!   listener consumes it, so the innermost open overlay, dialog or panel is the
//!   one that responds — no ordered cascade of `if visible` checks. A view that
//!   does not want a command calls `cx.propagate()` and it carries on outwards,
//!   ending at the focused text field. Blame and file history deliberately keep
//!   their own `escape` binding, because there Esc is a navigation (back to the
//!   diff) rather than a dismissal, and a deeper context wins.
//!
//! * **Bare letters.** Each is scoped to the view that owns it, and the ones that
//!   collide across views (`d`, `s`, `p`, `b`, `h`) simply appear in more than one
//!   block with different contexts. They also carry `!TextInput`, which gpui
//!   evaluates false whenever a text field is anywhere on the focus path, so a
//!   shortcut can never steal a typed character.
//!
//! When adding a dialog or overlay, give its root element a key context and bind
//! its commands in a `commands!` block; there is nothing to register here.
//!
//! The settings window has its own root context (`SettingsWindow`) and its own
//! `menu::Cancel` listener that calls `window.remove_window()`. Each window owns
//! its dismissal and Esc never crosses windows: workspace overlays are not
//! visible from the settings window and cannot be dismissed from it.

use gpui::Context;

use super::Workspace;

impl Workspace {
    /// Whether any overlay or dialog that suppresses panel shortcuts is open.
    ///
    /// Drives the `modal` key context on the workspace root, which is how
    /// `Workspace && !modal` keeps rebindable global shortcuts from firing while
    /// a modal is up.
    pub(super) fn any_overlay_active(&self, cx: &Context<Self>) -> bool {
        self.overlays.command_palette.read(cx).is_visible()
            || self.overlays.interactive_rebase.read(cx).is_visible()
            || self.overlays.theme_editor.read(cx).is_visible()
            || self.dialogs.branch_dialog.read(cx).is_visible()
            || self.dialogs.tag_dialog.read(cx).is_visible()
            || self.dialogs.worktree_dialog.read(cx).is_visible()
            || self.dialogs.rename_dialog.read(cx).is_visible()
            || self.overlays.repo_opener.read(cx).is_visible()
            || self.dialogs.confirm_dialog.read(cx).is_visible()
            || self.dialogs.stash_branch_dialog.read(cx).is_visible()
            || self.overlays.global_search.read(cx).is_visible()
            || self.overlays.shortcuts_help.read(cx).is_visible()
    }
}
