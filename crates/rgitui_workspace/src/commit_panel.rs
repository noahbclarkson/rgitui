use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Render,
    SharedString, Window,
};
use rgitui_ai::CommitStyle;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, IconButton, IconName, Label, LabelSize,
    TextInput, TextInputEvent,
};

const COMMIT_PANEL_HEADER_HEIGHT: f32 = 34.0;

pub(crate) fn commit_panel_height(expanded_height: f32, collapsed: bool) -> f32 {
    if collapsed {
        COMMIT_PANEL_HEADER_HEIGHT
    } else {
        expanded_height
    }
}

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
    CommitRequested {
        message: String,
        amend: bool,
    },
    GenerateAiMessage,
    /// Regenerate, optionally overriding the configured commit style for this
    /// one request. Belongs at the moment of dissatisfaction, not in a
    /// settings page in another window.
    RegenerateAiMessage {
        style: Option<CommitStyle>,
    },
    CancelAiMessage,
    /// Nothing is configured for AI yet, and the user asked to fix that.
    OpenAiSettings,
    CollapsedChanged,
}

/// Why the AI button cannot be used right now, or `None` when it can.
///
/// Every entry point (button, Ctrl+G, command palette) resolves through this
/// one predicate, so the keyboard and the mouse cannot disagree about whether
/// the feature is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBlocker {
    Disabled,
    NoApiKey,
    NothingStaged,
}

impl AiBlocker {
    /// The button's own label. The reason belongs in the control, not only in
    /// a hover tooltip on something that reads as inert.
    pub fn button_label(self) -> &'static str {
        match self {
            AiBlocker::Disabled => "AI is off",
            AiBlocker::NoApiKey => "Add an API key",
            AiBlocker::NothingStaged => "Stage files to use AI",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            AiBlocker::Disabled => "AI is turned off — enable it in Settings > AI",
            AiBlocker::NoApiKey => "No API key for the selected provider. Opens Settings > AI.",
            AiBlocker::NothingStaged => "Stage changes first to generate an AI message",
        }
    }

    /// Whether the button stays clickable.
    ///
    /// Never render a dead end when the fix is one click away: a missing key
    /// and a disabled feature both route to Settings. Only "nothing staged" is
    /// truly inert, and it is self-correcting — the user is about to stage.
    pub fn is_actionable(self) -> bool {
        matches!(self, AiBlocker::Disabled | AiBlocker::NoApiKey)
    }
}

/// Split a commit message into its summary line and body.
pub(crate) fn split_message(message: &str) -> (String, String) {
    match message.find('\n') {
        Some(index) => (
            message[..index].trim().to_string(),
            message[index + 1..].trim().to_string(),
        ),
        None => (message.trim().to_string(), String::new()),
    }
}

/// Resolve whether AI generation can run. Pure, so all three entry points can
/// share it and it is testable without a display.
pub fn ai_blocker(enabled: bool, has_api_key: bool, staged_count: usize) -> Option<AiBlocker> {
    if !enabled {
        return Some(AiBlocker::Disabled);
    }
    if !has_api_key {
        return Some(AiBlocker::NoApiKey);
    }
    if staged_count == 0 {
        return Some(AiBlocker::NothingStaged);
    }
    None
}

/// What the panel is doing about AI right now.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AiState {
    Idle,
    /// Running, with the latest tool-progress line if there is one.
    Generating {
        progress: Option<String>,
    },
    /// Just finished. Offers Regenerate and Undo for a short window.
    Completed,
    /// Failed, and the panel keeps saying so — a dismissed toast leaves no
    /// trace of why the field is still empty.
    Failed,
}

