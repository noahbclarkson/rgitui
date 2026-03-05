use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, CheckState, Checkbox, Label, LabelSize};

/// Events from the commit panel.
#[derive(Debug, Clone)]
pub enum CommitPanelEvent {
    CommitRequested { message: String, amend: bool },
    GenerateAiMessage,
}

/// The commit message panel at the bottom.
pub struct CommitPanel {
    commit_message: String,
    amend: bool,
    staged_count: usize,
    is_ai_generating: bool,
    focus_handle: FocusHandle,
    cursor_pos: usize,
}

impl EventEmitter<CommitPanelEvent> for CommitPanel {}

impl CommitPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            commit_message: String::new(),
            amend: false,
            staged_count: 0,
            is_ai_generating: false,
            focus_handle: cx.focus_handle(),
            cursor_pos: 0,
        }
    }

    pub fn set_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.cursor_pos = message.len();
        self.commit_message = message;
        cx.notify();
    }

    pub fn message(&self) -> &str {
        &self.commit_message
    }

    pub fn set_staged_count(&mut self, count: usize, cx: &mut Context<Self>) {
        self.staged_count = count;
        cx.notify();
    }

    pub fn set_ai_generating(&mut self, generating: bool, cx: &mut Context<Self>) {
        self.is_ai_generating = generating;
        cx.notify();
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();

        match key {
            "backspace" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.commit_message.remove(self.cursor_pos);
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.commit_message.len() {
                    self.commit_message.remove(self.cursor_pos);
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
                if self.cursor_pos < self.commit_message.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.commit_message.len();
                cx.notify();
            }
            "enter" => {
                if keystroke.modifiers.control || keystroke.modifiers.platform {
                    // Ctrl+Enter = commit
                    if !self.commit_message.is_empty() && self.staged_count > 0 {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: self.commit_message.clone(),
                            amend: self.amend,
                        });
                    }
                } else {
                    self.commit_message.insert(self.cursor_pos, '\n');
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            _ => {
                // Insert printable characters
                if let Some(key_char) = &keystroke.key_char {
                    for ch in key_char.chars() {
                        self.commit_message.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                    }
                    cx.notify();
                } else if key.len() == 1 && !keystroke.modifiers.control && !keystroke.modifiers.platform {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() || ch == ' ' {
                        self.commit_message.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for CommitPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let can_commit = !self.commit_message.is_empty() && self.staged_count > 0;

        let staged_label: SharedString = if self.staged_count > 0 {
            format!(
                "{} staged file{}",
                self.staged_count,
                if self.staged_count == 1 { "" } else { "s" }
            )
            .into()
        } else {
            "No staged changes".into()
        };

        // Build commit message display with cursor
        let is_focused = self.focus_handle.is_focused(window);
        let msg_display: SharedString = if self.commit_message.is_empty() {
            if is_focused {
                "|".into()
            } else {
                "Click to enter commit message...".into()
            }
        } else if is_focused {
            // Insert cursor character at position
            let mut display = self.commit_message.clone();
            let pos = self.cursor_pos.min(display.len());
            display.insert(pos, '|');
            display.into()
        } else {
            self.commit_message.clone().into()
        };

        let msg_color = if self.commit_message.is_empty() && !is_focused {
            Color::Placeholder
        } else {
            Color::Default
        };

        let amend = self.amend;
        let message = self.commit_message.clone();

        div()
            .v_flex()
            .w_full()
            .bg(colors.panel_background)
            .border_t_1()
            .border_color(colors.border_variant)
            .p_2()
            .gap_1()
            // Header row
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Label::new("Commit")
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(staged_label)
                            .size(LabelSize::XSmall)
                            .color(if self.staged_count > 0 {
                                Color::Added
                            } else {
                                Color::Muted
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(
                            "ai-btn",
                            if self.is_ai_generating {
                                "Generating..."
                            } else {
                                "AI Message"
                            },
                        )
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Outlined)
                        .disabled(self.is_ai_generating || self.staged_count == 0)
                        .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                            cx.emit(CommitPanelEvent::GenerateAiMessage);
                        })),
                    ),
            )
            // Commit message text area - interactive
            .child(
                div()
                    .id("commit-message-area")
                    .track_focus(&self.focus_handle)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .w_full()
                    .min_h(px(60.))
                    .max_h(px(120.))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_focused {
                        colors.border_focused
                    } else {
                        colors.border
                    })
                    .bg(colors.editor_background)
                    .overflow_y_scroll()
                    .cursor_text()
                    .child(
                        Label::new(msg_display)
                            .size(LabelSize::Small)
                            .color(msg_color),
                    ),
            )
            // Action buttons
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .id("amend-toggle")
                            .h_flex()
                            .gap_1()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.amend = !this.amend;
                                cx.notify();
                            }))
                            .child(
                                Checkbox::new(
                                    "amend-checkbox",
                                    if self.amend {
                                        CheckState::Checked
                                    } else {
                                        CheckState::Unchecked
                                    },
                                ),
                            )
                            .child(
                                Label::new("Amend")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("commit-btn", "Commit")
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Default)
                            .disabled(!can_commit)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.emit(CommitPanelEvent::CommitRequested {
                                    message: message.clone(),
                                    amend,
                                });
                                this.commit_message.clear();
                                this.cursor_pos = 0;
                                cx.notify();
                            })),
                    ),
            )
    }
}
