//! Workspace-window keyboard handler.
//!
//! Esc precedence: each window owns its Esc handling and Esc never bubbles
//! cross-window. The handler in this file dismisses the topmost overlay or
//! dialog within the workspace window — command palette, branch dialog,
//! confirm dialog, etc.
//!
//! The settings window has its own Esc handler in
//! `SettingsWindow::handle_key_down` (see
//! [`crate::SettingsWindow`]) that calls `window.remove_window()`.
//! Workspace overlays are not visible from there and cannot be dismissed
//! from there.
//!
//! When introducing a new dialog or overlay, decide which window it lives
//! in and add Esc dismissal to that window's handler.
//!
//! The workspace's *global* shortcuts no longer live here: they are declared by
//! `commands!` in [`crate::keymap::registry`] and dispatched as gpui actions, so
//! users can rebind them from `keymap.json`. What is left is the Esc cascade and
//! the view-local and panel-focus shortcuts, which Phase B will migrate too.

use gpui::{ClipboardItem, Context, KeyDownEvent, Window};

use crate::{CommandId, ToastKind};

use super::{BottomPanelMode, FocusedPanel, Workspace};

impl Workspace {
    /// Whether any overlay or dialog that suppresses panel shortcuts is open.
    ///
    /// Also drives the `modal` key context on the workspace root, which is how
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

    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let modifiers = &keystroke.modifiers;

