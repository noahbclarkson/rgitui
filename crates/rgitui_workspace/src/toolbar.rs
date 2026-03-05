use gpui::prelude::*;
use gpui::{div, ClickEvent, Context, EventEmitter, Render, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, VerticalDivider, Label, LabelSize};

/// Events emitted by the toolbar.
#[derive(Debug, Clone)]
pub enum ToolbarEvent {
    Fetch,
    Pull,
    Push,
    Branch,
    StashSave,
    StashPop,
    Undo,
    Redo,
}

/// The main toolbar with quick action buttons.
pub struct Toolbar {
    can_push: bool,
    can_pull: bool,
    has_stashes: bool,
    has_changes: bool,
}

impl EventEmitter<ToolbarEvent> for Toolbar {}

impl Toolbar {
    pub fn new() -> Self {
        Self {
            can_push: true,
            can_pull: true,
            has_stashes: false,
            has_changes: false,
        }
    }

    pub fn set_state(
        &mut self,
        can_push: bool,
        can_pull: bool,
        has_stashes: bool,
        has_changes: bool,
        cx: &mut Context<Self>,
    ) {
        self.can_push = can_push;
        self.can_pull = can_pull;
        self.has_stashes = has_stashes;
        self.has_changes = has_changes;
        cx.notify();
    }
}

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        div()
            .h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            // Fetch
            .child(
                Button::new("fetch-btn", "Fetch")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Fetch);
                    })),
            )
            // Pull
            .child(
                Button::new("pull-btn", "Pull")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .disabled(!self.can_pull)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Pull);
                    })),
            )
            // Push
            .child(
                Button::new("push-btn", "Push")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .disabled(!self.can_push)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Push);
                    })),
            )
            // Divider
            .child(VerticalDivider::new())
            // Branch
            .child(
                Button::new("branch-btn", "Branch")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Filled)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Branch);
                    })),
            )
            // Stash
            .child(
                Button::new("stash-btn", "Stash")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .disabled(!self.has_changes)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::StashSave);
                    })),
            )
            // Pop stash
            .child(
                Button::new("pop-stash-btn", "Pop")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Outlined)
                    .disabled(!self.has_stashes)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::StashPop);
                    })),
            )
            // Spacer
            .child(div().flex_1())
            // Status label
            .child(
                Label::new("Ready")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}
