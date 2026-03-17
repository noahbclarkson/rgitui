use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{div, px, Context, ElementId, Render, SharedString, WeakEntity, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

/// The kind of toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
    Warning,
}

impl ToastKind {
    fn color(&self) -> Color {
        match self {
            ToastKind::Success => Color::Success,
            ToastKind::Error => Color::Error,
            ToastKind::Info => Color::Info,
            ToastKind::Warning => Color::Warning,
        }
    }

    fn icon(&self) -> IconName {
        match self {
            ToastKind::Success => IconName::Check,
            ToastKind::Error => IconName::X,
            ToastKind::Info => IconName::Info,
            ToastKind::Warning => IconName::FileConflict,
        }
    }
}

/// A single toast notification.
struct Toast {
    id: usize,
    message: String,
    kind: ToastKind,
    _created_at: Instant,
}

/// Manages a stack of transient toast notifications.
pub struct ToastLayer {
    toasts: Vec<Toast>,
    next_id: usize,
}

impl ToastLayer {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            toasts: Vec::new(),
            next_id: 0,
        }
    }

    /// Show a new toast notification. It will auto-dismiss after 4 seconds.
    pub fn show_toast(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;

        self.toasts.push(Toast {
            id,
            message: message.into(),
            kind,
            _created_at: Instant::now(),
        });

        // Cap at 3 visible toasts — remove oldest if over limit
        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }

        // Schedule auto-dismiss
        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor().timer(Duration::from_secs(4)).await;
                this.update(cx, |this, cx| {
                    this.dismiss_toast(id, cx);
                })
                .ok();
            },
        )
        .detach();

        cx.notify();
    }

    /// Remove a toast by ID.
    fn dismiss_toast(&mut self, id: usize, cx: &mut Context<Self>) {
        self.toasts.retain(|t| t.id != id);
        cx.notify();
    }
}

impl Render for ToastLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.toasts.is_empty() {
            return div().into_any_element();
        }

        let colors = cx.colors();
        let status = cx.status();

        let mut stack = div()
            .absolute()
            .bottom(px(36.))
            .right(px(12.))
            .v_flex()
            .gap(px(6.))
            .w(px(300.));

        for toast in &self.toasts {
            let toast_id = toast.id;
            let kind_color = toast.kind.color();
            let icon_name = toast.kind.icon();

            let bg = match toast.kind {
                ToastKind::Success => status.success_background,
                ToastKind::Error => status.error_background,
                ToastKind::Info => status.info_background,
                ToastKind::Warning => status.warning_background,
            };

            let border_color = match toast.kind {
                ToastKind::Success => status.success,
                ToastKind::Error => status.error,
                ToastKind::Info => status.info,
                ToastKind::Warning => status.warning,
            };

            let entity = cx.entity().downgrade();
            let message: SharedString = toast.message.clone().into();

            stack =
                stack.child(
                    div()
                        .id(ElementId::NamedInteger("toast".into(), toast_id as u64))
                        .h_flex()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(colors.elevated_surface_background)
                        .border_1()
                        .border_color(border_color)
                        .rounded(px(8.))
                        .elevation_2(cx)
                        .items_center()
                        .child(
                            div().flex_shrink_0().p_1().rounded_md().bg(bg).child(
                                Icon::new(icon_name).size(IconSize::Small).color(kind_color),
                            ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Label::new(message)
                                    .size(LabelSize::Small)
                                    .color(Color::Default),
                            ),
                        )
                        .child(
                            div()
                                .id(ElementId::NamedInteger(
                                    "toast-close".into(),
                                    toast_id as u64,
                                ))
                                .flex_shrink_0()
                                .cursor_pointer()
                                .rounded_md()
                                .p(px(2.))
                                .hover(|s| s.bg(colors.ghost_element_hover))
                                .on_click(move |_event, _window, cx| {
                                    entity
                                        .update(cx, |this, cx| {
                                            this.dismiss_toast(toast_id, cx);
                                        })
                                        .ok();
                                })
                                .child(
                                    Icon::new(IconName::X)
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        ),
                );
        }

        stack.into_any_element()
    }
}
