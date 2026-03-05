use std::sync::Arc;

use gpui::{App, Global, Hsla, Styled, px, point, BoxShadow};

use crate::colors::{
    ThemeColors, StatusColors,
    catppuccin_mocha_colors, catppuccin_mocha_status,
    catppuccin_latte_colors, catppuccin_latte_status,
    one_dark_colors, one_dark_status,
};

/// Visual appearance mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// A complete theme definition.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub colors: ThemeColors,
    pub status: StatusColors,
}

/// Global theme state stored in GPUI's global system.
pub struct ThemeState {
    active: Arc<Theme>,
    available: Vec<Arc<Theme>>,
}

impl Global for ThemeState {}

impl ThemeState {
    pub fn theme(&self) -> &Arc<Theme> {
        &self.active
    }

    pub fn available_themes(&self) -> &[Arc<Theme>] {
        &self.available
    }

    pub fn set_theme_by_name(&mut self, name: &str) {
        if let Some(theme) = self.available.iter().find(|t| t.name == name) {
            self.active = theme.clone();
        }
    }
}

/// Convenience trait to access the theme from App context.
pub trait ActiveTheme {
    fn theme(&self) -> &Arc<Theme>;
    fn colors(&self) -> &ThemeColors;
    fn status(&self) -> &StatusColors;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Arc<Theme> {
        self.global::<ThemeState>().theme()
    }

    fn colors(&self) -> &ThemeColors {
        &self.global::<ThemeState>().theme().colors
    }

    fn status(&self) -> &StatusColors {
        &self.global::<ThemeState>().theme().status
    }
}

/// Build all built-in themes.
pub fn builtin_themes() -> Vec<Arc<Theme>> {
    vec![
        Arc::new(Theme {
            name: "Catppuccin Mocha".into(),
            appearance: Appearance::Dark,
            colors: catppuccin_mocha_colors(),
            status: catppuccin_mocha_status(),
        }),
        Arc::new(Theme {
            name: "Catppuccin Latte".into(),
            appearance: Appearance::Light,
            colors: catppuccin_latte_colors(),
            status: catppuccin_latte_status(),
        }),
        Arc::new(Theme {
            name: "One Dark".into(),
            appearance: Appearance::Dark,
            colors: one_dark_colors(),
            status: one_dark_status(),
        }),
    ]
}

/// Initialize the theme system. Must be called during app init.
pub fn init(cx: &mut App) {
    let themes = builtin_themes();
    let active = themes[0].clone(); // Catppuccin Mocha default
    cx.set_global(ThemeState {
        active,
        available: themes,
    });
}

// -- Elevation helpers --

/// Elevation levels for layered surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationIndex {
    Background,
    Surface,
    ElevatedSurface,
    ModalSurface,
}

impl ElevationIndex {
    pub fn bg(&self, cx: &App) -> Hsla {
        let colors = cx.colors();
        match self {
            ElevationIndex::Background => colors.background,
            ElevationIndex::Surface => colors.surface_background,
            ElevationIndex::ElevatedSurface => colors.elevated_surface_background,
            ElevationIndex::ModalSurface => colors.elevated_surface_background,
        }
    }

    pub fn shadow(&self, cx: &App) -> Vec<BoxShadow> {
        let is_dark = cx.theme().appearance == Appearance::Dark;
        match self {
            ElevationIndex::Background | ElevationIndex::Surface => vec![],
            ElevationIndex::ElevatedSurface => vec![
                BoxShadow {
                    color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.12 },
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(3.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 0.0,
                        a: if is_dark { 0.08 } else { 0.04 },
                    },
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.),
                },
            ],
            ElevationIndex::ModalSurface => vec![
                BoxShadow {
                    color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.2 },
                    offset: point(px(0.), px(8.)),
                    blur_radius: px(16.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.12 },
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(4.),
                    spread_radius: px(0.),
                },
            ],
        }
    }
}

/// Extension trait providing Zed-like styling helpers on any Styled element.
pub trait StyledExt: Styled + Sized {
    /// Horizontal flex layout with centered items.
    fn h_flex(self) -> Self {
        self.flex().flex_row().items_center()
    }

    /// Vertical flex layout.
    fn v_flex(self) -> Self {
        self.flex().flex_col()
    }

    /// Surface elevation — panels, tab bars.
    fn elevation_1(self, cx: &App) -> Self {
        let colors = cx.colors();
        self.bg(colors.surface_background)
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
    }

    /// Elevated surface — dropdowns, menus, floating panels.
    fn elevation_2(self, cx: &App) -> Self {
        let colors = cx.colors();
        let shadows = ElevationIndex::ElevatedSurface.shadow(cx);
        self.bg(colors.elevated_surface_background)
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .shadow(shadows)
    }

    /// Modal surface — dialogs, modals.
    fn elevation_3(self, cx: &App) -> Self {
        let colors = cx.colors();
        let shadows = ElevationIndex::ModalSurface.shadow(cx);
        self.bg(colors.elevated_surface_background)
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .shadow(shadows)
    }
}

impl<E: Styled> StyledExt for E {}
