use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{Color, StyledExt};

use crate::{Label, LabelSize};

/// Spinner sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpinnerSize {
    Small,
    #[default]
    Medium,
}

/// A simple loading spinner indicator with an optional label.
///
/// Renders a colored dot with a text label to indicate loading state.
#[derive(IntoElement)]
pub struct Spinner {
    size: SpinnerSize,
    label: Option<SharedString>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            size: SpinnerSize::default(),
            label: None,
        }
    }

    pub fn size(mut self, size: SpinnerSize) -> Self {
        self.size = size;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (dot_size, label_size, gap) = match self.size {
            SpinnerSize::Small => (px(6.), LabelSize::XSmall, px(4.)),
            SpinnerSize::Medium => (px(8.), LabelSize::Small, px(6.)),
        };

        let text_color = Color::Accent.color(cx);

        let mut el = div()
            .h_flex()
            .gap(gap)
            .items_center()
            .child(
                div()
                    .w(dot_size)
                    .h(dot_size)
                    .rounded_full()
                    .bg(text_color),
            );

        if let Some(label_text) = self.label {
            el = el.child(
                Label::new(label_text)
                    .size(label_size)
                    .color(Color::Muted),
            );
        }

        el
    }
}
