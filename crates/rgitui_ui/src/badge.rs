use gpui::prelude::*;
use gpui::{div, px, App, Hsla, SharedString, Window};
use rgitui_theme::{Color, StyledExt};

use crate::{Label, LabelSize};

/// A small badge/chip for displaying tags, status labels, branch names, etc.
#[derive(IntoElement)]
pub struct Badge {
    text: SharedString,
    color: Color,
    italic: bool,
    bold: bool,
    prefix: Option<SharedString>,
}

impl Badge {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            color: Color::Accent,
            italic: false,
            bold: false,
            prefix: None,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_color = self.color.color(cx);
        let bg = Hsla {
            a: 0.15,
            ..text_color
        };
        let border = Hsla {
            a: 0.3,
            ..text_color
        };

        let weight = if self.bold {
            gpui::FontWeight::BOLD
        } else {
            gpui::FontWeight::SEMIBOLD
        };

        let mut label = Label::new(self.text)
            .size(LabelSize::XSmall)
            .weight(weight)
            .color(self.color);

        if self.italic {
            label = label.italic();
        }

        let mut container = div()
            .h_flex()
            .gap(px(2.))
            .px(px(6.))
            .py(px(1.))
            .h(px(20.))
            .items_center()
            .rounded(px(10.))
            .bg(bg)
            .border_1()
            .border_color(border);

        if let Some(prefix_text) = self.prefix {
            container = container.child(
                Label::new(prefix_text)
                    .size(LabelSize::XSmall)
                    .color(self.color),
            );
        }

        container.child(label)
    }
}
