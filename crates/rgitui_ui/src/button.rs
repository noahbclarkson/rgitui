use gpui::prelude::*;
use gpui::{div, App, ClickEvent, CursorStyle, ElementId, Hsla, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::{Icon, IconName, IconSize, Label, LabelSize};

/// Button visual styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonStyle {
    /// Solid background color.
    Filled,
    /// Default: transparent, shows bg on hover.
    #[default]
    Subtle,
    /// Border + semi-transparent background.
    Outlined,
    /// Semantic tinted button.
    Tinted(TintColor),
    /// Fully transparent, only text color changes.
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintColor {
    Accent,
    Error,
    Warning,
    Success,
}

/// Button sizes controlling height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonSize {
    Large,
    #[default]
    Default,
    Compact,
    None,
}

impl ButtonSize {
    pub fn height(&self) -> gpui::Rems {
        match self {
            ButtonSize::Large => gpui::rems(2.0),     // 32px
            ButtonSize::Default => gpui::rems(1.75),  // 28px
            ButtonSize::Compact => gpui::rems(1.375), // 22px
            ButtonSize::None => gpui::rems(1.0),      // 16px
        }
    }
}

/// A clickable button with optional icon and label.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    label_color: Option<Color>,
    icon: Option<IconName>,
    icon_position: IconPosition,
    style: ButtonStyle,
    size: ButtonSize,
    disabled: bool,
    selected: bool,
    full_width: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconPosition {
    #[default]
    Start,
    End,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            label_color: None,
            icon: None,
            icon_position: IconPosition::Start,
            style: ButtonStyle::default(),
            size: ButtonSize::default(),
            disabled: false,
            selected: false,
            full_width: false,
            on_click: None,
        }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_position(mut self, pos: IconPosition) -> Self {
        self.icon_position = pos;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.label_color = Some(color);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
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

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let label_color = if self.disabled {
            Color::Disabled
        } else if self.selected {
            Color::Accent
        } else {
            self.label_color.unwrap_or(Color::Default)
        };

        let (bg, hover_bg, active_bg, border) = match self.style {
            ButtonStyle::Filled => (
                Some(colors.element_background),
                Some(colors.element_hover),
                Some(colors.element_active),
                None,
            ),
            ButtonStyle::Subtle => (
                None,
                Some(colors.ghost_element_hover),
                Some(colors.ghost_element_active),
                None,
            ),
            ButtonStyle::Outlined => (
                Some(colors.element_background),
                Some(colors.element_hover),
                Some(colors.element_active),
                Some(colors.border),
            ),
            ButtonStyle::Tinted(tint) => {
                let status = cx.status();
                let (_fg_color, bg_color) = match tint {
                    TintColor::Accent => (status.info, status.info_background),
                    TintColor::Error => (status.error, status.error_background),
                    TintColor::Warning => (status.warning, status.warning_background),
                    TintColor::Success => (status.success, status.success_background),
                };
                (
                    Some(bg_color),
                    Some(Hsla {
                        a: bg_color.a + 0.1,
                        ..bg_color
                    }),
                    Some(Hsla {
                        a: bg_color.a + 0.2,
                        ..bg_color
                    }),
                    None,
                )
            }
            ButtonStyle::Transparent => (None, None, None, None),
        };

        let height = self.size.height();

        let mut container = div()
            .id(self.id)
            .h_flex()
            .h(height)
            .px_2()
            .gap_1()
            .rounded_md()
            .cursor(if self.disabled {
                CursorStyle::default()
            } else {
                CursorStyle::PointingHand
            });

        if self.full_width {
            container = container.w_full().justify_center();
        }

        if let Some(bg) = bg {
            container = container.bg(bg);
        }

        if let Some(border) = border {
            container = container.border_1().border_color(border);
        }

        if !self.disabled {
            if let Some(hover_bg) = hover_bg {
                container = container.hover(|s| s.bg(hover_bg));
            }
            if let Some(active_bg) = active_bg {
                container = container.active(|s| s.bg(active_bg));
            }
        }

        if let Some(on_click) = self.on_click {
            if !self.disabled {
                container = container.on_click(on_click);
            }
        }

        let icon_size = match self.size {
            ButtonSize::Large => IconSize::Medium,
            ButtonSize::Default => IconSize::Small,
            ButtonSize::Compact => IconSize::XSmall,
            ButtonSize::None => IconSize::XSmall,
        };

        let label_size = match self.size {
            ButtonSize::Large => LabelSize::Default,
            ButtonSize::Default => LabelSize::Small,
            ButtonSize::Compact => LabelSize::XSmall,
            ButtonSize::None => LabelSize::XSmall,
        };

        let icon_el = self
            .icon
            .map(|name| Icon::new(name).size(icon_size).color(label_color));

        let label_el = Label::new(self.label).size(label_size).color(label_color);

        match self.icon_position {
            IconPosition::Start => {
                container = container.children(icon_el).child(label_el);
            }
            IconPosition::End => {
                container = container.child(label_el).children(icon_el);
            }
        }

        container
    }
}

/// An icon-only button.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    color: Color,
    style: ButtonStyle,
    size: ButtonSize,
    disabled: bool,
    selected: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            id: id.into(),
            icon,
            color: Color::Default,
            style: ButtonStyle::Subtle,
            size: ButtonSize::Default,
            disabled: false,
            selected: false,
            on_click: None,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let icon_color = if self.disabled {
            Color::Disabled
        } else if self.selected {
            Color::Accent
        } else {
            self.color
        };

        let size = self.size.height();

        let mut container = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .rounded_md()
            .cursor(if self.disabled {
                CursorStyle::default()
            } else {
                CursorStyle::PointingHand
            });

        match self.style {
            ButtonStyle::Filled => {
                container = container.bg(colors.element_background);
            }
            ButtonStyle::Outlined => {
                container = container
                    .bg(colors.element_background)
                    .border_1()
                    .border_color(colors.border);
            }
            _ => {}
        }

        if !self.disabled {
            container = container
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active));
        }

        if let Some(on_click) = self.on_click {
            if !self.disabled {
                container = container.on_click(on_click);
            }
        }

        container.child(Icon::new(self.icon).size(IconSize::Small).color(icon_color))
    }
}
