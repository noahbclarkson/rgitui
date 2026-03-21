use gpui::prelude::*;
use gpui::{div, px, App, ElementId, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::{ClickHandler, Icon, IconName, IconSize, Label, LabelSize};

/// A reusable disclosure (collapsible section header) component.
///
/// Shows a chevron icon indicating expanded/collapsed state, a label,
/// and triggers a callback when clicked. The parent manages the open/closed state.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    is_open: bool,
    label: SharedString,
    on_toggle: Option<ClickHandler>,
}

impl Disclosure {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, is_open: bool) -> Self {
        Self {
            id: id.into(),
            is_open,
            label: label.into(),
            on_toggle: None,
        }
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Disclosure {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;

        let chevron = if self.is_open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let mut row = div()
            .id(self.id)
            .h_flex()
            .gap(px(4.))
            .items_center()
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .active(move |s| s.bg(active_bg))
            .child(
                Icon::new(chevron)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new(self.label)
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

        if let Some(handler) = self.on_toggle {
            row = row.on_click(move |event, window, cx| handler(event, window, cx));
        }

        row
    }
}
