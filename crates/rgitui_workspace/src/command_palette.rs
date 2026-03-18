use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    KeyDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

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
    query_editor: Entity<TextInput>,
    commands: Vec<PaletteCommand>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    scroll_handle: UniformListScrollHandle,
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

        let query_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Type a command...");
            ti
        });

        cx.subscribe(&query_editor, |this: &mut Self, _, event: &TextInputEvent, cx| {
            match event {
                TextInputEvent::Changed(_) => {
                    this.update_filter(cx);
                    cx.notify();
                }
                TextInputEvent::Submit => {
                    this.select_current(cx);
                }
            }
        })
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

    fn update_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query_editor.read(cx).text().to_lowercase();
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
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
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
        let colors = cx.colors();

        if !self.visible {
            return div().id("command-palette").into_any_element();
        }

        let query_is_empty = self.query_editor.read(cx).is_empty();
        let filtered_count = self.filtered_indices.len();

        // Modal container
        let mut modal = div()
            .id("command-palette-modal")
            .track_focus(&self.focus_handle)
            .key_context("CommandPalette")
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
                    div().flex_1().child(self.query_editor.clone()),
                )
                .when(!query_is_empty, |el| {
                    let count_text: SharedString =
                        format!("{} results", filtered_count).into();
                    el.child(
                        Label::new(count_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                }),
        );

        // Results list via uniform_list for scroll-to-item support
        if filtered_count > 0 {
            let selected_index = self.selected_index;
            let selected_bg = colors.ghost_element_selected;
            let hover_bg = colors.ghost_element_hover;

            let commands: Arc<Vec<PaletteCommand>> = Arc::new(self.commands.clone());
            let filtered: Arc<Vec<usize>> = Arc::new(self.filtered_indices.clone());
            let view = cx.weak_entity();

            let visible_items = filtered_count.min(10);
            let list_height = visible_items as f32 * 36.0;

            let list = uniform_list(
                "palette-results",
                filtered_count,
                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                    range
                        .map(|display_idx| {
                            let cmd_idx = filtered[display_idx];
                            let cmd = &commands[cmd_idx];
                            let is_selected = display_idx == selected_index;

                            let label: SharedString = cmd.label.into();
                            let cmd_id = cmd.id.to_string();
                            let icon = CommandPalette::category_icon(cmd.category);
                            let shortcut = cmd.shortcut;
                            let category: SharedString = cmd.category.into();

                            let view_click = view.clone();

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
                                .on_click(move |_: &ClickEvent, _, cx| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.visible = false;
                                            cx.emit(CommandPaletteEvent::CommandSelected(
                                                cmd_id.clone(),
                                            ));
                                            cx.notify();
                                        })
                                        .ok();
                                });

                            row = row.child(
                                Icon::new(icon)
                                    .size(IconSize::Small)
                                    .color(if is_selected {
                                        Color::Accent
                                    } else {
                                        Color::Muted
                                    }),
                            );

                            row = row.child(
                                div()
                                    .h_flex()
                                    .gap(px(8.))
                                    .items_center()
                                    .child(
                                        Label::new(label)
                                            .size(LabelSize::Small)
                                            .when(is_selected, |l| {
                                                l.weight(gpui::FontWeight::MEDIUM)
                                            }),
                                    )
                                    .child(
                                        Label::new(category)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            );

                            row = row.child(div().flex_1());

                            if let Some(shortcut_text) = shortcut {
                                row = row.child(
                                    div()
                                        .h_flex()
                                        .gap(px(3.))
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .child(
                                            Label::new(SharedString::from(shortcut_text))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
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

            modal = modal.child(list);
        } else {
            modal = modal.child(
                div()
                    .id("palette-empty")
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
