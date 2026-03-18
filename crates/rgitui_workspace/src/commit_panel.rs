use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Render, SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, IconName, Label, LabelSize, TextInput,
    TextInputEvent,
};

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

        cx.subscribe(&summary_editor, |this: &mut Self, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Submit = event {
                let can_commit = !this.summary_editor.read(cx).is_empty() && this.staged_count > 0;
                if can_commit {
                    cx.emit(CommitPanelEvent::CommitRequested {
                        message: this.message(cx),
                        amend: this.amend,
                    });
                    this.summary_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                    this.description_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                    this.amend = false;
                    cx.notify();
                }
            }
        }).detach();

        cx.subscribe(&description_editor, |_this: &mut Self, _, _event: &TextInputEvent, _cx| {
        }).detach();

        Self {
            summary_editor,
            description_editor,
            amend: false,
            staged_count: 0,
            is_ai_generating: false,
            focus_handle: cx.focus_handle(),
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
        self.summary_editor.update(cx, |e: &mut TextInput, cx| e.set_text(summary, cx));
        self.description_editor.update(cx, |e: &mut TextInput, cx| e.set_text(description, cx));
        cx.notify();
    }

    pub fn message(&self, cx: &Context<Self>) -> String {
        let summary = self.summary_editor.read(cx).text().to_string();
        let description = self.description_editor.read(cx).text().to_string();
        if description.is_empty() {
            summary
        } else {
            format!("{}\n\n{}", summary, description)
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

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.summary_editor.update(cx, |e: &mut TextInput, cx| e.focus(window, cx));
    }

    pub fn is_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }
}

impl Render for CommitPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let can_commit = !self.summary_editor.read(cx).is_empty() && self.staged_count > 0;
        let summary_len = self.summary_editor.read(cx).text().chars().count();

        let staged_label: SharedString = if self.staged_count > 0 {
            format!("{} file{} staged", self.staged_count, if self.staged_count == 1 { "" } else { "s" }).into()
        } else {
            "No files staged".into()
        };

        let char_count_label: SharedString = format!("{}/72", summary_len).into();
        let char_count_color = if summary_len > 72 { Color::Warning } else { Color::Muted };

        let amend = self.amend;
        let message = self.message(cx);

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
                    .gap_2()
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
                            .bg(if self.staged_count > 0 { colors.ghost_element_selected } else { colors.element_disabled })
                            .items_center()
                            .child(Label::new(staged_label).size(LabelSize::XSmall).color(
                                if self.staged_count > 0 { Color::Added } else { Color::Muted },
                            )),
                    )
                    .child(div().flex_1())
                    .when(self.is_ai_generating, |el| {
                        el.child(
                            div().h_flex().h(px(20.)).px(px(8.)).rounded(px(3.))
                                .bg(colors.ghost_element_selected)
                                .items_center().gap(px(4.))
                                .child(rgitui_ui::Icon::new(IconName::Sparkle).size(rgitui_ui::IconSize::XSmall).color(Color::Accent))
                                .child(Label::new("Generating...").size(LabelSize::XSmall).color(Color::Accent)),
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
                        el.child(
                            btn.on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.emit(CommitPanelEvent::GenerateAiMessage);
                            })),
                        )
                    }),
            )
            .child(
                div()
                    .id("commit-content-area")
                    .track_focus(&self.focus_handle)
                    .key_context("CommitPanel")
                    .v_flex()
                    .flex_1()
                    .px(px(10.))
                    .py(px(10.))
                    .gap(px(8.))
                    .min_h(px(150.))
                    .child(
                        div().v_flex().gap(px(2.)).flex_shrink_0()
                            .child(
                                div().h_flex().items_center()
                                    .child(Label::new("Summary").size(LabelSize::XSmall).color(Color::Muted).weight(gpui::FontWeight::MEDIUM))
                                    .child(div().flex_1())
                                    .when(!self.summary_editor.read(cx).is_empty(), |el| {
                                        el.child(Label::new(char_count_label).size(LabelSize::XSmall).color(char_count_color))
                                    }),
                            )
                            .child(self.summary_editor.clone()),
                    )
                    .child(
                        div().v_flex().gap(px(2.)).flex_1().min_h(px(50.))
                            .child(Label::new("Description").size(LabelSize::XSmall).color(Color::Muted).weight(gpui::FontWeight::MEDIUM))
                            .child(self.description_editor.clone()),
                    )
                    .child(
                        div().h_flex().w_full().gap_2().items_center().flex_shrink_0()
                            .child(
                                div().id("amend-toggle").h_flex().gap_1().items_center().cursor_pointer()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.amend = !this.amend;
                                        cx.notify();
                                    }))
                                    .child(Checkbox::new("amend-checkbox", if self.amend { CheckState::Checked } else { CheckState::Unchecked }))
                                    .child(Label::new("Amend").size(LabelSize::XSmall).color(Color::Muted)),
                            )
                            .when(!self.summary_editor.read(cx).is_empty() || !self.description_editor.read(cx).is_empty(), |el| {
                                el.child(
                                    Button::new("clear-btn", "Clear")
                                        .icon(IconName::X)
                                        .size(ButtonSize::Compact)
                                        .style(ButtonStyle::Subtle)
                                        .color(Color::Muted)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            this.summary_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                            this.description_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(div().flex_1())
                            .when(self.staged_count == 0, |el| {
                                el.child(Label::new("No staged changes").size(LabelSize::XSmall).color(Color::Warning))
                            })
                            .when(can_commit, |el| {
                                el.child(Label::new("Ctrl+Enter").size(LabelSize::XSmall).color(Color::Muted))
                            })
                            .child(
                                Button::new("commit-btn", if self.staged_count == 0 { "No Staged Changes" } else if self.summary_editor.read(cx).is_empty() { "No Message" } else if self.amend { "Amend Commit" } else { "Commit" })
                                    .icon(IconName::GitCommit)
                                    .style(ButtonStyle::Filled)
                                    .size(ButtonSize::Default)
                                    .disabled(!can_commit)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(CommitPanelEvent::CommitRequested { message: message.clone(), amend });
                                        this.summary_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                        this.description_editor.update(cx, |e: &mut TextInput, cx| e.clear(cx));
                                        this.amend = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }
}
