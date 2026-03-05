use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, Window};
use rgitui_theme::StyledExt;
use smallvec::SmallVec;

use crate::{Divider, Label, LabelSize};

/// A modal dialog container with header, body, and optional footer.
#[derive(IntoElement)]
pub struct Modal {
    title: Option<String>,
    body: SmallVec<[AnyElement; 2]>,
    footer: Option<AnyElement>,
    width: f32,
}

impl Modal {
    pub fn new() -> Self {
        Self {
            title: None,
            body: SmallVec::new(),
            footer: None,
            width: 480.0,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.body.push(child.into_any_element());
        self
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut container = div()
            .v_flex()
            .w(px(self.width))
            .max_h(px(600.))
            .elevation_3(cx)
            .overflow_hidden();

        // Header
        if let Some(title) = self.title {
            container = container
                .child(
                    div()
                        .h_flex()
                        .px_4()
                        .py_3()
                        .child(
                            Label::new(title)
                                .size(LabelSize::Large)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(Divider::new());
        }

        // Body
        let mut body = div().id("modal-body").v_flex().flex_1().p_4().gap_3().overflow_y_scroll();
        for child in self.body {
            body = body.child(child);
        }
        container = container.child(body);

        // Footer
        if let Some(footer) = self.footer {
            container = container
                .child(Divider::new())
                .child(
                    div()
                        .h_flex()
                        .px_4()
                        .py_3()
                        .justify_end()
                        .gap_2()
                        .child(footer),
                );
        }

        // Backdrop
        div()
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
            .child(container)
    }
}
