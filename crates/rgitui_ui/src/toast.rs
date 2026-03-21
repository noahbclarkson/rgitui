use gpui::prelude::*;
use gpui::{div, px, App, ElementId, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::{Icon, IconName, IconSize, Label, LabelSize};

/// The severity level of a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Success,
    Error,
    Warning,
    Info,
}

impl ToastLevel {
    pub fn color(&self) -> Color {
        match self {
            ToastLevel::Success => Color::Success,
            ToastLevel::Error => Color::Error,
            ToastLevel::Warning => Color::Warning,
            ToastLevel::Info => Color::Info,
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            ToastLevel::Success => IconName::CheckCircle,
            ToastLevel::Error => IconName::XCircle,
            ToastLevel::Warning => IconName::AlertTriangle,
            ToastLevel::Info => IconName::Info,
        }
    }
}

/// A compact toast notification pill component.
///
/// Renders as a small rounded pill with an icon and message text,
/// styled according to the notification level.
#[derive(IntoElement)]
pub struct Toast {
    id: ElementId,
    message: SharedString,
    level: ToastLevel,
}

impl Toast {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<SharedString>,
        level: ToastLevel,
    ) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            level,
        }
    }
}

impl RenderOnce for Toast {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let status = cx.status();

        let level_color = self.level.color();
        let icon_name = self.level.icon();

        let icon_bg = match self.level {
            ToastLevel::Success => status.success_background,
            ToastLevel::Error => status.error_background,
            ToastLevel::Warning => status.warning_background,
            ToastLevel::Info => status.info_background,
        };

        let border = match self.level {
            ToastLevel::Success => status.success,
            ToastLevel::Error => status.error,
            ToastLevel::Warning => status.warning,
            ToastLevel::Info => status.info,
        };

        div()
            .id(self.id)
            .h_flex()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(border)
            .rounded(px(8.))
            .elevation_3(cx)
            .items_center()
            .child(
                div()
                    .flex_shrink_0()
                    .p(px(3.))
                    .rounded_md()
                    .bg(icon_bg)
                    .child(
                        Icon::new(icon_name)
                            .size(IconSize::Small)
                            .color(level_color),
                    ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(self.message)
                        .size(LabelSize::Small)
                        .color(Color::Default),
                ),
            )
    }
}
