//! Dialog for creating a stash with an optional message.
//!
//! Triggered by toolbar Stash button or Ctrl+Z. Provides an optional message
//! field; pressing Enter with an empty field creates `git stash push` (default "WIP" message).

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Render, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput,
    TextInputEvent, TintColor,
};

/// Events emitted by the stash save dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum StashSaveDialogEvent {
    /// Create a stash with the given message (None = default "WIP" message).
    CreateStash {
        message: Option<String>,
    },
    Dismissed,
}

/// A modal dialog for creating a stash with an optional message.
pub struct StashSaveDialog {
    editor: Entity<TextInput>,
    error_message: Option<String>,
    visible: bool,
    /// Set when the dialog is shown so the next render focuses the message
    /// field. Lets us focus from call sites that have no `Window` (command
    /// palette, toolbar) without leaving the user to click in first.
    pending_focus: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<StashSaveDialogEvent> for StashSaveDialog {}

impl StashSaveDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("WIP: ");
            ti
        });

        let focus_handle = cx.focus_handle();

        cx.subscribe(
            &editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_create(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = Self::validate_message(text);
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            editor,
            error_message: None,
            visible: false,
            pending_focus: false,
            focus_handle,
        }
    }

    /// Show the dialog and focus the message field on the next render.
    pub fn show_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.pending_focus = true;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(StashSaveDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        let message_text = self.editor.read(cx).text();
        if let Some(error) = Self::validate_message(message_text) {
            self.error_message = Some(error);
            cx.notify();
            return;
        }
        let message = if message_text.trim().is_empty() {
            None
        } else {
            Some(message_text.trim().to_string())
        };
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(StashSaveDialogEvent::CreateStash { message });
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Enter is handled solely via the editor's `Submit` event so it fires
        // exactly once; here we only need the modal-level Escape-to-dismiss.
        if event.keystroke.key.as_str() == "escape" {
            self.dismiss(cx);
        }
    }

    fn validate_message(msg: &str) -> Option<String> {
        if msg.chars().count() > 500 {
            return Some("Message too long (max 500 characters)".to_string());
        }
        None
    }
}

impl Render for StashSaveDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("stash-save-dialog").into_any_element();
        }

        if self.pending_focus {
            self.pending_focus = false;
            self.editor.update(cx, |e, cx| e.focus(window, cx));
        }

        let colors = cx.colors();

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let mut modal = div()
            .id("stash-save-dialog-backdrop")
            .occlude()
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
            .child(
                div()
                    .id("stash-save-dialog-modal")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .v_flex()
                    .w(px(480.))
                    .elevation_3(cx)
                    .p(px(24.))
                    .gap(px(20.))
                    .rounded(px(12.))
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    }),
            );

        // Header with icon and title
        modal = modal.child(
            div()
                .id("stash-save-dialog-header")
                .h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .rounded(px(10.))
                        .bg(icon_bg)
                        .child(
                            Icon::new(IconName::Stash)
                                .size(IconSize::Medium)
                                .color(Color::Custom(accent_color)),
                        ),
                )
                .child(
                    Label::new("Create Stash")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD),
                ),
        );

        // Message field
        modal = modal.child(
            div()
                .id("stash-save-dialog-content")
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Message (optional)")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(self.editor.clone())
                .child(
                    Label::new("Leave empty for default: \"WIP on <branch>\"")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                ),
        );

        // Error message
        if let Some(ref err) = self.error_message {
            modal = modal.child(
                Label::new(err.clone())
                    .size(LabelSize::Small)
                    .color(Color::Error),
            );
        }

        // Action buttons
        modal = modal.child(
            div()
                .id("stash-save-dialog-actions")
                .pt_2()
                .border_t_1()
                .border_color(colors.border_variant)
                .v_flex()
                .w_full()
                .gap_4()
                .child(
                    Label::new("Enter to confirm | Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .flex_nowrap()
                        .justify_end()
                        .w_full()
                        .child(
                            Button::new("stash-save-cancel", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("stash-save-confirm", "Create Stash")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Tinted(TintColor::Accent))
                                .disabled(self.error_message.is_some())
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.try_create(cx);
                                })),
                        ),
                ),
        );

        div()
            .id("stash-save-dialog")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(modal)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_save_dialog_event_debug() {
        let event = StashSaveDialogEvent::Dismissed;
        assert!(format!("{:?}", event).contains("Dismissed"));

        let event = StashSaveDialogEvent::CreateStash { message: None };
        assert!(format!("{:?}", event).contains("CreateStash"));

        let event = StashSaveDialogEvent::CreateStash {
            message: Some("my stash".to_string()),
        };
        assert!(format!("{:?}", event).contains("CreateStash"));
        assert!(format!("{:?}", event).contains("my stash"));
    }

    #[test]
    fn stash_save_dialog_event_clone_eq() {
        let event = StashSaveDialogEvent::CreateStash {
            message: Some("test".to_string()),
        };
        let clone = event.clone();
        assert_eq!(event, clone);

        let event = StashSaveDialogEvent::Dismissed;
        assert_eq!(event, event.clone());
    }

    #[test]
    fn validate_message_empty_is_valid() {
        assert!(StashSaveDialog::validate_message("").is_none());
        assert!(StashSaveDialog::validate_message("   ").is_none());
    }

    #[test]
    fn validate_message_too_long_returns_error() {
        let long_msg = "x".repeat(501);
        let result = StashSaveDialog::validate_message(&long_msg);
        assert!(result.is_some());
        assert!(result.unwrap().contains("500"));
    }

    #[test]
    fn validate_message_at_limit_is_valid() {
        let msg_at_limit = "x".repeat(500);
        assert!(StashSaveDialog::validate_message(&msg_at_limit).is_none());
    }

    #[test]
    fn validate_message_normal_content_is_valid() {
        assert!(StashSaveDialog::validate_message("WIP: feature").is_none());
        assert!(StashSaveDialog::validate_message("fix bug #123").is_none());
        assert!(StashSaveDialog::validate_message("中文 stash").is_none());
    }
}
