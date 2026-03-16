use gpui::prelude::*;
use gpui::{div, px, App, ClickEvent, CursorStyle, ElementId, Window};
use rgitui_theme::ActiveTheme;

use crate::{Icon, IconName, IconSize};

/// A checkbox with checked/unchecked/indeterminate states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Unchecked,
    Checked,
    Indeterminate,
}

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    state: CheckState,
    disabled: bool,
    on_click: Option<crate::ClickHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, state: CheckState) -> Self {
        Self {
            id: id.into(),
            state,
            disabled: false,
            on_click: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        let (bg, border, icon) = match self.state {
            CheckState::Checked => (
                colors.text_accent,
                colors.text_accent,
                Some(IconName::Check),
            ),
            CheckState::Indeterminate => (
                colors.text_accent,
                colors.text_accent,
                Some(IconName::Minus),
            ),
            CheckState::Unchecked => (colors.element_background, colors.border, None),
        };

        let mut container = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(16.))
            .h(px(16.))
            .rounded(px(3.))
            .bg(bg)
            .border_1()
            .border_color(border);

        if !self.disabled {
            container = container
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.opacity(0.8));
        }

        if let Some(on_click) = self.on_click {
            if !self.disabled {
                container = container.on_click(on_click);
            }
        }

        if let Some(icon_name) = icon {
            container = container.child(Icon::new(icon_name).size(IconSize::XSmall).color(
                rgitui_theme::Color::Custom(gpui::Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: if cx.theme().appearance == rgitui_theme::Appearance::Dark {
                        0.1
                    } else {
                        1.0
                    },
                    a: 1.0,
                }),
            ));
        }

        container
    }
}
