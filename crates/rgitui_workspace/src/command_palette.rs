use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Label, LabelSize};

/// A command that can be executed from the palette.
#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub category: &'static str,
}

/// Events from the command palette.
#[derive(Debug, Clone)]
pub enum CommandPaletteEvent {
    CommandSelected(String),
    Dismissed,
}

/// The command palette modal.
pub struct CommandPalette {
    visible: bool,
    query: String,
    commands: Vec<PaletteCommand>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let commands = vec![
            PaletteCommand {
                id: "fetch",
                label: "Git: Fetch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "pull",
                label: "Git: Pull",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "push",
                label: "Git: Push",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "commit",
                label: "Git: Commit",
                shortcut: Some("Ctrl+Enter"),
                category: "Git",
            },
            PaletteCommand {
                id: "stage_all",
                label: "Git: Stage All",
                shortcut: Some("Ctrl+S"),
                category: "Git",
            },
            PaletteCommand {
                id: "unstage_all",
                label: "Git: Unstage All",
                shortcut: Some("Ctrl+Shift+S"),
                category: "Git",
            },
            PaletteCommand {
                id: "stash_save",
                label: "Git: Stash",
                shortcut: Some("Ctrl+Z"),
                category: "Git",
            },
            PaletteCommand {
                id: "stash_pop",
                label: "Git: Pop Stash",
                shortcut: Some("Ctrl+Shift+Z"),
                category: "Git",
            },
            PaletteCommand {
                id: "create_branch",
                label: "Git: Create Branch",
                shortcut: Some("Ctrl+B"),
                category: "Git",
            },
            PaletteCommand {
                id: "toggle_diff_mode",
                label: "View: Toggle Diff Mode",
                shortcut: None,
                category: "View",
            },
            PaletteCommand {
                id: "ai_message",
                label: "AI: Generate Commit Message",
                shortcut: None,
                category: "AI",
            },
            PaletteCommand {
                id: "refresh",
                label: "Git: Refresh",
                shortcut: Some("F5"),
                category: "Git",
            },
            PaletteCommand {
                id: "interactive_rebase",
                label: "Git: Interactive Rebase",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "settings",
                label: "Preferences: Open Settings",
                shortcut: Some("Ctrl+,"),
                category: "Preferences",
            },
            PaletteCommand {
                id: "merge_branch",
                label: "Git: Merge Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "delete_branch",
                label: "Git: Delete Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "open_repo",
                label: "File: Open Repository",
                shortcut: Some("Ctrl+O"),
                category: "File",
            },
            PaletteCommand {
                id: "workspace_home",
                label: "Workspace: Home",
                shortcut: Some("Ctrl+Shift+W"),
                category: "Workspace",
            },
            PaletteCommand {
                id: "restore_last_workspace",
                label: "Workspace: Restore Last Workspace",
                shortcut: None,
                category: "Workspace",
            },
            PaletteCommand {
                id: "shortcuts",
                label: "Help: Keyboard Shortcuts",
                shortcut: Some("?"),
                category: "Help",
            },
            PaletteCommand {
                id: "search",
                label: "View: Search Commits",
                shortcut: Some("Ctrl+F"),
                category: "View",
            },
            PaletteCommand {
                id: "create_tag",
                label: "Git: Create Tag",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "rename_branch",
                label: "Git: Rename Branch",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "stash_drop",
                label: "Git: Drop Stash",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "stash_apply",
                label: "Git: Apply Stash (keep)",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "cherry_pick",
                label: "Git: Cherry-pick Commit",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "revert_commit",
                label: "Git: Revert Commit",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "force_push",
                label: "Git: Force Push",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "discard_all",
                label: "Git: Discard All Changes",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "abort_operation",
                label: "Git: Abort Merge/Rebase",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "continue_merge",
                label: "Git: Continue Merge",
                shortcut: None,
                category: "Git",
            },
            PaletteCommand {
                id: "reset_hard",
                label: "Git: Reset Hard (to HEAD)",
                shortcut: None,
                category: "Git",
            },
        ];

