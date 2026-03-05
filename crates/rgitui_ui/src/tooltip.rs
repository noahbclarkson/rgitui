use gpui::prelude::*;
use gpui::{div, App, SharedString, Window};
use rgitui_theme::StyledExt;

use crate::Label;

/// A simple text tooltip.
#[derive(IntoElement)]
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .elevation_2(cx)
            .px_2()
            .py_1()
            .child(Label::new(self.text).size(crate::LabelSize::Small))
    }
}
