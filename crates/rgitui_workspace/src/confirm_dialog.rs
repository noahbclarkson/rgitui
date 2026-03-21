use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize};

/// The action that was confirmed (so the workspace knows what to do).
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    DiscardFile(String),
    DiscardAll,
    ForcePush,
    StashDrop(usize),
    BranchDelete(String),
    TagDelete(String),
    RemoveRemote(String),
    ResetHard(String),
    AbortMerge,
}

/// Events emitted by the confirmation dialog.
#[derive(Debug, Clone)]
pub enum ConfirmDialogEvent {
    Confirmed(ConfirmAction),
    Cancelled,
}

/// A modal confirmation dialog for destructive operations.
pub struct ConfirmDialog {
    visible: bool,
    title: String,
    message: String,
    action: Option<ConfirmAction>,
    focus_handle: FocusHandle,
}

impl EventEmitter<ConfirmDialogEvent> for ConfirmDialog {}

impl ConfirmDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            title: String::new(),
            message: String::new(),
            action: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog with the given title, message, and action to confirm.
    pub fn show(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        action: ConfirmAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.title = title.into();
        self.message = message.into();
        self.action = Some(action);
        self.visible = true;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Show the dialog without focusing (for contexts where Window is unavailable).
    pub fn show_visible(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        action: ConfirmAction,
        cx: &mut Context<Self>,
    ) {
        self.title = title.into();
        self.message = message.into();
        self.action = Some(action);
        self.visible = true;
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(action) = self.action.take() {
            self.visible = false;
            cx.emit(ConfirmDialogEvent::Confirmed(action));
            cx.notify();
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.action = None;
        cx.emit(ConfirmDialogEvent::Cancelled);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(cx),
            "enter" => self.confirm(cx),
            _ => {}
        }
    }

    fn is_destructive(&self) -> bool {
        matches!(
            &self.action,
            Some(
                ConfirmAction::DiscardFile(_)
                    | ConfirmAction::DiscardAll
                    | ConfirmAction::BranchDelete(_)
                    | ConfirmAction::TagDelete(_)
                    | ConfirmAction::RemoveRemote(_)
                    | ConfirmAction::StashDrop(_)
                    | ConfirmAction::ResetHard(_)
                    | ConfirmAction::AbortMerge
            )
        )
    }

    fn severity_icon(&self) -> IconName {
        match &self.action {
            Some(ConfirmAction::ForcePush) => IconName::AlertTriangle,
            Some(_) => IconName::Trash,
            None => IconName::Check,
        }
    }

    fn severity_color(&self) -> Color {
        match &self.action {
            Some(ConfirmAction::ForcePush) => Color::Warning,
            Some(_) if self.is_destructive() => Color::Error,
            _ => Color::Accent,
        }
    }

    fn confirm_label(&self) -> &'static str {
        match &self.action {
            Some(ConfirmAction::DiscardFile(_) | ConfirmAction::DiscardAll) => "Discard",
            Some(ConfirmAction::BranchDelete(_)) => "Delete Branch",
            Some(ConfirmAction::TagDelete(_)) => "Delete Tag",
            Some(ConfirmAction::RemoveRemote(_)) => "Remove",
            Some(ConfirmAction::StashDrop(_)) => "Drop Stash",
            Some(ConfirmAction::ResetHard(_)) => "Reset",
            Some(ConfirmAction::AbortMerge) => "Abort",
            Some(ConfirmAction::ForcePush) => "Force Push",
            None => "Confirm",
        }
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("confirm-dialog").into_any_element();
        }

        let colors = cx.colors();
        let title: SharedString = self.title.clone().into();
        let message: SharedString = self.message.clone().into();
        let icon = self.severity_icon();
        let color = self.severity_color();
        let confirm_label = self.confirm_label();
        let is_destructive = self.is_destructive();

        let icon_bg = color.color(cx);
        let icon_bg_subtle = gpui::Hsla {
            a: 0.12,
            ..icon_bg
        };

        div()
            .id("confirm-dialog-backdrop")
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
                this.cancel(cx);
            }))
            .child(
                div()
                    .id("confirm-dialog-modal")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .v_flex()
                    .w(px(420.))
                    .elevation_3(cx)
                    .p(px(20.))
                    .gap(px(16.))
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
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
                                    .bg(icon_bg_subtle)
                                    .child(
                                        Icon::new(icon)
                                            .size(IconSize::Medium)
                                            .color(color),
                                    ),
                            )
                            .child(
                                Label::new(title)
                                    .size(LabelSize::Large)
                                    .weight(gpui::FontWeight::BOLD),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(48.))
                            .child(
                                Label::new(message)
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(colors.border_variant)
                            .h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                Label::new("Enter to confirm · Esc to cancel")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                            )
                            .child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("confirm-cancel", "Cancel")
                                            .size(ButtonSize::Default)
                                            .style(ButtonStyle::Subtle)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.cancel(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("confirm-ok", confirm_label)
                                            .icon(icon)
                                            .size(ButtonSize::Default)
                                            .style(if is_destructive {
                                                ButtonStyle::Tinted(
                                                    rgitui_ui::TintColor::Error,
                                                )
                                            } else {
                                                ButtonStyle::Filled
                                            })
                                            .color(color)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.confirm(cx);
                                                },
                                            )),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
