use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, IconName, Label, LabelSize, TextInput,
    TextInputEvent,
};

/// A co-author entry for the `Co-Authored-By:` trailer.
#[derive(Debug, Clone)]
struct CoAuthor {
    name: String,
    email: String,
}

impl CoAuthor {
    fn trailer(&self) -> String {
        format!("Co-Authored-By: {} <{}>", self.name, self.email)
    }
}

#[derive(Debug, Clone)]
pub enum CommitPanelEvent {
    CommitRequested { message: String, amend: bool },
    GenerateAiMessage,
}

pub struct CommitPanel {
    summary_editor: Entity<TextInput>,
    description_editor: Entity<TextInput>,
    amend: bool,
    staged_count: usize,
    is_ai_generating: bool,
    focus_handle: FocusHandle,
    co_authors: Vec<CoAuthor>,
    adding_co_author: bool,
    new_author_name: Entity<TextInput>,
    new_author_email: Entity<TextInput>,
}

impl EventEmitter<CommitPanelEvent> for CommitPanel {}

impl CommitPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let summary_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Commit summary...");
            ti
        });
        let description_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx).multiline();
            ti.set_placeholder("Optional extended description...");
            ti.set_font_size(px(12.0));
            ti
        });
        let new_author_name = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Co-author name...");
            ti
        });
        let new_author_email = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Co-author email...");
            ti
        });

        cx.subscribe(
            &summary_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Submit = event {
                    let can_commit =
                        !this.summary_editor.read(cx).is_empty() && this.staged_count > 0;
                    if can_commit {
                        cx.emit(CommitPanelEvent::CommitRequested {
                            message: this.message(cx),
                            amend: this.amend,
                        });
                        this.summary_editor
                            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
                        this.description_editor
                            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
                        this.amend = false;
                        this.co_authors.clear();
                        this.adding_co_author = false;
                        cx.notify();
                    }
                }
            },
        )
        .detach();

        cx.subscribe(
            &description_editor,
            |_this: &mut Self, _, _event: &TextInputEvent, _cx| {},
        )
        .detach();

        Self {
            summary_editor,
            description_editor,
            amend: false,
            staged_count: 0,
            is_ai_generating: false,
            focus_handle: cx.focus_handle(),
            co_authors: Vec::new(),
            adding_co_author: false,
            new_author_name,
            new_author_email,
        }
    }

    pub fn set_message(&mut self, message: String, cx: &mut Context<Self>) {
        let (summary, description) = match message.find('\n') {
            Some(idx) => (
                message[..idx].trim().to_string(),
                message[idx + 1..].trim().to_string(),
            ),
            None => (message, String::new()),
        };
        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(summary, cx));
        self.description_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(description, cx));
        self.co_authors.clear();
        self.adding_co_author = false;
        cx.notify();
    }

    pub fn message(&self, cx: &Context<Self>) -> String {
        let summary = self.summary_editor.read(cx).text().to_string();
        let description = self.description_editor.read(cx).text().to_string();
        let mut msg = if description.is_empty() {
            summary
        } else {
            format!("{}\n\n{}", summary, description)
        };
        for co_author in &self.co_authors {
            msg.push_str("\n\n");
            msg.push_str(&co_author.trailer());
        }
        msg
    }

    pub fn set_staged_count(&mut self, count: usize, cx: &mut Context<Self>) {
        self.staged_count = count;
        cx.notify();
    }

    pub fn set_ai_generating(&mut self, generating: bool, cx: &mut Context<Self>) {
        self.is_ai_generating = generating;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.focus(window, cx));
    }

    pub fn is_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    fn commit_button_label(&self, summary_empty: bool) -> &'static str {
        if self.staged_count == 0 {
            "No Staged Changes"
        } else if summary_empty {
            "No Message"
        } else if self.amend {
            "Amend Commit"
        } else {
            "Commit"
        }
    }

    fn start_adding_co_author(&mut self, cx: &mut Context<Self>) {
        self.adding_co_author = true;
        self.new_author_name
            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
        self.new_author_email
            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
        cx.notify();
    }

    fn cancel_adding_co_author(&mut self, cx: &mut Context<Self>) {
        self.adding_co_author = false;
        cx.notify();
    }

    fn confirm_add_co_author(&mut self, cx: &mut Context<Self>) {
        let name = self.new_author_name.read(cx).text().trim().to_string();
        let email = self.new_author_email.read(cx).text().trim().to_string();
        if !name.is_empty() && !email.is_empty() {
            self.co_authors.push(CoAuthor { name, email });
            self.adding_co_author = false;
            cx.notify();
        }
    }

    fn remove_co_author(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.co_authors.len() {
            self.co_authors.remove(index);
            cx.notify();
        }
    }
}