        // Dismiss interactive rebase dialog on Escape
        if key == "escape" && self.overlays.interactive_rebase.read(cx).is_visible() {
            self.overlays.interactive_rebase.update(cx, |ir, cx| {
                ir.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss confirm dialog on Escape
        if key == "escape" && self.dialogs.confirm_dialog.read(cx).is_visible() {
            self.dialogs.confirm_dialog.update(cx, |cd, cx| {
                cd.cancel(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss branch dialog on Escape
        if key == "escape" && self.dialogs.branch_dialog.read(cx).is_visible() {
            self.dialogs.branch_dialog.update(cx, |bd, cx| {
                bd.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss tag dialog on Escape
        if key == "escape" && self.dialogs.tag_dialog.read(cx).is_visible() {
            self.dialogs.tag_dialog.update(cx, |td, cx| {
                td.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss stash branch dialog on Escape
        if key == "escape" && self.dialogs.stash_branch_dialog.read(cx).is_visible() {
            self.dialogs.stash_branch_dialog.update(cx, |d, cx| {
                d.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss worktree dialog on Escape
        if key == "escape" && self.dialogs.worktree_dialog.read(cx).is_visible() {
            self.dialogs.worktree_dialog.update(cx, |wd, cx| {
                wd.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss rename dialog on Escape
        if key == "escape" && self.dialogs.rename_dialog.read(cx).is_visible() {
            self.dialogs.rename_dialog.update(cx, |rd, cx| {
                rd.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss repo opener on Escape
        if key == "escape" && self.overlays.repo_opener.read(cx).is_visible() {
            self.overlays.repo_opener.update(cx, |ro, cx| {
                ro.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss shortcuts help on Escape
        if key == "escape" && self.overlays.shortcuts_help.read(cx).is_visible() {
            self.overlays.shortcuts_help.update(cx, |sh, cx| {
                sh.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // Dismiss theme editor on Escape
        if key == "escape" && self.overlays.theme_editor.read(cx).is_visible() {
            self.overlays.theme_editor.update(cx, |te, cx| {
                te.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

        // When an overlay is active, only allow Escape (above) plus the
        // rebindable global actions, whose `!modal` context handles this for
        // them. Block all panel-specific shortcuts (j/k, Alt+1/2/3/4, Tab,
        // resize, etc.)
        // TODO(audit): QUAL-10 (phase B) — the remaining per-view
        // on_key_down string-matchers should also migrate to `commands!` +
        // KeyBinding, letting the focus/key_context tree resolve overlay
        // precedence instead of this manual `any_overlay_active` gate.
        let any_overlay_active = self.any_overlay_active(cx);

        // Ctrl+Shift+F to toggle global search
        if !any_overlay_active && modifiers.secondary() && modifiers.shift && key == "f" {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                if tab.bottom_panel_mode == BottomPanelMode::GlobalSearch {
                    tab.global_search_view
                        .update(cx, |search, cx| search.hide(cx));
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                } else {
                    tab.bottom_panel_mode = BottomPanelMode::GlobalSearch;
                    tab.global_search_view.update(cx, |search, cx| {
                        search.show(window, cx);
                    });
                }
                cx.notify();
            }
            return;
        }

        // Ctrl+F to toggle graph search
        if !any_overlay_active && modifiers.secondary() && !modifiers.shift && key == "f" {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                graph.update(cx, |g, cx| {
                    g.toggle_search_focused(window, cx);
                });
            }
            return;
        }

        // / to start graph search
        if !any_overlay_active && key == "/" {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                graph.update(cx, |g, cx| {
                    g.toggle_search_focused(window, cx);
                });
            }
            return;
        }

        // j/k vim-style navigation in the commit graph (skip when graph or detail panel
        // is focused, since they handle their own j/k to avoid double-movement)
        if !any_overlay_active
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            let panel_has_focus = self
                .tabs
                .get(self.active_tab)
                .map(|tab| {
                    tab.graph.read(cx).is_focused(window)
                        || tab.detail_panel.read(cx).is_focused(window)
                        || tab.diff_viewer.read(cx).is_focused(window)
                        || tab.blame_view.read(cx).is_focused(window)
                })
                .unwrap_or(false);

            if !panel_has_focus {
                match key {
                    "j" => {
                        if let Some(tab) = self.tabs.get(self.active_tab) {
                            let graph = tab.graph.clone();
                            graph.update(cx, |g, cx| {
                                let next = g
                                    .selected_index()
                                    .map(|i| (i + 1).min(g.row_count().saturating_sub(1)))
                                    .unwrap_or(0);
                                g.select_index(next, cx);
                            });
                        }
                    }
                    "k" => {
                        if let Some(tab) = self.tabs.get(self.active_tab) {
                            let graph = tab.graph.clone();
                            graph.update(cx, |g, cx| {
                                if let Some(i) = g.selected_index() {
                                    if i > 0 {
                                        g.select_index(i - 1, cx);
                                    }
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        // 'd' to switch to diff view (from blame/history)
        if !any_overlay_active
            && key == "d"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            let sidebar_has_focus = self
                .tabs
                .get(self.active_tab)
                .map(|tab| tab.sidebar.read(cx).is_focused(window))
                .unwrap_or(false);
            if !sidebar_has_focus {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    if tab.bottom_panel_mode != BottomPanelMode::Diff {
                        tab.bottom_panel_mode = BottomPanelMode::Diff;
                        cx.notify();
                        return;
                    }
                }
            }
        }
        // Shift+D to toggle diff display mode (unified/side-by-side)
        if !any_overlay_active
            && key == "d"
            && !modifiers.control
            && !modifiers.alt
            && modifiers.shift
            && !modifiers.platform
        {
            self.execute_command(CommandId::ToggleDiffMode, cx);
            return;
        }

        // 'b' to toggle blame view (not when sidebar has focus — user might be typing)
        if !any_overlay_active
            && key == "b"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            let sidebar_focused = self
                .tabs
                .get(self.active_tab)
                .map(|tab| tab.sidebar.read(cx).is_focused(window))
                .unwrap_or(false);
            if !sidebar_focused {
                self.execute_command(CommandId::Blame, cx);
                return;
            }
        }

        // 'y' to copy SHA of selected commit
        if !any_overlay_active
            && key == "y"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                if let Some(commit) = graph.read(cx).selected_commit() {
                    let sha = commit.oid.to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
                    let short = &sha[..7.min(sha.len())];
                    self.show_toast(format!("Copied SHA: {}", short), ToastKind::Success, cx);
                }
            }
            return;
        }

        // 'Shift+C' to copy commit message of selected commit
        if !any_overlay_active
            && key == "c"
            && !modifiers.control
            && !modifiers.alt
            && modifiers.shift
            && !modifiers.platform
        {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                if let Some(commit) = graph.read(cx).selected_commit() {
                    let msg = commit.message.clone();
                    cx.write_to_clipboard(ClipboardItem::new_string(msg.clone()));
                    let first_line = msg.lines().next().unwrap_or(&msg);
                    let preview = if first_line.chars().count() > 40 {
                        format!("{}...", first_line.chars().take(40).collect::<String>())
                    } else {
                        first_line.to_string()
                    };
                    self.show_toast(format!("Copied: {}", preview), ToastKind::Success, cx);
                }
            }
            return;
        }

        // 'h' to toggle file history view for selected file
        if !any_overlay_active
            && key == "h"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            self.execute_command(CommandId::FileHistory, cx);
            return;
        }

        // Ctrl+[ / Ctrl+] to resize detail panel width
        if !any_overlay_active && modifiers.secondary() && !modifiers.shift && !modifiers.alt {
            match key {
                "[" | "bracketleft" => {
                    self.layout.detail_panel_width = (self.layout.detail_panel_width - 20.0)
                        .max(super::layout::MIN_DETAIL_PANEL_WIDTH);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "]" | "bracketright" => {
                    self.layout.detail_panel_width = (self.layout.detail_panel_width + 20.0)
                        .min(super::layout::MAX_DETAIL_PANEL_WIDTH);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                // Ctrl+Up / Ctrl+Down to resize diff viewer height
                "up" => {
                    self.layout.diff_viewer_height = (self.layout.diff_viewer_height - 30.0)
                        .max(super::layout::MIN_DIFF_VIEWER_HEIGHT);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "down" => {
                    self.layout.diff_viewer_height = (self.layout.diff_viewer_height + 30.0)
                        .min(super::layout::MAX_DIFF_VIEWER_HEIGHT);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                _ => {}
            }
        }

        // Alt+1/2/3/4 to focus sidebar/graph/detail/diff panel
        if !any_overlay_active && modifiers.alt && !modifiers.secondary() {
            match key {
                "1" => {
                    self.focus_panel(FocusedPanel::Sidebar, window, cx);
                    return;
                }
                "2" => {
                    self.focus_panel(FocusedPanel::Graph, window, cx);
                    return;
                }
                "3" => {
                    self.focus_panel(FocusedPanel::DetailPanel, window, cx);
                    return;
                }
                "4" => {
                    self.focus_panel(FocusedPanel::DiffViewer, window, cx);
                    return;
                }
                _ => {}
            }
        }

        // Tab / Shift+Tab to cycle between panels (only when no overlay is active)
        if !any_overlay_active && !modifiers.secondary() && !modifiers.alt && key == "tab" {
            if modifiers.shift {
                self.focus_prev_panel(window, cx);
            } else {
                self.focus_next_panel(window, cx);
            }
        }
    }
}