pub struct CommitPanel {
    summary_editor: Entity<TextInput>,
    description_editor: Entity<TextInput>,
    amend: bool,
    staged_count: usize,
    ai_state: AiState,
    /// The message the AI replaced, so the overwrite can be undone.
    /// `UndoStack` covers git operations, not the commit editor.
    ai_undo: Option<PreviousMessage>,
    /// Open state of the regenerate style menu.
    regenerate_menu_open: bool,
    focus_handle: FocusHandle,
    co_authors: Vec<CoAuthor>,
    adding_co_author: bool,
    new_author_name: Entity<TextInput>,
    new_author_email: Entity<TextInput>,
    collapsed: bool,
}

/// A snapshot of the commit editors, taken before the AI overwrites them.
#[derive(Debug, Clone, Default)]
struct PreviousMessage {
    summary: String,
    description: String,
    co_authors: Vec<CoAuthor>,
}

impl PreviousMessage {
    fn is_empty(&self) -> bool {
        self.summary.trim().is_empty()
            && self.description.trim().is_empty()
            && self.co_authors.is_empty()
    }
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
                    this.request_commit(cx);
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
            ai_state: AiState::Idle,
            ai_undo: None,
            regenerate_menu_open: false,
            focus_handle: cx.focus_handle(),
            co_authors: Vec::new(),
            adding_co_author: false,
            new_author_name,
            new_author_email,
            collapsed: false,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn toggle_collapsed(&mut self, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        cx.emit(CommitPanelEvent::CollapsedChanged);
        cx.notify();
    }

