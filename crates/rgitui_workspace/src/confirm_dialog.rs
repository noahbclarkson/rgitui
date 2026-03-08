use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Label, LabelSize};

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
}

/// Events emitted by the confirmation dialog.
#[derive(Debug, Clone)]
pub enum ConfirmDialogEvent {
    Confirmed(ConfirmAction),
    Cancelled,
}

/// A simple modal confirmation dialog for destructive operations.
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
}

impl Render for ConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("confirm-dialog").into_any_element();
        }

        let title: SharedString = self.title.clone().into();
        let message: SharedString = self.message.clone().into();

        let _confirm_bg = colors.element_selected;
        let _cancel_hover = colors.ghost_element_hover;

        let backdrop = div()
            .id("confirm-dialog-backdrop")
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
                    .w(px(400.))
                    .bg(colors.elevated_surface_background)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_lg()
                    .elevation_3(cx)
                    .p_4()
                    .gap_3()
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    // Title
                    .child(
                        Label::new(title)
                            .size(LabelSize::Large)
                            .weight(gpui::FontWeight::BOLD),
                    )
                    // Message
                    .child(
                        Label::new(message)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    // Buttons row
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(
                                Button::new("confirm-cancel", "Cancel")
                                    .size(ButtonSize::Default)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("confirm-ok", "Confirm")
                                    .size(ButtonSize::Default)
                                    .style(ButtonStyle::Filled)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.confirm(cx);
                                    })),
                            ),
                    ),
            );

        backdrop.into_any_element()
    }
}
