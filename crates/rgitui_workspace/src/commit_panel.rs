use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, IconName, Label, LabelSize,
};

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

    /// Get the byte index of the start of the character before `pos`.
    fn prev_char_boundary(s: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut p = pos - 1;
        while p > 0 && !s.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// Get the byte index of the start of the character after `pos`.
    fn next_char_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() {
            return s.len();
        }
        let mut p = pos + 1;
        while p < s.len() && !s.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        // Handle Ctrl+ shortcuts first
        if ctrl {
            match key {
                "a" => {
                    // Ctrl+A: select all (clear the message for simplicity)
                    // In a full editor this would select; here we just move cursor to end
                    self.cursor_pos = self.commit_message.len();
                    cx.notify();
                    return;
                }
                "v" => {
                    // Ctrl+V: paste from clipboard
                    if let Some(clipboard) = cx.read_from_clipboard() {
                        if let Some(text) = clipboard.text() {
                            self.commit_message.insert_str(self.cursor_pos, &text);
                            self.cursor_pos += text.len();
                            cx.notify();
                        }
                    }
                    return;
                }
                "enter" => {
                    // Ctrl+Enter = commit
                    if !self.commit_message.is_empty() && self.staged_count > 0 {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: self.commit_message.clone(),
                            amend: self.amend,
                        });
                    }
                    return;
                }
                _ => {}
            }
        }

        match key {
            "backspace" => {
                if self.cursor_pos > 0 {
                    let prev = Self::prev_char_boundary(&self.commit_message, self.cursor_pos);
                    self.commit_message.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.commit_message.len() {
                    let next = Self::next_char_boundary(&self.commit_message, self.cursor_pos);
                    self.commit_message.drain(self.cursor_pos..next);
                    cx.notify();
                }
            }
            "left" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos =
                        Self::prev_char_boundary(&self.commit_message, self.cursor_pos);
                    cx.notify();
                }
            }
            "right" => {
                if self.cursor_pos < self.commit_message.len() {
                    self.cursor_pos =
                        Self::next_char_boundary(&self.commit_message, self.cursor_pos);
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
                self.commit_message.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
                cx.notify();
            }
            _ => {
                // Insert printable characters (only when no ctrl/platform modifier)
                if !ctrl {
                    if let Some(key_char) = &keystroke.key_char {
                        self.commit_message.insert_str(self.cursor_pos, key_char);
                        self.cursor_pos += key_char.len();
                        cx.notify();
                    } else if key.len() == 1 {
                        let ch = key.chars().next().unwrap();
                        if ch.is_ascii_graphic() || ch == ' ' {
                            self.commit_message.insert(self.cursor_pos, ch);
                            self.cursor_pos += ch.len_utf8();
                            cx.notify();
                        }
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
                "{} file{} staged",
                self.staged_count,
                if self.staged_count == 1 { "" } else { "s" }
            )
            .into()
        } else {
            "No files staged".into()
        };

        // Character count for first line (conventional commits suggest 72 chars)
        let first_line = self.commit_message.lines().next().unwrap_or("");
        let first_line_len = first_line.len();
        let char_count_label: SharedString = format!("{}/72", first_line_len).into();
        let char_count_color = if first_line_len > 72 {
            Color::Warning
        } else {
            Color::Muted
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
            .size_full()
            .bg(colors.surface_background)
            .p(px(10.))
            .gap(px(6.))
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
                            .weight(gpui::FontWeight::BOLD),
                    )
                    // Staged count badge
                    .child(
                        div()
                            .h_flex()
                            .h(px(20.))
                            .px(px(8.))
                            .rounded(px(10.))
                            .bg(if self.staged_count > 0 {
                                colors.ghost_element_selected
                            } else {
                                colors.element_disabled
                            })
                            .items_center()
                            .child(Label::new(staged_label).size(LabelSize::XSmall).color(
                                if self.staged_count > 0 {
                                    Color::Added
                                } else {
                                    Color::Muted
                                },
                            )),
                    )
                    .child(div().flex_1())
                    .when(self.is_ai_generating, |el| {
                        el.child(
                            div()
                                .h_flex()
                                .h(px(24.))
                                .px(px(10.))
                                .rounded(px(12.))
                                .bg(colors.ghost_element_selected)
                                .items_center()
                                .gap_1()
                                .child(
                                    Label::new("Generating...")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Accent),
                                ),
                        )
                    })
                    .when(!self.is_ai_generating, |el| {
                        el.child(
                            Button::new("ai-btn", "AI Message")
                                .icon(IconName::Sparkle)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Outlined)
                                .color(Color::Accent)
                                .disabled(self.staged_count == 0)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(CommitPanelEvent::GenerateAiMessage);
                                })),
                        )
                    }),
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
                    .min_h(px(64.))
                    .max_h(px(120.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
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
            // Status row: char count + hints
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    // Left side: status hints
                    .child(
                        div()
                            .h_flex()
                            .flex_1()
                            .gap_1()
                            .items_center()
                            .when(self.staged_count == 0, |el| {
                                el.child(
                                    Label::new("No staged changes")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                )
                            })
                            .when(
                                self.staged_count > 0 && self.commit_message.is_empty(),
                                |el| {
                                    el.child(
                                        Label::new("Enter a commit message")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                },
                            ),
                    )
                    // Right side: char count
                    .when(!self.commit_message.is_empty(), |el| {
                        el.child(
                            Label::new(char_count_label)
                                .size(LabelSize::XSmall)
                                .color(char_count_color),
                        )
                    }),
            )
            // Action buttons
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .gap_2()
                    .pt(px(2.))
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
                            .child(Checkbox::new(
                                "amend-checkbox",
                                if self.amend {
                                    CheckState::Checked
                                } else {
                                    CheckState::Unchecked
                                },
                            ))
                            .child(
                                Label::new("Amend")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .when(!self.commit_message.is_empty(), |el| {
                        el.child(
                            Button::new("clear-btn", "Clear")
                                .icon(IconName::X)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Outlined)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.commit_message.clear();
                                    this.cursor_pos = 0;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new(
                            "commit-btn",
                            if self.staged_count == 0 {
                                "No Staged Changes"
                            } else if self.commit_message.is_empty() {
                                "No Message"
                            } else if self.amend {
                                "Amend Commit"
                            } else {
                                "Commit"
                            },
                        )
                        .icon(IconName::GitCommit)
                        .style(ButtonStyle::Filled)
                        .size(ButtonSize::Default)
                        .disabled(!can_commit)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.emit(CommitPanelEvent::CommitRequested {
                                    message: message.clone(),
                                    amend,
                                });
                                this.commit_message.clear();
                                this.cursor_pos = 0;
                                this.amend = false;
                                cx.notify();
                            },
                        )),
                    ),
            )
    }
}