impl Render for CommitPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let summary_empty = self.summary_editor.read(cx).is_empty();
        let can_commit = !summary_empty && self.staged_count > 0;
        let summary_len = self.summary_editor.read(cx).text().chars().count();
        let description_len = self.description_editor.read(cx).text().chars().count();
        let description_lines = if description_len == 0 {
            0
        } else {
            self.description_editor.read(cx).text().lines().count()
        };

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

        // Conventional commits: 50 chars is the recommended limit, 72 is the hard wrap limit.
        // Color: muted (≤50), warning (>50…72), error (>72).
        let char_count_label: SharedString = format!("{}/50", summary_len).into();
        let char_count_color = if summary_len > 72 {
            Color::Error
        } else if summary_len > 50 {
            Color::Warning
        } else {
            Color::Muted
        };

        let desc_count_label: SharedString = if description_len > 0 {
            format!(
                "{} char{}, {} line{}",
                description_len,
                if description_len == 1 { "" } else { "s" },
                description_lines,
                if description_lines == 1 { "" } else { "s" },
            )
            .into()
        } else {
            SharedString::default()
        };

        let amend = self.amend;
        let message = self.message(cx);
        let commit_label = self.commit_button_label(summary_empty);

        let ai_settings = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|s| {
                let settings = s.settings();
                (settings.ai.enabled, s.ai_api_key().is_some())
            })
            .unwrap_or((false, false));
        let (ai_enabled, has_api_key) = ai_settings;

        div()
            .v_flex()
            .size_full()
            .bg(colors.panel_background)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
                    .items_center()
                    .flex_shrink_0()
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
                    .when(!self.is_ai_generating && ai_enabled, |el| {
                        let no_staged = self.staged_count == 0;
                        let is_disabled = no_staged || !has_api_key;
                        let mut btn = Button::new("ai-btn", "AI Message")
                            .icon(IconName::Sparkle)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Outlined)
                            .color(Color::Accent)
                            .disabled(is_disabled);
                        if !has_api_key {
                            btn = btn.tooltip("Set an API key in Settings to use AI");
                        } else if no_staged {
                            btn = btn.tooltip("Stage changes first to generate an AI message");
                        }
                        el.child(btn.on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                            cx.emit(CommitPanelEvent::GenerateAiMessage);
                        })))
                    }),
            )
            .child(
                div()
                    .id("commit-content-area")
                    .track_focus(&self.focus_handle)
                    .key_context("CommitPanel")
                    .v_flex()
                    .flex_1()
                    .min_h(px(120.))
                    .overflow_hidden()
                    .child(
                        div()
                            .id("commit-scroll-area")
                            .v_flex()
                            .flex_1()
                            .px(px(12.))
                            .pt(px(10.))
                            .pb(px(6.))
                            .gap(px(8.))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .v_flex()
                                    .gap(px(4.))
                                    .flex_shrink_0()
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
                                            .when(!summary_empty, |el| {
                                                el.child(
                                                    Label::new(char_count_label)
                                                        .size(LabelSize::XSmall)
                                                        .color(char_count_color),
                                                )
                                            }),
                                    )
                                    .child(self.summary_editor.clone()),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap(px(4.))
                                    .flex_1()
                                    .min_h(px(50.))
                                    .child(
                                        div()
                                            .h_flex()
                                            .items_center()
                                            .child(
                                                Label::new("Description")
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted)
                                                    .weight(gpui::FontWeight::MEDIUM),
                                            )
                                            .child(div().flex_1())
                                            .when(description_len > 0, |el| {
                                                el.child(
                                                    Label::new(desc_count_label)
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Muted),
                                                )
                                            }),
                                    )
                                    .child(self.description_editor.clone()),
                            )
                            // Co-authors section
                            .child(
                                div()
                                    .v_flex()
                                    .gap(px(4.))
                                    .w_full()
                                    .flex_shrink_0()
                                    .pt(px(4.))
                                    .pl(px(12.))
                                    .child(
                                        div()
                                            .h_flex()
                                            .w_full()
                                            .items_center()
                                            .child(
                                                Label::new("Co-Authors")
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted)
                                                    .weight(gpui::FontWeight::MEDIUM),
                                            )
                                            .child(div().flex_1())
                                            .child(
                                                Button::new(
                                                    "add-co-author-btn",
                                                    if self.adding_co_author {
                                                        "Close"
                                                    } else {
                                                        "Add"
                                                    },
                                                )
                                                .icon(if self.adding_co_author {
                                                    IconName::X
                                                } else {
                                                    IconName::Plus
                                                })
                                                .size(ButtonSize::Compact)
                                                .style(ButtonStyle::Subtle)
                                                .color(if self.adding_co_author {
                                                    Color::Muted
                                                } else {
                                                    Color::Accent
                                                })
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _, cx| {
                                                        if this.adding_co_author {
                                                            this.cancel_adding_co_author(cx);
                                                        } else {
                                                            this.start_adding_co_author(cx);
                                                        }
                                                    },
                                                )),
                                            ),
                                    )
                                    .when(!self.co_authors.is_empty(), |el| {
                                        el.child(div().v_flex().gap(px(2.)).children(
                                            self.co_authors.iter().enumerate().map(|(i, ca)| {
                                                let idx = i;
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .px(px(4.))
                                                    .child(
                                                        rgitui_ui::Icon::new(IconName::User)
                                                            .size(rgitui_ui::IconSize::XSmall)
                                                            .color(Color::Muted),
                                                    )
                                                    .child(
                                                        Label::new(format!(
                                                            "{} <{}>",
                                                            ca.name, ca.email
                                                        ))
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Default),
                                                    )
                                                    .child(
                                                        Button::new(
                                                            ElementId::NamedInteger(
                                                                "remove-co-author".into(),
                                                                idx as u64,
                                                            ),
                                                            "",
                                                        )
                                                        .icon(IconName::X)
                                                        .size(ButtonSize::Compact)
                                                        .style(ButtonStyle::Subtle)
                                                        .color(Color::Muted)
                                                        .on_click(cx.listener(
                                                            move |this, _: &ClickEvent, _, cx| {
                                                                this.remove_co_author(idx, cx);
                                                            },
                                                        )),
                                                    )
                                            }),
                                        ))
                                    })
                                    .when(self.adding_co_author, |el| {
                                        el.child(
                                            div()
                                                .v_flex()
                                                .gap(px(4.))
                                                .p(px(8.))
                                                .bg(colors.ghost_element_hover)
                                                .rounded(px(4.))
                                                .child(
                                                    div()
                                                        .v_flex()
                                                        .gap(px(4.))
                                                        .w_full()
                                                        .child(self.new_author_name.clone())
                                                        .child(self.new_author_email.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .h_flex()
                                                        .gap_2()
                                                        .child(
                                                            Button::new(
                                                                "confirm-co-author-btn",
                                                                "Add Co-Author",
                                                            )
                                                            .size(ButtonSize::Compact)
                                                            .style(ButtonStyle::Filled)
                                                            .color(Color::Accent)
                                                            .on_click(cx.listener(
                                                                |this, _: &ClickEvent, _, cx| {
                                                                    this.confirm_add_co_author(cx);
                                                                },
                                                            )),
                                                        )
                                                        .child(
                                                            Button::new(
                                                                "cancel-co-author-btn",
                                                                "Cancel",
                                                            )
                                                            .size(ButtonSize::Compact)
                                                            .style(ButtonStyle::Subtle)
                                                            .color(Color::Muted)
                                                            .on_click(cx.listener(
                                                                |this, _: &ClickEvent, _, cx| {
                                                                    this.cancel_adding_co_author(
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        ),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    // Action row pinned at bottom (never scrolls away)
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .gap(px(8.))
                            .items_center()
                            .flex_shrink_0()
                            .px(px(12.))
                            .py(px(6.))
                            .border_t_1()
                            .border_color(colors.border_variant)
                            .child(
                                div()
                                    .id("amend-toggle")
                                    .h_flex()
                                    .gap(px(4.))
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
                                !summary_empty || !self.description_editor.read(cx).is_empty(),
                                |el| {
                                    el.child(
                                        Button::new("clear-btn", "Clear")
                                            .icon(IconName::X)
                                            .size(ButtonSize::Compact)
                                            .style(ButtonStyle::Subtle)
                                            .color(Color::Muted)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    cx.stop_propagation();
                                                    this.summary_editor
                                                        .update(cx, |e: &mut TextInput, cx| {
                                                            e.clear(cx)
                                                        });
                                                    this.description_editor
                                                        .update(cx, |e: &mut TextInput, cx| {
                                                            e.clear(cx)
                                                        });
                                                    this.co_authors.clear();
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                },
                            )
                            .child(div().flex_1())
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
                                Button::new("commit-btn", commit_label)
                                    .icon(IconName::GitCommit)
                                    .style(if can_commit {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Outlined
                                    })
                                    .size(ButtonSize::Default)
                                    .disabled(!can_commit)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(CommitPanelEvent::CommitRequested {
                                            message: message.clone(),
                                            amend,
                                        });
                                        this.summary_editor
                                            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                        this.description_editor
                                            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                        this.amend = false;
                                        this.co_authors.clear();
                                        this.adding_co_author = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_co_author_trailer() {
        let ca = CoAuthor {
            name: "Alice Smith".into(),
            email: "alice@example.com".into(),
        };
        assert_eq!(
            ca.trailer(),
            "Co-Authored-By: Alice Smith <alice@example.com>"
        );
    }

    #[test]
    fn test_co_author_trailer_special_chars() {
        let ca = CoAuthor {
            name: "Bob Jr.".into(),
            email: "bob+tag@example.com".into(),
        };
        assert_eq!(
            ca.trailer(),
            "Co-Authored-By: Bob Jr. <bob+tag@example.com>"
        );
    }
}
