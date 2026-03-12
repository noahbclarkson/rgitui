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

/// Which text field is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitField {
    None,
    Summary,
    Description,
}

/// The commit message panel at the bottom.
pub struct CommitPanel {
    summary: String,
    description: String,
    amend: bool,
    staged_count: usize,
    is_ai_generating: bool,
    focus_handle: FocusHandle,
    focused_field: CommitField,
    cursor_pos: usize,
}

impl EventEmitter<CommitPanelEvent> for CommitPanel {}

impl CommitPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            summary: String::new(),
            description: String::new(),
            amend: false,
            staged_count: 0,
            is_ai_generating: false,
            focus_handle: cx.focus_handle(),
            focused_field: CommitField::None,
            cursor_pos: 0,
        }
    }

    pub fn set_message(&mut self, message: String, cx: &mut Context<Self>) {
        // Split message into summary (first line) and description (rest)
        let (summary, description) = match message.find('\n') {
            Some(idx) => {
                let s = message[..idx].to_string();
                let d = message[idx + 1..].trim_start_matches('\n').to_string();
                (s, d)
            }
            None => (message, String::new()),
        };
        self.summary = summary;
        self.description = description;
        self.cursor_pos = self.summary.len();
        self.focused_field = CommitField::None;
        cx.notify();
    }

    pub fn message(&self) -> String {
        if self.description.is_empty() {
            self.summary.clone()
        } else {
            format!("{}\n\n{}", self.summary, self.description)
        }
    }

    pub fn set_staged_count(&mut self, count: usize, cx: &mut Context<Self>) {
        self.staged_count = count;
        cx.notify();
    }

    pub fn set_ai_generating(&mut self, generating: bool, cx: &mut Context<Self>) {
        self.is_ai_generating = generating;
        cx.notify();
    }

    fn focused_text(&self) -> &str {
        match self.focused_field {
            CommitField::None => "",
            CommitField::Summary => &self.summary,
            CommitField::Description => &self.description,
        }
    }

    fn focused_text_mut(&mut self) -> &mut String {
        match self.focused_field {
            CommitField::None => &mut self.summary, // fallback, shouldn't be used
            CommitField::Summary => &mut self.summary,
            CommitField::Description => &mut self.description,
        }
    }

    fn focus_field(&mut self, field: CommitField, cx: &mut Context<Self>) {
        self.focused_field = field;
        self.cursor_pos = match field {
            CommitField::None => 0,
            CommitField::Summary => self.summary.len(),
            CommitField::Description => self.description.len(),
        };
        cx.notify();
    }

    fn blur(&mut self, cx: &mut Context<Self>) {
        if self.focused_field != CommitField::None {
            self.focused_field = CommitField::None;
            cx.notify();
        }
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
        if self.focused_field == CommitField::None {
            return;
        }

        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        // Extract cursor_pos to avoid borrow issues with focused_text_mut()
        let pos = self.cursor_pos;

        // Handle Ctrl+ shortcuts first
        if ctrl {
            match key {
                "a" => {
                    self.cursor_pos = self.focused_text().len();
                    cx.notify();
                    return;
                }
                "v" => {
                    if let Some(clipboard) = cx.read_from_clipboard() {
                        if let Some(text) = clipboard.text() {
                            let text_len = text.len();
                            let t = text.clone();
                            self.focused_text_mut().insert_str(pos, &t);
                            self.cursor_pos = pos + text_len;
                            cx.notify();
                        }
                    }
                    return;
                }
                "enter" => {
                    if !self.summary.is_empty() && self.staged_count > 0 {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: self.message(),
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
                if pos > 0 {
                    let prev = Self::prev_char_boundary(self.focused_text(), pos);
                    self.focused_text_mut().drain(prev..pos);
                    self.cursor_pos = prev;
                    cx.notify();
                }
            }
            "delete" => {
                let len = self.focused_text().len();
                if pos < len {
                    let next = Self::next_char_boundary(self.focused_text(), pos);
                    self.focused_text_mut().drain(pos..next);
                    cx.notify();
                }
            }
            "left" => {
                if pos > 0 {
                    self.cursor_pos = Self::prev_char_boundary(self.focused_text(), pos);
                    cx.notify();
                }
            }
            "right" => {
                let len = self.focused_text().len();
                if pos < len {
                    self.cursor_pos = Self::next_char_boundary(self.focused_text(), pos);
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.focused_text().len();
                cx.notify();
            }
            "tab" => {
                // Tab from summary → description
                if self.focused_field == CommitField::Summary {
                    self.focus_field(CommitField::Description, cx);
                    return;
                }
            }
            "escape" => {
                self.blur(cx);
                return;
            }
            "enter" => {
                if self.focused_field == CommitField::Summary {
                    // Enter in summary → move to description
                    self.focus_field(CommitField::Description, cx);
                    return;
                }
                // In description, insert newline
                self.focused_text_mut().insert(pos, '\n');
                self.cursor_pos = pos + 1;
                cx.notify();
            }
            _ => {
                if !ctrl {
                    if let Some(key_char) = &keystroke.key_char {
                        let key_len = key_char.len();
                        let kc = key_char.clone();
                        self.focused_text_mut().insert_str(pos, &kc);
                        self.cursor_pos = pos + key_len;
                        cx.notify();
                    } else if key.len() == 1 {
                        let ch = key.chars().next().unwrap();
                        if ch.is_ascii_graphic() || ch == ' ' {
                            self.focused_text_mut().insert(pos, ch);
                            self.cursor_pos = pos + ch.len_utf8();
                            cx.notify();
                        }
                    }
                }
            }
        }
    }

    /// Build display text with cursor indicator for a field.
    fn field_display(
        &self,
        field: CommitField,
        value: &str,
        placeholder: &'static str,
    ) -> (SharedString, Color) {
        let is_focused = self.focused_field == field;
        if value.is_empty() {
            if is_focused {
                ("|".into(), Color::Default)
            } else {
                (placeholder.into(), Color::Placeholder)
            }
        } else if is_focused {
            let mut display = value.to_string();
            let pos = self.cursor_pos.min(display.len());
            display.insert(pos, '|');
            (display.into(), Color::Default)
        } else {
            (value.to_string().into(), Color::Default)
        }
    }
}

impl Render for CommitPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let can_commit = !self.summary.is_empty() && self.staged_count > 0;

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

        // Character count for summary line
        let summary_len = self.summary.chars().count();
        let char_count_label: SharedString = format!("{}/72", summary_len).into();
        let char_count_color = if summary_len > 72 {
            Color::Warning
        } else {
            Color::Muted
        };
        let summary_over = summary_len > 72;

        let summary_focused = self.focused_field == CommitField::Summary;
        let desc_focused = self.focused_field == CommitField::Description;

        let (summary_display, summary_color) =
            self.field_display(CommitField::Summary, &self.summary, "Commit summary...");
        let (desc_display, desc_color) = self.field_display(
            CommitField::Description,
            &self.description,
            "Optional extended description...",
        );

        let amend = self.amend;
        let message = self.message();

        div()
            .v_flex()
            .size_full()
            .bg(colors.panel_background)
            // Header bar
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(26.))
                    .px(px(10.))
                    .gap_2()
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        rgitui_ui::Icon::new(IconName::GitCommit)
                            .size(rgitui_ui::IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Commit")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    // Staged count badge
                    .child(
                        div()
                            .h_flex()
                            .h(px(18.))
                            .px(px(6.))
                            .rounded(px(3.))
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
                                .h(px(20.))
                                .px(px(8.))
                                .rounded(px(3.))
                                .bg(colors.ghost_element_selected)
                                .items_center()
                                .gap(px(4.))
                                .child(
                                    rgitui_ui::Icon::new(IconName::Sparkle)
                                        .size(rgitui_ui::IconSize::XSmall)
                                        .color(Color::Accent),
                                )
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
            // Content area — click anywhere to blur fields
            .child(
                div()
                    .id("commit-content-area")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        // Click on the content area background blurs any focused field
                        this.blur(cx);
                    }))
                    .v_flex()
                    .flex_1()
                    .p(px(8.))
                    .gap(px(6.))
                    // Summary field
                    .child(
                        div()
                            .v_flex()
                            .gap(px(2.))
                            .child(
                                div()
                                    .h_flex()
                                    .items_center()
                                    .child(
                                        Label::new("Summary")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .weight(gpui::FontWeight::MEDIUM),
                                    )
                                    .child(div().flex_1())
                                    .when(!self.summary.is_empty(), |el| {
                                        el.child(
                                            Label::new(char_count_label)
                                                .size(LabelSize::XSmall)
                                                .color(char_count_color),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .id("commit-summary-field")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.focus_field(CommitField::Summary, cx);
                                    }))
                                    .h_flex()
                                    .items_center()
                                    .w_full()
                                    .h(px(30.))
                                    .px(px(10.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(if summary_over {
                                        colors.vc_deleted
                                    } else if summary_focused {
                                        colors.border_focused
                                    } else {
                                        colors.border
                                    })
                                    .bg(colors.editor_background)
                                    .when(!summary_focused && !summary_over, |el| {
                                        let hover_border = colors.border_focused;
                                        el.hover(move |s| s.border_color(hover_border))
                                    })
                                    .cursor_text()
                                    .child(
                                        Label::new(summary_display)
                                            .size(LabelSize::Small)
                                            .color(summary_color)
                                            .truncate(),
                                    ),
                            ),
                    )
                    // Description field
                    .child(
                        div()
                            .v_flex()
                            .gap(px(2.))
                            .child(
                                Label::new("Description")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .weight(gpui::FontWeight::MEDIUM),
                            )
                            .child(
                                div()
                                    .id("commit-description-field")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.focus_field(CommitField::Description, cx);
                                    }))
                                    .w_full()
                                    .min_h(px(48.))
                                    .flex_1()
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(if desc_focused {
                                        colors.border_focused
                                    } else {
                                        colors.border
                                    })
                                    .bg(colors.editor_background)
                                    .when(!desc_focused, |el| {
                                        let hover_border = colors.border_focused;
                                        el.hover(move |s| s.border_color(hover_border))
                                    })
                                    .overflow_y_scroll()
                                    .cursor_text()
                                    .child(
                                        Label::new(desc_display)
                                            .size(LabelSize::XSmall)
                                            .color(desc_color),
                                    ),
                            ),
                    )
                    // Action row
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
                                        cx.stop_propagation();
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
                            .when(
                                !self.summary.is_empty() || !self.description.is_empty(),
                                |el| {
                                    el.child(
                                        Button::new("clear-btn", "Clear")
                                            .icon(IconName::X)
                                            .size(ButtonSize::Compact)
                                            .style(ButtonStyle::Subtle)
                                            .color(Color::Muted)
                                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                this.summary.clear();
                                                this.description.clear();
                                                this.cursor_pos = 0;
                                                this.focused_field = CommitField::None;
                                                cx.notify();
                                            })),
                                    )
                                },
                            )
                            .child(div().flex_1())
                            // Status hint
                            .when(self.staged_count == 0, |el| {
                                el.child(
                                    Label::new("No staged changes")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                )
                            })
                            .when(can_commit, |el| {
                                el.child(
                                    Label::new("Ctrl+Enter")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            })
                            .child(
                                Button::new(
                                    "commit-btn",
                                    if self.staged_count == 0 {
                                        "No Staged Changes"
                                    } else if self.summary.is_empty() {
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
                                        cx.stop_propagation();
                                        cx.emit(CommitPanelEvent::CommitRequested {
                                            message: message.clone(),
                                            amend,
                                        });
                                        this.summary.clear();
                                        this.description.clear();
                                        this.cursor_pos = 0;
                                        this.amend = false;
                                        this.focused_field = CommitField::None;
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
    }
}
