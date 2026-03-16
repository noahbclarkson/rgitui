use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub category: &'static str,
}

#[derive(Debug, Clone)]
pub enum CommandPaletteEvent {
    CommandSelected(String),
    Dismissed,
}

pub struct CommandPalette {
    visible: bool,
    query: String,
    cursor_pos: usize,
    commands: Vec<PaletteCommand>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let commands = vec![
            PaletteCommand { id: "fetch", label: "Git: Fetch", shortcut: Some("Ctrl+Shift+F"), category: "Git" },
            PaletteCommand { id: "pull", label: "Git: Pull", shortcut: None, category: "Git" },
            PaletteCommand { id: "push", label: "Git: Push", shortcut: None, category: "Git" },
            PaletteCommand { id: "force_push", label: "Git: Force Push", shortcut: None, category: "Git" },
            PaletteCommand { id: "commit", label: "Git: Commit", shortcut: Some("Ctrl+Enter"), category: "Git" },
            PaletteCommand { id: "stage_all", label: "Git: Stage All", shortcut: Some("Ctrl+S"), category: "Git" },
            PaletteCommand { id: "unstage_all", label: "Git: Unstage All", shortcut: Some("Ctrl+Shift+S"), category: "Git" },
            PaletteCommand { id: "stash_save", label: "Git: Stash", shortcut: Some("Ctrl+Z"), category: "Git" },
            PaletteCommand { id: "stash_pop", label: "Git: Pop Stash", shortcut: Some("Ctrl+Shift+Z"), category: "Git" },
            PaletteCommand { id: "stash_apply", label: "Git: Apply Stash (keep)", shortcut: None, category: "Git" },
            PaletteCommand { id: "stash_drop", label: "Git: Drop Stash", shortcut: None, category: "Git" },
            PaletteCommand { id: "create_branch", label: "Git: Create Branch", shortcut: Some("Ctrl+B"), category: "Git" },
            PaletteCommand { id: "delete_branch", label: "Git: Delete Branch", shortcut: None, category: "Git" },
            PaletteCommand { id: "rename_branch", label: "Git: Rename Branch", shortcut: None, category: "Git" },
            PaletteCommand { id: "merge_branch", label: "Git: Merge Branch", shortcut: None, category: "Git" },
            PaletteCommand { id: "create_tag", label: "Git: Create Tag", shortcut: None, category: "Git" },
            PaletteCommand { id: "cherry_pick", label: "Git: Cherry-pick Commit", shortcut: None, category: "Git" },
            PaletteCommand { id: "revert_commit", label: "Git: Revert Commit", shortcut: None, category: "Git" },
            PaletteCommand { id: "interactive_rebase", label: "Git: Interactive Rebase", shortcut: None, category: "Git" },
            PaletteCommand { id: "discard_all", label: "Git: Discard All Changes", shortcut: None, category: "Git" },
            PaletteCommand { id: "reset_hard", label: "Git: Reset Hard (to HEAD)", shortcut: None, category: "Git" },
            PaletteCommand { id: "abort_operation", label: "Git: Abort Merge/Rebase", shortcut: None, category: "Git" },
            PaletteCommand { id: "continue_merge", label: "Git: Continue Merge", shortcut: None, category: "Git" },
            PaletteCommand { id: "toggle_diff_mode", label: "View: Toggle Diff Mode", shortcut: Some("d"), category: "View" },
            PaletteCommand { id: "search", label: "View: Search Commits", shortcut: Some("Ctrl+F"), category: "View" },
            PaletteCommand { id: "ai_message", label: "AI: Generate Commit Message", shortcut: Some("Ctrl+G"), category: "AI" },
            PaletteCommand { id: "refresh", label: "Git: Refresh", shortcut: Some("F5"), category: "Git" },
            PaletteCommand { id: "settings", label: "Preferences: Open Settings", shortcut: Some("Ctrl+,"), category: "Preferences" },
            PaletteCommand { id: "open_repo", label: "File: Open Repository", shortcut: Some("Ctrl+O"), category: "File" },
            PaletteCommand { id: "workspace_home", label: "Workspace: Home", shortcut: None, category: "Workspace" },
            PaletteCommand { id: "restore_last_workspace", label: "Workspace: Restore Last", shortcut: None, category: "Workspace" },
            PaletteCommand { id: "shortcuts", label: "Help: Keyboard Shortcuts", shortcut: Some("?"), category: "Help" },
        ];

        let filtered_indices = (0..commands.len()).collect();

