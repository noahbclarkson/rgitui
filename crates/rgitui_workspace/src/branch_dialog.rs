use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, Label, LabelSize};

/// Events emitted by the branch creation dialog.
#[derive(Debug, Clone)]
pub enum BranchDialogEvent {
    CreateBranch { name: String, base_ref: String },
    Dismissed,
}

/// A modal dialog for creating a new Git branch.
pub struct BranchDialog {
    branch_name: String,
    base_ref: String,
    error_message: Option<String>,
    visible: bool,
    cursor_pos: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<BranchDialogEvent> for BranchDialog {}

impl BranchDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            branch_name: String::new(),
            base_ref: "HEAD".to_string(),
            error_message: None,
            visible: false,
            cursor_pos: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog, optionally setting the base ref (e.g. current branch name).
    pub fn show(&mut self, base_ref: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.branch_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        if let Some(base) = base_ref {
            self.base_ref = base;
        } else {
            self.base_ref = "HEAD".to_string();
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Show the dialog without focusing (for use from contexts where Window is unavailable).
    pub fn show_visible(&mut self, base_ref: Option<String>, cx: &mut Context<Self>) {
        self.visible = true;
        self.branch_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        if let Some(base) = base_ref {
            self.base_ref = base;
        } else {
            self.base_ref = "HEAD".to_string();
        }
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.branch_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        cx.emit(BranchDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Validate the branch name and return an error message if invalid.
    fn validate_branch_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Branch name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Branch name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Branch name cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Branch name cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") {
            return Some("Branch name cannot contain '..'".to_string());
        }
        if name.contains("~") || name.contains("^") || name.contains(":") || name.contains("\\") {
            return Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string());
        }
        if name.contains("?") || name.contains("*") || name.contains("[") {
            return Some("Branch name cannot contain glob characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Branch name cannot contain control characters".to_string());
        }
        if name.contains("@{") {
            return Some("Branch name cannot contain '@{'".to_string());
        }
        if name == "@" {
            return Some("Branch name cannot be '@'".to_string());
        }
        if name.contains("//") {
            return Some("Branch name cannot contain consecutive slashes".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Branch name cannot end with '.lock'".to_string());
        }
        None
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        if let Some(err) = Self::validate_branch_name(&self.branch_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        let name = self.branch_name.clone();
        let base_ref = self.base_ref.clone();
        self.visible = false;
        self.branch_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        cx.emit(BranchDialogEvent::CreateBranch { name, base_ref });
        cx.notify();
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();

        match key {
            "escape" => {
                self.dismiss(cx);
            }
            "enter" => {
                self.try_create(cx);
            }
            "backspace" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.branch_name.remove(self.cursor_pos);
                    self.error_message = None;
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.branch_name.len() {
                    self.branch_name.remove(self.cursor_pos);
                    self.error_message = None;
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
                if self.cursor_pos < self.branch_name.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.branch_name.len();
                cx.notify();
            }
            _ => {
                // Handle character input
                if let Some(key_char) = &keystroke.key_char {
                    self.branch_name.insert_str(self.cursor_pos, key_char);
                    self.cursor_pos += key_char.len();
                    self.error_message = None;
                    cx.notify();
                } else if key.len() == 1
                    && !keystroke.modifiers.control
                    && !keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() {
                        self.branch_name.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                        self.error_message = None;
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for BranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("branch-dialog").into_any_element();
        }

        // Build the input display with cursor
        let (before_cursor, cursor_char, after_cursor) = if self.branch_name.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            let before = self.branch_name[..self.cursor_pos].to_string();
            let cursor = if self.cursor_pos < self.branch_name.len() {
                self.branch_name[self.cursor_pos..self.cursor_pos + 1].to_string()
            } else {
                String::new()
            };
            let after = if self.cursor_pos + 1 < self.branch_name.len() {
                self.branch_name[self.cursor_pos + 1..].to_string()
            } else {
                String::new()
            };
            (before, cursor, after)
        };

        let is_empty = self.branch_name.is_empty();
        let has_error = self.error_message.is_some();

        let input_border_color = if has_error {
            colors.vc_deleted
        } else {
            colors.border_focused
        };

        // Text input field
        let mut input_row = div()
            .h_flex()
            .items_center()
            .w_full();

        if is_empty {
            input_row = input_row
                .child(
                    div()
                        .w(px(2.))
                        .h(px(16.))
                        .bg(colors.text),
                )
                .child(
                    Label::new("feature/my-branch")
                        .size(LabelSize::Small)
                        .color(Color::Placeholder),
                );
        } else {
            if !before_cursor.is_empty() {
                input_row = input_row.child(
                    Label::new(SharedString::from(before_cursor))
                        .size(LabelSize::Small),
                );
            }
            if !cursor_char.is_empty() {
                input_row = input_row.child(
                    div()
                        .bg(colors.text)
                        .child(
                            Label::new(SharedString::from(cursor_char))
                                .size(LabelSize::Small)
                                .color(Color::Custom(gpui::Hsla { h: 0.0, s: 0.0, l: 0.0, a: 1.0 })),
                        ),
                );
            } else {
                // Cursor at end
                input_row = input_row.child(
                    div()
                        .w(px(2.))
                        .h(px(16.))
                        .bg(colors.text),
                );
            }
            if !after_cursor.is_empty() {
                input_row = input_row.child(
                    Label::new(SharedString::from(after_cursor))
                        .size(LabelSize::Small),
                );
            }
        }

        // Build the modal content
        let mut modal = div()
            .id("branch-dialog-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(420.))
            .elevation_3(cx)
            .p_4()
            .gap_3()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        // Title
        modal = modal.child(
            Label::new("Create Branch")
                .size(LabelSize::Large)
                .weight(gpui::FontWeight::BOLD)
                .color(Color::Default),
        );

        // Branch name input
        modal = modal.child(
            div()
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Branch name")
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

        // Base ref display
        let base_display: SharedString = format!("Based on: {}", self.base_ref).into();
        modal = modal.child(
            Label::new(base_display)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
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
        modal = modal.child(
            div()
                .h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("cancel-branch", "Cancel")
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.dismiss(cx);
                        })),
                )
                .child(
                    Button::new("create-branch", "Create")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.try_create(cx);
                        })),
                ),
        );

        // Backdrop + modal
        div()
            .id("branch-dialog-backdrop")
            .absolute()
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
