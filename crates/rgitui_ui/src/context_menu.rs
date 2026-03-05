use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{ActiveTheme, StyledExt};

use crate::{Label, LabelSize};

/// A single item in a context menu.
pub struct ContextMenuItem {
    label: SharedString,
    disabled: bool,
    is_separator: bool,
    shortcut: Option<SharedString>,
}

impl ContextMenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            disabled: false,
            is_separator: false,
            shortcut: None,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: "".into(),
            disabled: false,
            is_separator: true,
            shortcut: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

/// A context menu that shows a list of actions.
#[derive(IntoElement)]
pub struct ContextMenu {
    items: Vec<ContextMenuItem>,
    min_width: f32,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            min_width: 180.0,
        }
    }

    pub fn item(mut self, item: ContextMenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(ContextMenuItem::separator());
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        let mut menu = div()
            .v_flex()
            .min_w(px(self.min_width))
            .py(px(4.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .elevation_3(cx);

        for (i, item) in self.items.into_iter().enumerate() {
            if item.is_separator {
                menu = menu.child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .my(px(4.))
                        .bg(colors.border_variant),
                );
                continue;
            }

            let label = item.label.clone();
            let disabled = item.disabled;

            let mut row = div()
                .id(gpui::ElementId::NamedInteger("ctx-item".into(), i as u64))
                .h_flex()
                .w_full()
                .h(px(28.))
                .px_3()
                .gap_2()
                .items_center();

            if !disabled {
                row = row
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .cursor_pointer();
            }

            let text_color = if disabled {
                crate::Color::Disabled
            } else {
                crate::Color::Default
            };

            row = row.child(
                Label::new(label)
                    .size(LabelSize::Small)
                    .color(text_color),
            );

            if let Some(shortcut) = item.shortcut {
                row = row
                    .child(div().flex_1())
                    .child(
                        Label::new(shortcut)
                            .size(LabelSize::XSmall)
                            .color(crate::Color::Muted),
                    );
            }

            menu = menu.child(row);
        }

        menu
    }
}