    pub fn set_message(&mut self, message: String, cx: &mut Context<Self>) {
        let (summary, description) = split_message(&message);
        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(summary, cx));
        self.description_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(description, cx));
        self.co_authors.clear();
        self.adding_co_author = false;
        cx.notify();
    }

    /// Whether a commit can be made right now: a summary has been typed and
    /// something is staged.
    pub fn can_commit(&self, cx: &Context<Self>) -> bool {
        !self.summary_editor.read(cx).is_empty() && self.staged_count > 0
    }

    /// Emit a commit request and reset the panel.
    ///
    /// Every entry point — the commit button, Enter in the summary field, and
    /// the Ctrl+Enter command — routes through here so they cannot disagree
    /// about the amend flag or the staged-changes guard. Returns whether the
    /// request was emitted.
    pub fn request_commit(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_commit(cx) {
            return false;
        }

        cx.emit(CommitPanelEvent::CommitRequested {
            message: self.message(cx),
            amend: self.amend,
        });

        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
        self.description_editor
            .update(cx, |e: &mut TextInput, cx| e.clear(cx));
        self.amend = false;
        self.co_authors.clear();
        self.adding_co_author = false;
        cx.notify();
        true
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

    pub fn staged_count(&self) -> usize {
        self.staged_count
    }

    pub fn set_staged_count(&mut self, count: usize, cx: &mut Context<Self>) {
        self.staged_count = count;
        cx.notify();
    }

    pub fn is_ai_generating(&self) -> bool {
        matches!(self.ai_state, AiState::Generating { .. })
    }

    /// A generation has started for this panel.
    ///
    /// The editors go read-only for the duration, so the user cannot type a
    /// message that the response is about to silently destroy.
    pub fn begin_ai_generation(&mut self, cx: &mut Context<Self>) {
        self.ai_state = AiState::Generating { progress: None };
        self.regenerate_menu_open = false;
        self.set_editors_read_only(true, cx);
        cx.notify();
    }

    /// Report what the model is doing right now ("Reading diff.rs").
    pub fn set_ai_progress(&mut self, progress: Option<String>, cx: &mut Context<Self>) {
        if let AiState::Generating { progress: slot } = &mut self.ai_state {
            *slot = progress;
            cx.notify();
        }
    }

    /// Apply a generated message, preserving anything the user had already
    /// written.
    ///
    /// The old behaviour overwrote both editors and cleared every co-author
    /// with no undo, so typing while waiting lost the work outright.
    pub fn apply_ai_message(&mut self, message: String, cx: &mut Context<Self>) {
        let previous = self.snapshot(cx);
        self.set_editors_read_only(false, cx);

        let (summary, description) = split_message(&message);
        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(summary, cx));
        self.description_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(description, cx));
        // Co-authors are the user's own attribution, never the model's to
        // remove.
        self.adding_co_author = false;

        self.ai_undo = (!previous.is_empty()).then_some(previous);
        self.ai_state = AiState::Completed;
        cx.notify();
    }

    /// The generation failed or was cancelled.
    pub fn fail_ai_generation(&mut self, cx: &mut Context<Self>) {
        self.set_editors_read_only(false, cx);
        self.ai_state = AiState::Failed;
        cx.notify();
    }

    /// Restore the message the AI replaced.
    pub fn undo_ai_message(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.ai_undo.take() else {
            return;
        };
        self.summary_editor
            .update(cx, |e: &mut TextInput, cx| e.set_text(previous.summary, cx));
        self.description_editor.update(cx, |e: &mut TextInput, cx| {
            e.set_text(previous.description, cx)
        });
        self.co_authors = previous.co_authors;
        self.ai_state = AiState::Idle;
        cx.notify();
    }

    /// Dismiss the post-generation controls without changing the message.
    pub fn dismiss_ai_state(&mut self, cx: &mut Context<Self>) {
        self.ai_state = AiState::Idle;
        self.ai_undo = None;
        self.regenerate_menu_open = false;
        cx.notify();
    }

    fn snapshot(&self, cx: &Context<Self>) -> PreviousMessage {
        PreviousMessage {
            summary: self.summary_editor.read(cx).text().to_string(),
            description: self.description_editor.read(cx).text().to_string(),
            co_authors: self.co_authors.clone(),
        }
    }

    fn set_editors_read_only(&self, read_only: bool, cx: &mut Context<Self>) {
        self.summary_editor
            .update(cx, |e: &mut TextInput, _cx| e.set_read_only(read_only));
        self.description_editor
            .update(cx, |e: &mut TextInput, _cx| e.set_read_only(read_only));
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

    /// The AI control in the panel header.
    ///
    /// Four states, and none of them is "absent": a vanishing control is
    /// worse than a disabled one, and a user who turned AI off previously saw
    /// no AI affordance at all and no route back.
    fn render_ai_control(
        &self,
        blocker: Option<AiBlocker>,
        use_tools: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors().clone();

        match &self.ai_state {
            AiState::Generating { progress } => {
                // The tool progress line goes here, not only to the status
                // bar: "Reading diff.rs" turns an opaque spinner into a
                // legible trace of what the model is actually doing.
                let label: SharedString = match progress {
                    Some(description) => description.clone().into(),
                    None if use_tools => "Generating (with tools)…".into(),
                    None => "Generating…".into(),
                };
                div()
                    .h_flex()
                    .flex_shrink_0()
                    .h(px(22.))
                    .pl(px(8.))
                    .pr(px(2.))
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
                        Label::new(label)
                            .size(LabelSize::XSmall)
                            .color(Color::Accent),
                    )
                    // Required, not decorative: with tools on a generation
                    // runs 30s or more, and there was previously no way to
                    // stop one short of restarting the app.
                    .child(
                        IconButton::new("ai-cancel", IconName::X)
                            .size(ButtonSize::Compact)
                            .color(Color::Muted)
                            .tooltip("Stop generating")
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                cx.emit(CommitPanelEvent::CancelAiMessage);
                            })),
                    )
                    .into_any_element()
            }

            AiState::Completed => self.render_post_generation_controls(cx),

            AiState::Failed => div()
                .h_flex()
                .flex_shrink_0()
                .gap(px(4.))
                .items_center()
                .child(
                    // A dismissed toast leaves no trace of why the field is
                    // still empty, so the panel keeps the marker.
                    rgitui_ui::Icon::new(IconName::AlertTriangle)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Error),
                )
                .child(
                    Button::new("ai-retry", "AI failed — retry")
                        .icon(IconName::Refresh)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Outlined)
                        .color(Color::Error)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.ai_state = AiState::Idle;
                            cx.emit(CommitPanelEvent::GenerateAiMessage);
                        })),
                )
                .into_any_element(),

            AiState::Idle => self.render_ai_trigger(blocker, cx),
        }
    }

    /// The idle AI button, carrying its own reason when it cannot be used.
    fn render_ai_trigger(
        &self,
        blocker: Option<AiBlocker>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let summary_present = !self.summary_editor.read(cx).is_empty();
        let label = match blocker {
            Some(blocker) => blocker.button_label(),
            // Say plainly that this will replace what is already written.
            None if summary_present => "Rewrite message",
            None => "AI Message",
        };

        let mut button = Button::new("ai-btn", label)
            .icon(IconName::Sparkle)
            .size(ButtonSize::Compact)
            .style(ButtonStyle::Outlined)
            .color(Color::Accent)
            // Disabled buttons are dropped from the tab order, so anything
            // that stays disabled becomes unreachable without a mouse. Only
            // the self-correcting case does.
            .disabled(blocker.is_some_and(|blocker| !blocker.is_actionable()));

        button = match blocker {
            Some(blocker) => button.tooltip(blocker.tooltip()),
            // Surface the shortcut. It was registered all along and never
            // shown, so the fastest path to the feature was invisible.
            None => button.tooltip_fn(crate::keymap::command_tooltip(
                "Generate a commit message from the staged diff",
                crate::CommandId::AiMessage,
            )),
        };

        button
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                match blocker {
                    // The fix is one click away; take the user to it rather
                    // than rendering a dead end.
                    Some(AiBlocker::Disabled) | Some(AiBlocker::NoApiKey) => {
                        cx.emit(CommitPanelEvent::OpenAiSettings)
                    }
                    Some(AiBlocker::NothingStaged) => {}
                    None => cx.emit(CommitPanelEvent::GenerateAiMessage),
                }
            }))
            .into_any_element()
    }

    /// Regenerate (with an optional style override) and Undo, offered right
    /// where dissatisfaction happens rather than in a settings page in another
    /// window.
    fn render_post_generation_controls(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors().clone();
        let mut row = div()
            .h_flex()
            .flex_shrink_0()
            .gap(px(4.))
            .items_center()
            .child(
                Button::new("ai-regenerate", "Regenerate")
                    .icon(IconName::Refresh)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .color(Color::Accent)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(CommitPanelEvent::RegenerateAiMessage { style: None });
                    })),
            )
            .child(
                IconButton::new("ai-regenerate-menu", IconName::ChevronDown)
                    .size(ButtonSize::Compact)
                    .color(Color::Muted)
                    .tooltip("Regenerate in a different style")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.regenerate_menu_open = !this.regenerate_menu_open;
                        cx.notify();
                    })),
            );

        if self.ai_undo.is_some() {
            row = row.child(
                Button::new("ai-undo", "Undo")
                    .icon(IconName::Undo)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .color(Color::Muted)
                    .tooltip("Restore the message the AI replaced")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.undo_ai_message(cx);
                    })),
            );
        }

        row = row.child(
            IconButton::new("ai-dismiss", IconName::X)
                .size(ButtonSize::Compact)
                .color(Color::Muted)
                .tooltip("Dismiss")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.dismiss_ai_state(cx);
                })),
        );

        if !self.regenerate_menu_open {
            return row.into_any_element();
        }

        let mut menu = div()
            .flex()
            .flex_col()
            .min_w(px(150.))
            .py(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .elevation_2(cx);

        for style in CommitStyle::ALL {
            let style = *style;
            menu = menu.child(
                div()
                    .id(ElementId::Name(format!("ai-style-{}", style.id()).into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(26.))
                    .mx(px(4.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.regenerate_menu_open = false;
                        cx.emit(CommitPanelEvent::RegenerateAiMessage { style: Some(style) });
                    }))
                    .child(
                        Label::new(style.display_name())
                            .size(LabelSize::XSmall)
                            .color(Color::Default),
                    ),
            );
        }

        div()
            .relative()
            .child(row)
            .child(gpui::deferred(
                gpui::anchored()
                    .snap_to_window_with_margin(px(8.))
                    .child(div().absolute().top(px(26.)).right(px(0.)).child(menu)),
            ))
            .into_any_element()
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
        let colors = cx.colors().clone();
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

        let commit_label = self.commit_button_label(summary_empty);

        // `has_ai_api_key()` reads the cached flag. The old
        // `ai_api_key().is_some()` deep-cloned the AI key, the HTTPS token and
        // every git provider token into fresh heap strings on every frame —
        // ~120 copies a second of every secret the app holds, dropped without
        // zeroization — purely to evaluate `.is_some()`.
        let ai_settings = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|s| {
                let settings = s.settings();
                (
                    settings.ai.enabled,
                    s.has_ai_api_key(),
                    settings.ai.use_tools,
                )
            })
            .unwrap_or((false, false, false));
        let (ai_enabled, has_api_key, ai_use_tools) = ai_settings;
        let blocker = ai_blocker(ai_enabled, has_api_key, self.staged_count);
        // Built before the tree so it can take `&mut Context` without
        // conflicting with the immutable borrows the layout holds.
        let ai_control =
            (!self.collapsed).then(|| self.render_ai_control(blocker, ai_use_tools, cx));

        div()
            .v_flex()
            .size_full()
            .overflow_hidden()
            .bg(colors.panel_background)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(COMMIT_PANEL_HEADER_HEIGHT))
                    .px(px(10.))
                    .items_center()
                    .justify_between()
                    .flex_shrink_0()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    // Left group: disclosure + label + badge
                    .child(
                        div()
                            .h_flex()
                            .gap(px(4.))
                            .items_center()
                            .flex_shrink_0()
                            .child(
                                IconButton::new(
                                    "toggle-commit-panel",
                                    if self.collapsed {
                                        IconName::ChevronUp
                                    } else {
                                        IconName::ChevronDown
                                    },
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Muted)
                                .tooltip(if self.collapsed {
                                    "Expand commit panel"
                                } else {
                                    "Collapse commit panel"
                                })
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_collapsed(cx);
                                    },
                                )),
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
                            ),
                    )
                    // Right group: the AI control, in whichever of its four
                    // states applies.
                    .when_some(ai_control, |el, control| el.child(control)),
            )
            .child(
                div()
                    .id("commit-content-area")
                    .track_focus(&self.focus_handle)
                    .v_flex()
                    .flex_1()
                    .min_h(px(120.))
                    .when(self.collapsed, |el| el.h(px(0.)).min_h(px(0.)))
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
                                    .flex_shrink_0()
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
                            .when_some(
                                crate::keymap::shortcut(crate::CommandId::Commit, cx)
                                    .filter(|_| can_commit),
                                |el, keystrokes| {
                                    el.child(
                                        Label::new(SharedString::from(keystrokes))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                },
                            )
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
                                        this.request_commit(cx);
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
    fn collapsed_panel_uses_only_header_height() {
        assert_eq!(commit_panel_height(420.0, true), 34.0);
        assert_eq!(commit_panel_height(420.0, false), 420.0);
    }

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

    // --- CoAuthor trailer tests ---

    #[test]
    fn co_author_trailer_format() {
        // CoAuthor trailer should follow git's Co-Authored-By convention
        let ca = CoAuthor {
            name: "Test User".into(),
            email: "test@example.com".into(),
        };
        let trailer = ca.trailer();
        assert!(trailer.starts_with("Co-Authored-By: "));
        assert!(trailer.contains("<test@example.com>"));
    }

    #[test]
    fn co_author_trailer_name_with_email() {
        // Name and email should appear in trailer
        let ca = CoAuthor {
            name: "Jane Doe".into(),
            email: "jane@example.org".into(),
        };
        let trailer = ca.trailer();
        assert!(trailer.contains("Jane Doe"));
        assert!(trailer.contains("<jane@example.org>"));
    }

    // ── the shared AI guard ───────────────────────────────────────

    /// The one predicate the button, Ctrl+G and the command palette all share.
    /// Only the button used to check `enabled` and `has_api_key`, so Ctrl+G
    /// with AI turned off still fired a full request and spent tokens.
    #[test]
    fn generation_is_allowed_only_when_everything_is_in_place() {
        assert_eq!(ai_blocker(true, true, 3), None);
    }

    #[test]
    fn each_missing_precondition_is_reported_in_priority_order() {
        // Disabled outranks everything: with AI off, "add a key" would be the
        // wrong instruction.
        assert_eq!(ai_blocker(false, false, 0), Some(AiBlocker::Disabled));
        assert_eq!(ai_blocker(false, true, 3), Some(AiBlocker::Disabled));
        assert_eq!(ai_blocker(true, false, 3), Some(AiBlocker::NoApiKey));
        assert_eq!(ai_blocker(true, true, 0), Some(AiBlocker::NothingStaged));
    }

    /// Never render a dead end when the fix is one click away — and disabled
    /// buttons drop out of the tab order, so anything left disabled becomes
    /// unreachable without a mouse.
    #[test]
    fn only_the_self_correcting_blocker_actually_disables_the_button() {
        assert!(AiBlocker::Disabled.is_actionable());
        assert!(AiBlocker::NoApiKey.is_actionable());
        assert!(!AiBlocker::NothingStaged.is_actionable());
    }

    #[test]
    fn every_blocker_states_its_reason_in_the_control_and_the_tooltip() {
        for blocker in [
            AiBlocker::Disabled,
            AiBlocker::NoApiKey,
            AiBlocker::NothingStaged,
        ] {
            assert!(!blocker.button_label().is_empty());
            assert!(!blocker.tooltip().is_empty());
            assert_ne!(blocker.button_label(), blocker.tooltip());
        }
    }

    // ── message splitting ─────────────────────────────────────────

    #[test]
    fn a_single_line_message_is_all_summary() {
        assert_eq!(
            split_message("feat: do the thing"),
            ("feat: do the thing".to_string(), String::new())
        );
    }

    #[test]
    fn a_body_is_separated_from_the_summary_and_trimmed() {
        assert_eq!(
            split_message(
                "feat: do it

Because reasons.
"
            ),
            ("feat: do it".to_string(), "Because reasons.".to_string())
        );
    }

    #[test]
    fn an_empty_message_splits_into_two_empty_halves() {
        assert_eq!(split_message(""), (String::new(), String::new()));
        assert_eq!(split_message("   "), (String::new(), String::new()));
    }

    // ── the AI overwrite snapshot ─────────────────────────────────

    /// Typing while a generation ran used to lose the work outright: both
    /// editors were overwritten and every co-author cleared, with no undo.
    #[test]
    fn a_snapshot_with_any_content_is_worth_restoring() {
        let empty = PreviousMessage::default();
        assert!(empty.is_empty());

        let with_summary = PreviousMessage {
            summary: "wip".into(),
            ..PreviousMessage::default()
        };
        assert!(!with_summary.is_empty());

        let with_co_author = PreviousMessage {
            co_authors: vec![CoAuthor {
                name: "Jane Doe".into(),
                email: "jane@example.org".into(),
            }],
            ..PreviousMessage::default()
        };
        assert!(!with_co_author.is_empty());
    }

    #[test]
    fn whitespace_alone_is_not_worth_restoring() {
        let blank = PreviousMessage {
            summary: "  ".into(),
            description: "
"
            .into(),
            ..PreviousMessage::default()
        };
        assert!(blank.is_empty());
    }
}
