use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, EventEmitter, Render, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, VerticalDivider};

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

        let bg = colors.element_background;
        let hover = colors.element_hover;
        let active = colors.element_active;
        let border = colors.border_variant;
        let dis_bg = colors.element_disabled;
        let dis_border = colors.border_disabled;

        div()
            .h_flex()
            .w_full()
            .h(px(44.))
            .px_3()
            .gap_2()
            .items_center()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            // Fetch
            .child(
                div()
                    .id("tb-fetch")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .hover(move |s| s.bg(hover))
                    .active(move |s| s.bg(active))
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Fetch);
                    }))
                    .child(Icon::new(IconName::Refresh).size(IconSize::Small))
                    .child(Label::new("Fetch").size(LabelSize::XSmall)),
            )
            // Pull
            .child({
                let disabled = !self.can_pull;
                let mut el = div()
                    .id("tb-pull")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1();
                if disabled {
                    el = el
                        .border_color(dis_border)
                        .bg(dis_bg)
                        .child(Icon::new(IconName::ArrowDown).size(IconSize::Small).color(Color::Disabled))
                        .child(Label::new("Pull").size(LabelSize::XSmall).color(Color::Disabled));
                } else {
                    el = el
                        .border_color(border)
                        .bg(bg)
                        .hover(move |s| s.bg(hover))
                        .active(move |s| s.bg(active))
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(ToolbarEvent::Pull);
                        }))
                        .child(Icon::new(IconName::ArrowDown).size(IconSize::Small))
                        .child(Label::new("Pull").size(LabelSize::XSmall));
                }
                el
            })
            // Push
            .child({
                let disabled = !self.can_push;
                let mut el = div()
                    .id("tb-push")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1();
                if disabled {
                    el = el
                        .border_color(dis_border)
                        .bg(dis_bg)
                        .child(Icon::new(IconName::ArrowUp).size(IconSize::Small).color(Color::Disabled))
                        .child(Label::new("Push").size(LabelSize::XSmall).color(Color::Disabled));
                } else {
                    el = el
                        .border_color(border)
                        .bg(bg)
                        .hover(move |s| s.bg(hover))
                        .active(move |s| s.bg(active))
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(ToolbarEvent::Push);
                        }))
                        .child(Icon::new(IconName::ArrowUp).size(IconSize::Small))
                        .child(Label::new("Push").size(LabelSize::XSmall));
                }
                el
            })
            .child(VerticalDivider::new())
            // Branch
            .child(
                div()
                    .id("tb-branch")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .hover(move |s| s.bg(hover))
                    .active(move |s| s.bg(active))
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                        cx.emit(ToolbarEvent::Branch);
                    }))
                    .child(Icon::new(IconName::GitBranch).size(IconSize::Small))
                    .child(Label::new("Branch").size(LabelSize::XSmall)),
            )
            .child(VerticalDivider::new())
            // Stash
            .child({
                let disabled = !self.has_changes;
                let mut el = div()
                    .id("tb-stash")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1();
                if disabled {
                    el = el
                        .border_color(dis_border)
                        .bg(dis_bg)
                        .child(Icon::new(IconName::Stash).size(IconSize::Small).color(Color::Disabled))
                        .child(Label::new("Stash").size(LabelSize::XSmall).color(Color::Disabled));
                } else {
                    el = el
                        .border_color(border)
                        .bg(bg)
                        .hover(move |s| s.bg(hover))
                        .active(move |s| s.bg(active))
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(ToolbarEvent::StashSave);
                        }))
                        .child(Icon::new(IconName::Stash).size(IconSize::Small))
                        .child(Label::new("Stash").size(LabelSize::XSmall));
                }
                el
            })
            // Pop
            .child({
                let disabled = !self.has_stashes;
                let mut el = div()
                    .id("tb-pop")
                    .h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(4.))
                    .border_1();
                if disabled {
                    el = el
                        .border_color(dis_border)
                        .bg(dis_bg)
                        .child(Icon::new(IconName::Undo).size(IconSize::Small).color(Color::Disabled))
                        .child(Label::new("Pop").size(LabelSize::XSmall).color(Color::Disabled));
                } else {
                    el = el
                        .border_color(border)
                        .bg(bg)
                        .hover(move |s| s.bg(hover))
                        .active(move |s| s.bg(active))
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(ToolbarEvent::StashPop);
                        }))
                        .child(Icon::new(IconName::Undo).size(IconSize::Small))
                        .child(Label::new("Pop").size(LabelSize::XSmall));
                }
                el
            })
            .child(div().flex_1())
            .child(
                Label::new("Ready")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}
