use gpui::prelude::*;
use gpui::{div, px, App, Window};
use rgitui_theme::ActiveTheme;

/// A horizontal divider line.
#[derive(IntoElement)]
pub struct Divider;

impl Divider {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for Divider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border_color = cx.colors().border_variant;
        div()
            .w_full()
            .h(px(1.))
            .bg(border_color)
    }
}

/// A vertical divider line.
#[derive(IntoElement)]
pub struct VerticalDivider;

impl VerticalDivider {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for VerticalDivider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border_color = cx.colors().border_variant;
        div()
            .h_full()
            .w(px(1.))
            .bg(border_color)
    }
}
