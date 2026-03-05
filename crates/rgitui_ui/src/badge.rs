use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{Color, StyledExt};

use crate::{Label, LabelSize};

/// A small badge/chip for displaying tags, status labels, branch names, etc.
#[derive(IntoElement)]
pub struct Badge {
    text: SharedString,
    color: Color,
}

impl Badge {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            color: Color::Accent,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_color = self.color.color(cx);
        let bg = gpui::Hsla {
            a: 0.2,
            ..text_color
        };
        let border = gpui::Hsla {
            a: 0.3,
            ..text_color
        };

        div()
            .h_flex()
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .bg(bg)
            .border_1()
            .border_color(border)
            .child(
                Label::new(self.text)
                    .size(LabelSize::XSmall)
                    .color(self.color),
            )
    }
}