        let filtered_indices = (0..commands.len()).collect();

        Self {
            visible: false,
            query: String::new(),
            commands,
            filtered_indices,
            selected_index: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.query.clear();
            self.selected_index = 0;
            self.update_filter();
            self.focus_handle.focus(window, cx);
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

    fn update_filter(&mut self) {
        let query = self.query.to_lowercase();
        if query.is_empty() {
            self.filtered_indices = (0..self.commands.len()).collect();
        } else {
            self.filtered_indices = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| {
                    cmd.label.to_lowercase().contains(&query)
                        || cmd.id.to_lowercase().contains(&query)
                        || cmd.category.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_index = 0;
    }

    fn select_current(&mut self, cx: &mut Context<Self>) {
        if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
            let cmd_id = self.commands[idx].id.to_string();
            self.visible = false;
            cx.emit(CommandPaletteEvent::CommandSelected(cmd_id));
            cx.notify();
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
            }
            "enter" => {
                self.select_current(cx);
            }
            "up" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    cx.notify();
                }
            }
            "down" => {
                if self.selected_index + 1 < self.filtered_indices.len() {
                    self.selected_index += 1;
                    cx.notify();
                }
            }
            "backspace" => {
                if !self.query.is_empty() {
                    self.query.pop();
                    self.update_filter();
                    cx.notify();
                }
            }
            _ => {
                if let Some(key_char) = &event.keystroke.key_char {
                    self.query.push_str(key_char);
                    self.update_filter();
                    cx.notify();
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() || ch == ' ' {
                        self.query.push(ch);
                        self.update_filter();
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("command-palette").into_any_element();
        }

        let query_display: SharedString = if self.query.is_empty() {
            "Type a command...".into()
        } else {
            self.query.clone().into()
        };

        let query_color = if self.query.is_empty() {
            Color::Placeholder
        } else {
            Color::Default
        };

        let mut palette = div()
            .id("command-palette-backdrop")
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
            }));

        let mut modal = div()
            .id("command-palette-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(500.))
            .max_h(px(400.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .elevation_3(cx)
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        // Search input
        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border_variant)
                .child(Label::new(">").size(LabelSize::Small).color(Color::Accent))
                .child(
                    div().flex_1().pl_2().child(
                        Label::new(query_display)
                            .size(LabelSize::Small)
                            .color(query_color),
                    ),
                ),
        );

        // Results list
        let mut results = div()
            .id("palette-results")
            .v_flex()
            .w_full()
            .overflow_y_scroll()
            .max_h(px(340.));

        for (display_idx, &cmd_idx) in self.filtered_indices.iter().enumerate() {
            let cmd = &self.commands[cmd_idx];
            let label: SharedString = cmd.label.into();
            let is_selected = display_idx == self.selected_index;
            let cmd_id = cmd.id.to_string();

            let mut row = div()
                .id(ElementId::NamedInteger(
                    "palette-cmd".into(),
                    display_idx as u64,
                ))
                .h_flex()
                .w_full()
                .h(px(32.))
                .px_3()
                .gap_2()
                .items_center()
                .cursor_pointer()
                .when(is_selected, |el| el.bg(colors.ghost_element_selected))
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.visible = false;
                    cx.emit(CommandPaletteEvent::CommandSelected(cmd_id.clone()));
                    cx.notify();
                }));

            row = row.child(Label::new(label).size(LabelSize::Small));

            if let Some(shortcut) = cmd.shortcut {
                let sc: SharedString = shortcut.into();
                row = row
                    .child(div().flex_1())
                    .child(Label::new(sc).size(LabelSize::XSmall).color(Color::Muted));
            }

            results = results.child(row);
        }

        if self.filtered_indices.is_empty() {
            results = results.child(
                div().w_full().py_4().flex().justify_center().child(
                    Label::new("No matching commands")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }

        modal = modal.child(results);
        palette = palette.child(modal);

        palette.into_any_element()
    }
}
