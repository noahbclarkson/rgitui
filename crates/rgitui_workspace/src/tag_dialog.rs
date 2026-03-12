use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize};

/// Events emitted by the tag creation dialog.
#[derive(Debug, Clone)]
pub enum TagDialogEvent {
    CreateTag {
        name: String,
        target_oid: git2::Oid,
    },
    Dismissed,
}

/// A modal dialog for creating a new Git tag at a specific commit.
pub struct TagDialog {
    tag_name: String,
    target_oid: Option<git2::Oid>,
    target_sha_short: String,
    error_message: Option<String>,
    visible: bool,
    cursor_pos: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<TagDialogEvent> for TagDialog {}

impl TagDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tag_name: String::new(),
            target_oid: None,
            target_sha_short: String::new(),
            error_message: None,
            visible: false,
            cursor_pos: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog for creating a tag at the given commit.
    pub fn show_visible(
        &mut self,
        target_oid: git2::Oid,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.tag_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        self.target_sha_short = target_oid.to_string()[..7].to_string();
        self.target_oid = Some(target_oid);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.tag_name.clear();
        self.cursor_pos = 0;
        self.error_message = None;
        self.target_oid = None;
        cx.emit(TagDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Validate the tag name.
    fn validate_tag_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Tag name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Tag name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Tag name cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Tag name cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") {
            return Some("Tag name cannot contain '..'".to_string());
        }
        if name.contains("~")
            || name.contains("^")
            || name.contains(":")
            || name.contains("\\")
        {
            return Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string());
        }
        if name.contains("?") || name.contains("*") || name.contains("[") {
            return Some("Tag name cannot contain glob characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Tag name cannot contain control characters".to_string());
        }
        if name.contains("@{") {
            return Some("Tag name cannot contain '@{'".to_string());
        }
        if name.contains("//") {
            return Some("Tag name cannot contain consecutive slashes".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Tag name cannot end with '.lock'".to_string());
        }
        None
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        if let Some(err) = Self::validate_tag_name(&self.tag_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        if let Some(oid) = self.target_oid {
            let name = self.tag_name.clone();
            self.visible = false;
            self.tag_name.clear();
            self.cursor_pos = 0;
            self.error_message = None;
            self.target_oid = None;
            cx.emit(TagDialogEvent::CreateTag {
                name,
                target_oid: oid,
            });
            cx.notify();
        }
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
            "escape" => {
                self.dismiss(cx);
            }
            "enter" => {
                self.try_create(cx);
            }
            "backspace" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.tag_name.remove(self.cursor_pos);
                    self.error_message = if self.tag_name.is_empty() {
                        None
                    } else {
                        Self::validate_tag_name(&self.tag_name)
                    };
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.tag_name.len() {
                    self.tag_name.remove(self.cursor_pos);
                    self.error_message = if self.tag_name.is_empty() {
                        None
                    } else {
                        Self::validate_tag_name(&self.tag_name)
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
                if self.cursor_pos < self.tag_name.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.tag_name.len();
                cx.notify();
            }
            _ => {
                if let Some(key_char) = &keystroke.key_char {
                    self.tag_name.insert_str(self.cursor_pos, key_char);
                    self.cursor_pos += key_char.len();
                    self.error_message = Self::validate_tag_name(&self.tag_name);
                    cx.notify();
                } else if key.len() == 1
                    && !keystroke.modifiers.control
                    && !keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() {
                        self.tag_name.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                        self.error_message = Self::validate_tag_name(&self.tag_name);
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for TagDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("tag-dialog").into_any_element();
        }

        // Build the input display with cursor
        let (before_cursor, cursor_char, after_cursor) = if self.tag_name.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            let before = self.tag_name[..self.cursor_pos].to_string();
            let cursor = if self.cursor_pos < self.tag_name.len() {
                self.tag_name[self.cursor_pos..self.cursor_pos + 1].to_string()
            } else {
                String::new()
            };
            let after = if self.cursor_pos + 1 < self.tag_name.len() {
                self.tag_name[self.cursor_pos + 1..].to_string()
            } else {
                String::new()
            };
            (before, cursor, after)
        };

        let is_empty = self.tag_name.is_empty();
        let has_error = self.error_message.is_some();

        let input_border_color = if has_error {
            colors.vc_deleted
        } else {
            colors.border_focused
        };

        // Text input field
        let mut input_row = div().h_flex().items_center().w_full();

        if is_empty {
            input_row = input_row
                .child(div().w(px(2.)).h(px(16.)).bg(colors.text))
                .child(
                    Label::new("v1.0.0")
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

        // Build the modal content
        let mut modal = div()
            .id("tag-dialog-modal")
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

        // Title with icon
        modal = modal.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(IconName::Tag)
                        .size(IconSize::Medium)
                        .color(Color::Accent),
                )
                .child(
                    Label::new("Create Tag")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Default),
                ),
        );

        // Tag name input
        modal = modal.child(
            div()
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Tag name")
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

        // Target commit badge
        let target_sha: SharedString = self.target_sha_short.clone().into();
        modal = modal.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    Label::new("At commit")
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
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(target_sha)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::MEDIUM)
                                .color(Color::Accent),
                        ),
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
        let can_create = !self.tag_name.is_empty() && self.error_message.is_none();
        modal = modal.child(
            div()
                .h_flex()
                .justify_between()
                .items_center()
                .child(
                    Label::new("Enter to create · Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("cancel-tag", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("create-tag", "Create Tag")
                                .icon(IconName::Tag)
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Filled)
                                .color(Color::Accent)
                                .disabled(!can_create)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.try_create(cx);
                                })),
                        ),
                ),
        );

        // Backdrop + modal
        div()
            .id("tag-dialog-backdrop")
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