        Self {
            visible: false,
            query: String::new(),
            cursor_pos: 0,
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
            self.cursor_pos = 0;
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

    fn category_icon(category: &str) -> IconName {
        match category {
            "Git" => IconName::GitBranch,
            "View" => IconName::Eye,
            "AI" => IconName::Sparkle,
            "Preferences" => IconName::Settings,
            "File" => IconName::Folder,
            "Workspace" => IconName::Menu,
            "Help" => IconName::Star,
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
            "escape" => self.dismiss(cx),
            "enter" => self.select_current(cx),
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
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.query.remove(self.cursor_pos);
                    self.update_filter();
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.query.len();
                cx.notify();
            }
            "left" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    cx.notify();
                }
            }
            "right" => {
                if self.cursor_pos < self.query.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            _ => {
                if let Some(key_char) = &event.keystroke.key_char {
                    self.query.insert_str(self.cursor_pos, key_char);
                    self.cursor_pos += key_char.len();
                    self.update_filter();
                    cx.notify();
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() || ch == ' ' {
                        self.query.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
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

        // Build query display with cursor
        let query_display: SharedString = if self.query.is_empty() {
            "Type a command...".into()
        } else {
            let mut display = self.query.clone();
            let pos = self.cursor_pos.min(display.len());
            display.insert(pos, '|');
            display.into()
        };
        let query_color = if self.query.is_empty() {
            Color::Placeholder
        } else {
            Color::Default
        };

        // Modal container
        let mut modal = div()
            .id("command-palette-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(560.))
            .max_h(px(440.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.))
            .elevation_3(cx)
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

        // Search input row
        let search_bg = gpui::Hsla { a: 0.5, ..colors.surface_background };
        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .h(px(44.))
                .px(px(14.))
                .gap(px(10.))
                .items_center()
                .bg(search_bg)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div().flex_1().child(
                        Label::new(query_display)
                            .size(LabelSize::default())
                            .color(query_color),
                    ),
                )
                .when(!self.query.is_empty(), |el| {
                    let count_text: SharedString =
                        format!("{} results", self.filtered_indices.len()).into();
                    el.child(
                        Label::new(count_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                }),
        );

        // Results list
        let mut results = div()
            .id("palette-results")
            .v_flex()
            .w_full()
            .overflow_y_scroll()
            .flex_1()
            .py_1();

        let mut last_category: Option<&str> = None;

        for (display_idx, &cmd_idx) in self.filtered_indices.iter().enumerate() {
            let cmd = &self.commands[cmd_idx];
            let is_selected = display_idx == self.selected_index;

            // Category separator
            if self.query.is_empty() && last_category != Some(cmd.category) {
                last_category = Some(cmd.category);
                if display_idx > 0 {
                    results = results.child(
                        div()
                            .w_full()
                            .h(px(1.))
                            .mx(px(10.))
                            .my(px(4.))
                            .bg(colors.border_variant),
                    );
                }
                results = results.child(
                    div()
                        .w_full()
                        .px(px(14.))
                        .pt(px(6.))
                        .pb(px(2.))
                        .child(
                            Label::new(SharedString::from(cmd.category))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                );
            }

            let label: SharedString = cmd.label.into();
            let cmd_id = cmd.id.to_string();
            let icon = Self::category_icon(cmd.category);
            let selected_bg = colors.ghost_element_selected;
            let hover_bg = colors.ghost_element_hover;

            let mut row = div()
                .id(ElementId::NamedInteger(
                    "palette-cmd".into(),
                    display_idx as u64,
                ))
                .h_flex()
                .w_full()
                .h(px(36.))
                .px(px(10.))
                .mx(px(4.))
                .gap(px(10.))
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(is_selected, move |el| el.bg(selected_bg))
                .hover(move |s| s.bg(hover_bg))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.visible = false;
                    cx.emit(CommandPaletteEvent::CommandSelected(cmd_id.clone()));
                    cx.notify();
                }));

            row = row.child(
                Icon::new(icon)
                    .size(IconSize::Small)
                    .color(if is_selected { Color::Accent } else { Color::Muted }),
            );

            row = row.child(
                Label::new(label)
                    .size(LabelSize::Small)
                    .when(is_selected, |l| l.weight(gpui::FontWeight::MEDIUM)),
            );

            row = row.child(div().flex_1());

            if let Some(shortcut) = cmd.shortcut {
                let shortcut_bg = gpui::Hsla { a: 0.08, ..colors.text };
                let shortcut_border = gpui::Hsla { a: 0.15, ..colors.text };
                row = row.child(
                    div()
                        .h_flex()
                        .gap(px(3.))
                        .px(px(6.))
                        .py(px(2.))
                        .bg(shortcut_bg)
                        .border_1()
                        .border_color(shortcut_border)
                        .rounded(px(4.))
                        .child(
                            Label::new(SharedString::from(shortcut))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                );
            }

            results = results.child(row);
        }

        if self.filtered_indices.is_empty() {
            results = results.child(
                div()
                    .w_full()
                    .py(px(24.))
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

        modal = modal.child(results);

        // Footer hint
        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .h(px(30.))
                .px(px(14.))
                .gap(px(12.))
                .items_center()
                .border_t_1()
                .border_color(colors.border_variant)
                .bg(search_bg)
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
            .pt(px(60.))
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
