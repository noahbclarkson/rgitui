use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize};

/// Events emitted by the rename dialog.
#[derive(Debug, Clone)]
pub enum RenameDialogEvent {
    Rename {
        old_name: String,
        new_name: String,
    },
    Dismissed,
}

/// A modal dialog for renaming a Git branch.
pub struct RenameDialog {
    old_name: String,
    new_name: String,
    error_message: Option<String>,
    visible: bool,
    cursor_pos: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<RenameDialogEvent> for RenameDialog {}

impl RenameDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            old_name: String::new(),
            new_name: String::new(),
            error_message: None,
            visible: false,
            cursor_pos: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog for renaming the given branch.
    pub fn show_visible(&mut self, old_name: String, cx: &mut Context<Self>) {
        self.old_name = old_name.clone();
        self.new_name = old_name;
        self.cursor_pos = self.new_name.len();
        self.error_message = None;
        self.visible = true;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.old_name.clear();
        self.new_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        cx.emit(RenameDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn validate(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Branch name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Branch name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") || name.contains("//") {
            return Some("Cannot contain '..' or '//'".to_string());
        }
        if name.contains("~")
            || name.contains("^")
            || name.contains(":")
            || name.contains("\\")
            || name.contains("?")
            || name.contains("*")
            || name.contains("[")
        {
            return Some("Contains invalid characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Contains control characters".to_string());
        }
        if name.contains("@{") || name == "@" {
            return Some("Invalid ref name".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Cannot end with '.lock'".to_string());
        }
        None
    }

    fn try_rename(&mut self, cx: &mut Context<Self>) {
        if self.new_name == self.old_name {
            self.dismiss(cx);
            return;
        }
        if let Some(err) = Self::validate(&self.new_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        let old = self.old_name.clone();
        let new = self.new_name.clone();
        self.visible = false;
        self.old_name.clear();
        self.new_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        cx.emit(RenameDialogEvent::Rename {
            old_name: old,
            new_name: new,
        });
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();

        match key {
            "escape" => self.dismiss(cx),
            "enter" => self.try_rename(cx),
            "backspace" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.new_name.remove(self.cursor_pos);
                    self.error_message = if self.new_name.is_empty() {
                        None
                    } else {
                        Self::validate(&self.new_name)
                    };
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.new_name.len() {
                    self.new_name.remove(self.cursor_pos);
                    self.error_message = if self.new_name.is_empty() {
                        None
                    } else {
                        Self::validate(&self.new_name)
                    };
                    cx.notify();
                }
            }
            "left" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    cx.notify();
                }
            }
            "right" => {
                if self.cursor_pos < self.new_name.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.new_name.len();
                cx.notify();
            }
            _ => {
                if let Some(key_char) = &keystroke.key_char {
                    self.new_name.insert_str(self.cursor_pos, key_char);
                    self.cursor_pos += key_char.len();
                    self.error_message = Self::validate(&self.new_name);
                    cx.notify();
                } else if key.len() == 1
                    && !keystroke.modifiers.control
                    && !keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() {
                        self.new_name.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                        self.error_message = Self::validate(&self.new_name);
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for RenameDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("rename-dialog").into_any_element();
        }

        // Build the input display with cursor
        let (before_cursor, cursor_char, after_cursor) = if self.new_name.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            let before = self.new_name[..self.cursor_pos].to_string();
            let cursor = if self.cursor_pos < self.new_name.len() {
                self.new_name[self.cursor_pos..self.cursor_pos + 1].to_string()
            } else {
                String::new()
            };
            let after = if self.cursor_pos + 1 < self.new_name.len() {
                self.new_name[self.cursor_pos + 1..].to_string()
            } else {
                String::new()
            };
            (before, cursor, after)
        };

        let is_empty = self.new_name.is_empty();
        let has_error = self.error_message.is_some();
        let is_unchanged = self.new_name == self.old_name;

        let input_border_color = if has_error {
            colors.vc_deleted
        } else {
            colors.border_focused
        };

        let mut input_row = div().h_flex().items_center().w_full();

        if is_empty {
            input_row = input_row
                .child(div().w(px(2.)).h(px(16.)).bg(colors.text))
                .child(
                    Label::new("new-branch-name")
                        .size(LabelSize::Small)
                        .color(Color::Placeholder),
                );
        } else {
            if !before_cursor.is_empty() {
                input_row = input_row
                    .child(Label::new(SharedString::from(before_cursor)).size(LabelSize::Small));
            }
            if !cursor_char.is_empty() {
                input_row = input_row.child(
                    div().bg(colors.text).child(
                        Label::new(SharedString::from(cursor_char))
                            .size(LabelSize::Small)
                            .color(Color::Custom(gpui::Hsla {
                                h: 0.0,
                                s: 0.0,
                                l: 0.0,
                                a: 1.0,
                            })),
                    ),
                );
            } else {
                input_row = input_row.child(div().w(px(2.)).h(px(16.)).bg(colors.text));
            }
            if !after_cursor.is_empty() {
                input_row = input_row
                    .child(Label::new(SharedString::from(after_cursor)).size(LabelSize::Small));
            }
        }

        let old_name: SharedString = self.old_name.clone().into();

        let mut modal = div()
            .id("rename-dialog-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(420.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .elevation_3(cx)
            .p_4()
            .gap_3()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        // Title
        modal = modal.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(IconName::Edit)
                        .size(IconSize::Medium)
                        .color(Color::Accent),
                )
                .child(
                    Label::new("Rename Branch")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Default),
                ),
        );

        // Current name badge
        modal = modal.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    Label::new("Current name")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .h(px(20.))
                        .px(px(8.))
                        .gap(px(4.))
                        .rounded(px(4.))
                        .bg(colors.ghost_element_selected)
                        .items_center()
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(old_name)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::MEDIUM)
                                .color(Color::Muted),
                        ),
                ),
        );

        // New name input
        modal = modal.child(
            div()
                .v_flex()
                .gap_1()
                .child(
                    Label::new("New name")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .h(px(30.))
                        .px_2()
                        .bg(colors.editor_background)
                        .border_1()
                        .border_color(input_border_color)
                        .rounded_md()
                        .items_center()
                        .child(input_row),
                ),
        );

        // Error message
        if let Some(ref err) = self.error_message {
            modal = modal.child(
                Label::new(SharedString::from(err.clone()))
                    .size(LabelSize::XSmall)
                    .color(Color::Deleted),
            );
        }

        // Buttons
        let can_rename = !is_empty && !has_error && !is_unchanged;
        modal = modal.child(
            div()
                .h_flex()
                .justify_between()
                .items_center()
                .child(
                    Label::new("Enter to rename · Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("cancel-rename", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("do-rename", "Rename")
                                .icon(IconName::Edit)
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Filled)
                                .color(Color::Accent)
                                .disabled(!can_rename)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.try_rename(cx);
                                })),
                        ),
                ),
        );

        div()
            .id("rename-dialog-backdrop")
            .occlude().absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.5,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}
