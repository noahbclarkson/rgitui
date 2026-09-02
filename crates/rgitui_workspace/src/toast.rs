use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, Context, ElementId, Render, WeakEntity, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Toast};

pub use rgitui_ui::ToastLevel as ToastKind;

/// Internal state for a single active toast.
struct ToastEntry {
    id: usize,
    message: String,
    kind: ToastKind,
    /// An optional single action, e.g. `[Open Settings]` or `[Retry]`.
    action: Option<(String, Rc<rgitui_ui::ClickHandler>)>,
}

/// Manages a stack of transient toast notifications.
pub struct ToastLayer {
    toasts: Vec<ToastEntry>,
    next_id: usize,
}

impl ToastLayer {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            toasts: Vec::new(),
            next_id: 0,
        }
    }

    /// Show a new toast notification.
    ///
    /// How long it stays is decided by its level: an error is sticky, because
    /// one hardcoded three-second timeout meant errors vanished before they
    /// could be read.
    pub fn show_toast(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        self.push(message.into(), kind, None, cx);
    }

    /// Show a toast carrying a single action.
    pub fn show_toast_with_action(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        action_label: impl Into<String>,
        on_action: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let action = Some((
            action_label.into(),
            Rc::new(Box::new(on_action) as rgitui_ui::ClickHandler),
        ));
        self.push(message.into(), kind, action, cx);
    }

    fn push(
        &mut self,
        message: String,
        kind: ToastKind,
        action: Option<(String, Rc<rgitui_ui::ClickHandler>)>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;

        self.toasts.push(ToastEntry {
            id,
            message,
            kind,
            action,
        });

        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }

        if let Some(after) = kind.auto_dismiss_after() {
            cx.spawn(
                async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                    cx.background_executor().timer(after).await;
                    this.update(cx, |this, cx| {
                        this.dismiss_toast(id, cx);
                    })
                    .ok();
                },
            )
            .detach();
        }

        cx.notify();
    }

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

        let mut stack = div()
            .absolute()
            .bottom(px(36.))
            .right(px(12.))
            .v_flex()
            .gap(px(6.))
            .w(px(320.));

        for entry in &self.toasts {
            let toast_id = entry.id;
            let entity = cx.entity().downgrade();

            let mut toast = Toast::new(
                ElementId::NamedInteger("toast".into(), toast_id as u64),
                entry.message.clone(),
                entry.kind,
            );
            if let Some((label, handler)) = &entry.action {
                let handler = handler.clone();
                let dismiss_entity = cx.entity().downgrade();
                toast = toast.action(label.clone(), move |event, window, cx| {
                    handler(event, window, cx);
                    // Acting on a toast is also dismissing it; leaving it on
                    // screen would invite a second, now-pointless click.
                    dismiss_entity
                        .update(cx, |this, cx| this.dismiss_toast(toast_id, cx))
                        .ok();
                });
            }

            stack = stack.child(
                div()
                    .id(ElementId::NamedInteger("toast-row".into(), toast_id as u64))
                    .h_flex()
                    .items_center()
                    .gap(px(0.))
                    .child(div().flex_1().min_w_0().child(toast))
                    .child(
                        div()
                            .id(ElementId::NamedInteger(
                                "toast-close".into(),
                                toast_id as u64,
                            ))
                            .absolute()
                            .top(px(4.))
                            .right(px(4.))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: usize, message: &str, kind: ToastKind) -> ToastEntry {
        ToastEntry {
            id,
            message: message.to_string(),
            kind,
            action: None,
        }
    }

    #[test]
    fn a_toast_entry_carries_its_message_and_level() {
        let success = entry(0, "test message", ToastKind::Success);
        assert_eq!(success.message, "test message");
        assert_eq!(success.kind, ToastKind::Success);
        assert!(success.action.is_none());

        assert_eq!(
            entry(1, "error occurred", ToastKind::Error).kind,
            ToastKind::Error
        );
    }

    /// A three-second timeout on every level meant errors vanished before
    /// they could be read, and a "Generating..." notice expired mid-operation
    /// on a tool-calling generation that runs 30s or more.
    #[test]
    fn errors_stay_until_dismissed_while_lesser_levels_expire() {
        assert_eq!(ToastKind::Error.auto_dismiss_after(), None);
        assert_eq!(
            ToastKind::Warning.auto_dismiss_after(),
            Some(std::time::Duration::from_secs(6))
        );
        assert_eq!(
            ToastKind::Info.auto_dismiss_after(),
            Some(std::time::Duration::from_secs(3))
        );
        assert_eq!(
            ToastKind::Success.auto_dismiss_after(),
            Some(std::time::Duration::from_secs(3))
        );
    }

    #[test]
    fn a_warning_lingers_longer_than_an_info() {
        let warning = ToastKind::Warning.auto_dismiss_after().unwrap();
        let info = ToastKind::Info.auto_dismiss_after().unwrap();
        assert!(warning > info);
    }

    #[test]
    fn every_level_maps_to_a_distinct_colour_and_icon() {
        assert_eq!(ToastKind::Success.color(), Color::Success);
        assert_eq!(ToastKind::Error.color(), Color::Error);
        assert_eq!(ToastKind::Warning.color(), Color::Warning);
        assert_eq!(ToastKind::Info.color(), Color::Info);

        assert_eq!(ToastKind::Success.icon(), IconName::CheckCircle);
        assert_eq!(ToastKind::Error.icon(), IconName::XCircle);
        assert_eq!(ToastKind::Warning.icon(), IconName::AlertTriangle);
        assert_eq!(ToastKind::Info.icon(), IconName::Info);
    }
}
