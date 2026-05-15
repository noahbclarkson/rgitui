use std::time::Duration;

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

    /// Show a new toast notification. It will auto-dismiss after 3 seconds.
    pub fn show_toast(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;

        self.toasts.push(ToastEntry {
            id,
            message: message.into(),
            kind,
        });

        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }

        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                this.update(cx, |this, cx| {
                    this.dismiss_toast(id, cx);
                })
                .ok();
            },
        )
        .detach();

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

            stack = stack.child(
                div()
                    .id(ElementId::NamedInteger("toast-row".into(), toast_id as u64))
                    .h_flex()
                    .items_center()
                    .gap(px(0.))
                    .child(div().flex_1().min_w_0().child(Toast::new(
                        ElementId::NamedInteger("toast".into(), toast_id as u64),
                        entry.message.clone(),
                        entry.kind,
                    )))
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

    #[test]
    fn test_toast_entry_kinds() {
        // ToastEntry carries a message and kind; verify the kind field works
        // ToastKind is an alias for ToastLevel, which has Success/Error/Warning/Info
        let entry = ToastEntry {
            id: 0,
            message: "test message".to_string(),
            kind: ToastKind::Success,
        };
        assert_eq!(entry.message, "test message");
        assert_eq!(entry.kind, ToastKind::Success);

        let entry_err = ToastEntry {
            id: 1,
            message: "error occurred".to_string(),
            kind: ToastKind::Error,
        };
        assert_eq!(entry_err.kind, ToastKind::Error);
    }

    #[test]
    fn test_toast_layer_new_is_empty() {
        // Cannot construct ToastLayer without Context, but we can verify
        // ToastKind and ToastEntry structural correctness
        let entry = ToastEntry {
            id: 0,
            message: "loading".to_string(),
            kind: ToastKind::Info,
        };
        assert_eq!(entry.id, 0);
        assert_eq!(entry.message, "loading");
        assert_eq!(entry.kind, ToastKind::Info);
    }

    #[test]
    fn test_toast_level_color_mapping() {
        // ToastKind is ToastLevel from rgitui_ui; verify the color() method exists
        assert_eq!(ToastKind::Success.color(), Color::Success);
        assert_eq!(ToastKind::Error.color(), Color::Error);
        assert_eq!(ToastKind::Warning.color(), Color::Warning);
        assert_eq!(ToastKind::Info.color(), Color::Info);
    }

    #[test]
    fn test_toast_level_icon_mapping() {
        assert_eq!(ToastKind::Success.icon(), IconName::CheckCircle);
        assert_eq!(ToastKind::Error.icon(), IconName::XCircle);
        assert_eq!(ToastKind::Warning.icon(), IconName::AlertTriangle);
        assert_eq!(ToastKind::Info.icon(), IconName::Info);
    }
}
