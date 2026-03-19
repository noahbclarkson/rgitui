use gpui::{Context, KeyDownEvent, Window};

use crate::CommandId;

use super::{FocusedPanel, Workspace};

impl Workspace {
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let modifiers = &keystroke.modifiers;

        // Dismiss settings modal on Escape
        if key == "escape" && self.overlays.settings_modal.read(cx).is_visible() {
            self.overlays.settings_modal.update(cx, |sm, cx| {
                sm.dismiss(cx);
            });
            self.restore_focus(window, cx);
            return;
        }

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

        // When an overlay is active, only allow modal toggle shortcuts (below) and Escape (above).
        // Block all panel-specific shortcuts (j/k, Alt+1/2/3/4, Tab, resize, etc.)
        let any_overlay_active = self.overlays.command_palette.read(cx).is_visible()
            || self.overlays.interactive_rebase.read(cx).is_visible()
            || self.overlays.settings_modal.read(cx).is_visible()
            || self.dialogs.branch_dialog.read(cx).is_visible()
            || self.dialogs.tag_dialog.read(cx).is_visible()
            || self.dialogs.rename_dialog.read(cx).is_visible()
            || self.overlays.repo_opener.read(cx).is_visible()
            || self.dialogs.confirm_dialog.read(cx).is_visible()
            || self.overlays.shortcuts_help.read(cx).is_visible();

        // Ctrl+Shift+F to fetch
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && modifiers.shift
            && key == "f"
        {
            self.execute_command(CommandId::Fetch, cx);
            return;
        }

        // Ctrl+F to toggle graph search
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && key == "f"
        {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                let graph = tab.graph.clone();
                graph.update(cx, |g, cx| {
                    g.toggle_search_focused(window, cx);
                });
            }
            return;
        }

        // Ctrl+G to generate AI commit message
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && key == "g"
        {
            self.execute_command(CommandId::AiMessage, cx);
            return;
        }

        // Ctrl+Shift+P or Cmd+Shift+P to open command palette
        if (modifiers.control || modifiers.platform) && modifiers.shift && key == "p" {
            self.save_focus(window, cx);
            self.overlays.command_palette.update(cx, |cp, cx| {
                cp.toggle(window, cx);
            });
        }

        // Ctrl+, to open settings
        if (modifiers.control || modifiers.platform) && key == "," {
            self.save_focus(window, cx);
            self.overlays.settings_modal.update(cx, |sm, cx| {
                sm.toggle(window, cx);
            });
            return;
        }

        // F5 to refresh
        if !any_overlay_active && key == "f5" {
            self.execute_command(CommandId::Refresh, cx);
        }

        // Ctrl+O to open repo opener
        if (modifiers.control || modifiers.platform) && key == "o" {
            self.save_focus(window, cx);
            self.overlays.repo_opener.update(cx, |ro, cx| {
                ro.toggle(window, cx);
            });
            return;
        }

        // ? to toggle shortcuts help (without modifiers)
        if !any_overlay_active
            && key == "?"
            && !modifiers.control
            && !modifiers.platform
            && !modifiers.alt
        {
            self.save_focus(window, cx);
            self.overlays.shortcuts_help.update(cx, |sh, cx| {
                sh.toggle(window, cx);
            });
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

        // 'd' to toggle diff display mode (unified/side-by-side)
        // Only when not in a text-entry context (sidebar, commit panel don't consume 'd')
        if !any_overlay_active
            && key == "d"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.platform
        {
            // Skip if sidebar has focus (user might be navigating branches)
            let sidebar_has_focus = self
                .tabs
                .get(self.active_tab)
                .map(|tab| tab.sidebar.read(cx).is_focused(window))
                .unwrap_or(false);
            if !sidebar_has_focus {
                self.execute_command(CommandId::ToggleDiffMode, cx);
                return;
            }
        }

        // Ctrl+[ / Ctrl+] to resize detail panel width
        if !any_overlay_active && modifiers.control && !modifiers.shift && !modifiers.alt {
            match key {
                "[" | "bracketleft" => {
                    self.layout.detail_panel_width =
                        (self.layout.detail_panel_width - 20.0).max(180.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "]" | "bracketright" => {
                    self.layout.detail_panel_width =
                        (self.layout.detail_panel_width + 20.0).min(480.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                // Ctrl+Up / Ctrl+Down to resize diff viewer height
                "up" => {
                    self.layout.diff_viewer_height =
                        (self.layout.diff_viewer_height - 30.0).max(100.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                "down" => {
                    self.layout.diff_viewer_height =
                        (self.layout.diff_viewer_height + 30.0).min(600.0);
                    self.schedule_layout_save(cx);
                    cx.notify();
                }
                _ => {}
            }
        }

        // Ctrl+S to stage all
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "s" {
            self.execute_command(CommandId::StageAll, cx);
            return;
        }

        // Ctrl+Shift+S to unstage all
        if !any_overlay_active && modifiers.control && modifiers.shift && key == "s" {
            self.execute_command(CommandId::UnstageAll, cx);
            return;
        }

        // Ctrl+U to unstage all (alternative)
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && key == "u"
        {
            self.execute_command(CommandId::UnstageAll, cx);
            return;
        }

        // Ctrl+B to create branch
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "b" {
            self.execute_command(CommandId::CreateBranch, cx);
            return;
        }

        // Ctrl+Shift+B to switch branch (focus sidebar for branch navigation)
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && modifiers.shift
            && key == "b"
        {
            self.focus_panel(FocusedPanel::Sidebar, window, cx);
            return;
        }

        // Ctrl+Enter to commit
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "enter" {
            self.execute_command(CommandId::Commit, cx);
            return;
        }

        // Ctrl+Z to stash save
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "z" {
            self.execute_command(CommandId::StashSave, cx);
            return;
        }

        // Ctrl+Shift+Z to stash pop
        if !any_overlay_active && modifiers.control && modifiers.shift && key == "z" {
            self.execute_command(CommandId::StashPop, cx);
            return;
        }

        // Ctrl+Tab to switch to next tab
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "tab" {
            if !self.tabs.is_empty() {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                cx.notify();
            }
            return;
        }

        // Ctrl+Shift+Tab to switch to previous tab
        if !any_overlay_active && modifiers.control && modifiers.shift && key == "tab" {
            if !self.tabs.is_empty() {
                if self.active_tab == 0 {
                    self.active_tab = self.tabs.len() - 1;
                } else {
                    self.active_tab -= 1;
                }
                cx.notify();
            }
            return;
        }

        // Ctrl+W to close current tab
        if !any_overlay_active && modifiers.control && !modifiers.shift && key == "w" {
            if !self.tabs.is_empty() {
                self.close_tab(self.active_tab, cx);
            }
            return;
        }

        // Ctrl+H to return to workspace home
        if !any_overlay_active
            && (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && key == "h"
        {
            self.go_home(cx);
            return;
        }

        // Alt+1/2/3/4 to focus sidebar/graph/detail/diff panel
        if !any_overlay_active && modifiers.alt && !modifiers.control {
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
        if !any_overlay_active && !modifiers.control && !modifiers.alt && key == "tab" {
            if modifiers.shift {
                self.focus_prev_panel(window, cx);
            } else {
                self.focus_next_panel(window, cx);
            }
        }
    }
}
