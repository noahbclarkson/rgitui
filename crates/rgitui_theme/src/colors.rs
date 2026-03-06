use gpui::Hsla;
use serde::{Deserialize, Serialize};

/// Semantic color palette for the entire application.
/// Modeled after Zed's ThemeColors but focused on Git GUI needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeColors {
    // -- Backgrounds --
    pub background: Hsla,
    pub surface_background: Hsla,
    pub elevated_surface_background: Hsla,
    pub editor_background: Hsla,

    // -- Borders --
    pub border: Hsla,
    pub border_variant: Hsla,
    pub border_focused: Hsla,
    pub border_selected: Hsla,
    pub border_disabled: Hsla,
    pub border_transparent: Hsla,

    // -- Element states --
    pub element_background: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub element_selected: Hsla,
    pub element_disabled: Hsla,
    pub ghost_element_background: Hsla,
    pub ghost_element_hover: Hsla,
    pub ghost_element_active: Hsla,
    pub ghost_element_selected: Hsla,

    // -- Text --
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    pub text_accent: Hsla,

    // -- Icons --
    pub icon: Hsla,
    pub icon_muted: Hsla,
    pub icon_disabled: Hsla,
    pub icon_accent: Hsla,

    // -- UI chrome --
    pub title_bar_background: Hsla,
    pub toolbar_background: Hsla,
    pub tab_bar_background: Hsla,
    pub tab_active_background: Hsla,
    pub tab_inactive_background: Hsla,
    pub status_bar_background: Hsla,
    pub panel_background: Hsla,
    pub scrollbar_thumb_background: Hsla,
    pub scrollbar_thumb_hover_background: Hsla,

    // -- Version control (our specialty!) --
    pub vc_added: Hsla,
    pub vc_modified: Hsla,
    pub vc_deleted: Hsla,
    pub vc_conflict: Hsla,
    pub vc_renamed: Hsla,
    pub vc_untracked: Hsla,
}

/// Status colors for feedback states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusColors {
    pub error: Hsla,
    pub error_background: Hsla,
    pub warning: Hsla,
    pub warning_background: Hsla,
    pub success: Hsla,
    pub success_background: Hsla,
    pub info: Hsla,
    pub info_background: Hsla,
    pub hint: Hsla,
}

/// Semantic color enum used by UI components.
/// Maps intent to actual theme colors at render time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Color {
    #[default]
    Default,
    Muted,
    Accent,
    Disabled,
    Placeholder,
    Error,
    Warning,
    Success,
    Info,
    Added,
    Modified,
    Deleted,
    Conflict,
    Renamed,
    Untracked,
    Custom(Hsla),
}

impl Color {
    /// Resolve this semantic color to an actual HSLA value using the current theme.
    pub fn color(&self, cx: &gpui::App) -> Hsla {
        let theme = cx.global::<crate::ThemeState>().theme();
        match self {
            Color::Default => theme.colors.text,
            Color::Muted => theme.colors.text_muted,
            Color::Accent => theme.colors.text_accent,
            Color::Disabled => theme.colors.text_disabled,
            Color::Placeholder => theme.colors.text_placeholder,
            Color::Error => theme.status.error,
            Color::Warning => theme.status.warning,
            Color::Success => theme.status.success,
            Color::Info => theme.status.info,
            Color::Added => theme.colors.vc_added,
            Color::Modified => theme.colors.vc_modified,
            Color::Deleted => theme.colors.vc_deleted,
            Color::Conflict => theme.colors.vc_conflict,
            Color::Renamed => theme.colors.vc_renamed,
            Color::Untracked => theme.colors.vc_untracked,
            Color::Custom(c) => *c,
        }
    }

    /// Resolve to an icon color (same mapping but uses icon slots where appropriate).
    pub fn icon_color(&self, cx: &gpui::App) -> Hsla {
        let theme = cx.global::<crate::ThemeState>().theme();
        match self {
            Color::Default => theme.colors.icon,
            Color::Muted => theme.colors.icon_muted,
            Color::Disabled => theme.colors.icon_disabled,
            Color::Accent => theme.colors.icon_accent,
            _ => self.color(cx),
        }
    }
}

impl From<Hsla> for Color {
    fn from(color: Hsla) -> Self {
        Color::Custom(color)
    }
}

// -- Catppuccin Mocha color definitions --

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        h: h / 360.0,
        s: s / 100.0,
        l: l / 100.0,
        a,
    }
}

/// Catppuccin Mocha dark theme colors.
pub fn catppuccin_mocha_colors() -> ThemeColors {
    // Catppuccin Mocha palette
    let base = hsla(240.0, 21.0, 15.0, 1.0);        // #1e1e2e
    let mantle = hsla(240.0, 21.0, 12.0, 1.0);      // #181825
    let crust = hsla(240.0, 23.0, 9.0, 1.0);        // #11111b
    let surface0 = hsla(237.0, 16.0, 23.0, 1.0);    // #313244
    let surface1 = hsla(234.0, 13.0, 31.0, 1.0);    // #45475a
    let surface2 = hsla(233.0, 12.0, 39.0, 1.0);    // #585b70
    let overlay0 = hsla(231.0, 11.0, 47.0, 1.0);    // #6c7086
    let overlay1 = hsla(230.0, 13.0, 55.0, 1.0);    // #7f849c
    let subtext0 = hsla(228.0, 17.0, 64.0, 1.0);    // #a6adc8
    let text = hsla(227.0, 35.0, 80.0, 1.0);        // #cdd6f4
    let mauve = hsla(267.0, 84.0, 81.0, 1.0);       // #cba6f7 (accent)
    let red = hsla(343.0, 81.0, 75.0, 1.0);         // #f38ba8
    let green = hsla(115.0, 54.0, 76.0, 1.0);       // #a6e3a1
    let yellow = hsla(41.0, 86.0, 83.0, 1.0);       // #f9e2af
    let blue = hsla(217.0, 92.0, 76.0, 1.0);        // #89b4fa
    let peach = hsla(23.0, 92.0, 75.0, 1.0);        // #fab387
    let teal = hsla(170.0, 57.0, 73.0, 1.0);        // #94e2d5
    let _sapphire = hsla(189.0, 71.0, 73.0, 1.0);    // #74c7ec

    ThemeColors {
        background: crust,
        surface_background: mantle,
        elevated_surface_background: surface0,
        editor_background: base,

        border: surface1,
        border_variant: surface0,
        border_focused: mauve,
        border_selected: mauve,
        border_disabled: surface0,
        border_transparent: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },

        element_background: surface0,
        element_hover: surface1,
        element_active: surface2,
        element_selected: surface1,
        element_disabled: surface0,
        ghost_element_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        ghost_element_hover: hsla(237.0, 16.0, 23.0, 0.5),
        ghost_element_active: hsla(234.0, 13.0, 31.0, 0.5),
        ghost_element_selected: hsla(234.0, 13.0, 31.0, 0.7),

        text,
        text_muted: subtext0,
        text_placeholder: overlay1,
        text_disabled: overlay0,
        text_accent: mauve,

        icon: text,
        icon_muted: subtext0,
        icon_disabled: overlay0,
        icon_accent: mauve,

        title_bar_background: mantle,
        toolbar_background: base,
        tab_bar_background: mantle,
        tab_active_background: base,
        tab_inactive_background: mantle,
        status_bar_background: mantle,
        panel_background: base,
        scrollbar_thumb_background: surface1,
        scrollbar_thumb_hover_background: surface2,

        vc_added: green,
        vc_modified: yellow,
        vc_deleted: red,
        vc_conflict: peach,
        vc_renamed: blue,
        vc_untracked: teal,
    }
}

pub fn catppuccin_mocha_status() -> StatusColors {
    let red = hsla(343.0, 81.0, 75.0, 1.0);
    let yellow = hsla(41.0, 86.0, 83.0, 1.0);
    let green = hsla(115.0, 54.0, 76.0, 1.0);
    let blue = hsla(217.0, 92.0, 76.0, 1.0);
    let teal = hsla(170.0, 57.0, 73.0, 1.0);

    StatusColors {
        error: red,
        error_background: hsla(343.0, 81.0, 75.0, 0.15),
        warning: yellow,
        warning_background: hsla(41.0, 86.0, 83.0, 0.15),
        success: green,
        success_background: hsla(115.0, 54.0, 76.0, 0.15),
        info: blue,
        info_background: hsla(217.0, 92.0, 76.0, 0.15),
        hint: teal,
    }
}

/// Catppuccin Latte light theme colors.
pub fn catppuccin_latte_colors() -> ThemeColors {
    let base = hsla(220.0, 23.0, 95.0, 1.0);        // #eff1f5
    let mantle = hsla(220.0, 22.0, 92.0, 1.0);      // #e6e9ef
    let crust = hsla(220.0, 21.0, 89.0, 1.0);       // #dce0e8
    let surface0 = hsla(223.0, 16.0, 83.0, 1.0);    // #ccd0da
    let surface1 = hsla(225.0, 14.0, 77.0, 1.0);    // #bcc0cc
    let surface2 = hsla(227.0, 12.0, 71.0, 1.0);    // #acb0be
    let overlay0 = hsla(228.0, 11.0, 65.0, 1.0);    // #9ca0b0
    let overlay1 = hsla(227.0, 12.0, 58.0, 1.0);    // #8c8fa1
    let subtext0 = hsla(228.0, 14.0, 45.0, 1.0);    // #6c6f85
    let text = hsla(234.0, 16.0, 35.0, 1.0);        // #4c4f69
    let mauve = hsla(266.0, 85.0, 58.0, 1.0);       // #8839ef
    let red = hsla(347.0, 87.0, 44.0, 1.0);         // #d20f39
    let green = hsla(109.0, 58.0, 40.0, 1.0);       // #40a02b
    let yellow = hsla(35.0, 77.0, 49.0, 1.0);       // #df8e1d
    let blue = hsla(220.0, 91.0, 54.0, 1.0);        // #1e66f5
    let peach = hsla(22.0, 99.0, 52.0, 1.0);        // #fe640b
    let teal = hsla(183.0, 74.0, 35.0, 1.0);        // #179299
    let _sapphire = hsla(189.0, 70.0, 42.0, 1.0);    // #209fb5

    ThemeColors {
        background: crust,
        surface_background: base,
        elevated_surface_background: mantle,
        editor_background: base,

        border: surface1,
        border_variant: surface0,
        border_focused: mauve,
        border_selected: mauve,
        border_disabled: surface0,
        border_transparent: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },

        element_background: surface0,
        element_hover: surface1,
        element_active: surface2,
        element_selected: surface1,
        element_disabled: surface0,
        ghost_element_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        ghost_element_hover: hsla(223.0, 16.0, 83.0, 0.5),
        ghost_element_active: hsla(225.0, 14.0, 77.0, 0.5),
        ghost_element_selected: hsla(225.0, 14.0, 77.0, 0.7),

        text,
        text_muted: subtext0,
        text_placeholder: overlay1,
        text_disabled: overlay0,
        text_accent: mauve,

        icon: text,
        icon_muted: subtext0,
        icon_disabled: overlay0,
        icon_accent: mauve,

        title_bar_background: mantle,
        toolbar_background: base,
        tab_bar_background: mantle,
        tab_active_background: base,
        tab_inactive_background: mantle,
        status_bar_background: mantle,
        panel_background: base,
        scrollbar_thumb_background: surface1,
        scrollbar_thumb_hover_background: surface2,

        vc_added: green,
        vc_modified: yellow,
        vc_deleted: red,
        vc_conflict: peach,
        vc_renamed: blue,
        vc_untracked: teal,
    }
}

pub fn catppuccin_latte_status() -> StatusColors {
    let red = hsla(347.0, 87.0, 44.0, 1.0);
    let yellow = hsla(35.0, 77.0, 49.0, 1.0);
    let green = hsla(109.0, 58.0, 40.0, 1.0);
    let blue = hsla(220.0, 91.0, 54.0, 1.0);
    let teal = hsla(183.0, 74.0, 35.0, 1.0);

    StatusColors {
        error: red,
        error_background: hsla(347.0, 87.0, 44.0, 0.1),
        warning: yellow,
        warning_background: hsla(35.0, 77.0, 49.0, 0.1),
        success: green,
        success_background: hsla(109.0, 58.0, 40.0, 0.1),
        info: blue,
        info_background: hsla(220.0, 91.0, 54.0, 0.1),
        hint: teal,
    }
}

// -- One Dark theme colors --

/// One Dark (Atom) theme colors — GitKraken-like dark theme.
pub fn one_dark_colors() -> ThemeColors {
    let bg = hsla(220.0, 13.0, 18.0, 1.0);          // #282c34
    let bg_lighter = hsla(220.0, 13.0, 21.0, 1.0);   // #2c313a
    let gutter = hsla(220.0, 13.0, 24.0, 1.0);        // #353b45
    let guide = hsla(220.0, 10.0, 30.0, 1.0);         // #3b4048
    let text = hsla(219.0, 14.0, 71.0, 1.0);          // #abb2bf
    let text_muted = hsla(220.0, 9.0, 55.0, 1.0);     // #7f848e
    let text_dim = hsla(220.0, 9.0, 44.0, 1.0);       // #636d83
    let accent = hsla(207.0, 82.0, 66.0, 1.0);        // #61afef (blue)
    let red = hsla(355.0, 65.0, 65.0, 1.0);           // #e06c75
    let green = hsla(95.0, 38.0, 62.0, 1.0);          // #98c379
    let yellow = hsla(39.0, 67.0, 69.0, 1.0);         // #e5c07b
    let _blue = hsla(207.0, 82.0, 66.0, 1.0);         // #61afef (same as accent)
    let magenta = hsla(286.0, 60.0, 67.0, 1.0);       // #c678dd
    let cyan = hsla(187.0, 47.0, 55.0, 1.0);          // #56b6c2
    let orange = hsla(29.0, 54.0, 61.0, 1.0);         // #d19a66

    ThemeColors {
        background: hsla(220.0, 13.0, 15.0, 1.0),
        surface_background: bg,
        elevated_surface_background: bg_lighter,
        editor_background: bg,

        border: guide,
        border_variant: gutter,
        border_focused: accent,
        border_selected: accent,
        border_disabled: gutter,
        border_transparent: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },

        element_background: gutter,
        element_hover: guide,
        element_active: hsla(220.0, 10.0, 35.0, 1.0),
        element_selected: guide,
        element_disabled: gutter,
        ghost_element_background: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        ghost_element_hover: hsla(220.0, 13.0, 24.0, 0.5),
        ghost_element_active: hsla(220.0, 10.0, 30.0, 0.5),
        ghost_element_selected: hsla(220.0, 10.0, 30.0, 0.7),

        text,
        text_muted,
        text_placeholder: text_dim,
        text_disabled: text_dim,
        text_accent: accent,

        icon: text,
        icon_muted: text_muted,
        icon_disabled: text_dim,
        icon_accent: accent,

        title_bar_background: hsla(220.0, 13.0, 15.0, 1.0),
        toolbar_background: bg,
        tab_bar_background: hsla(220.0, 13.0, 15.0, 1.0),
        tab_active_background: bg,
        tab_inactive_background: hsla(220.0, 13.0, 15.0, 1.0),
        status_bar_background: hsla(220.0, 13.0, 15.0, 1.0),
        panel_background: bg,
        scrollbar_thumb_background: guide,
        scrollbar_thumb_hover_background: hsla(220.0, 10.0, 35.0, 1.0),

        vc_added: green,
        vc_modified: yellow,
        vc_deleted: red,
        vc_conflict: orange,
        vc_renamed: cyan,
        vc_untracked: magenta,
    }
}

pub fn one_dark_status() -> StatusColors {
    let red = hsla(355.0, 65.0, 65.0, 1.0);
    let yellow = hsla(39.0, 67.0, 69.0, 1.0);
    let green = hsla(95.0, 38.0, 62.0, 1.0);
    let blue = hsla(207.0, 82.0, 66.0, 1.0);
    let cyan = hsla(187.0, 47.0, 55.0, 1.0);

    StatusColors {
        error: red,
        error_background: hsla(355.0, 65.0, 65.0, 0.15),
        warning: yellow,
        warning_background: hsla(39.0, 67.0, 69.0, 0.15),
        success: green,
        success_background: hsla(95.0, 38.0, 62.0, 0.15),
        info: blue,
        info_background: hsla(207.0, 82.0, 66.0, 0.15),
        hint: cyan,
    }
}

/// Lane colors for the commit graph. Cycles through these for branch visualization.
pub const GRAPH_LANE_COLORS: &[fn() -> Hsla] = &[
    || hsla(267.0, 84.0, 75.0, 1.0),  // mauve (boosted)
    || hsla(217.0, 92.0, 65.0, 1.0),  // blue (boosted)
    || hsla(115.0, 60.0, 65.0, 1.0),  // green (boosted)
    || hsla(23.0, 92.0, 65.0, 1.0),   // peach (boosted)
    || hsla(343.0, 81.0, 65.0, 1.0),  // red (boosted)
    || hsla(170.0, 65.0, 60.0, 1.0),  // teal (boosted)
    || hsla(41.0, 86.0, 70.0, 1.0),   // yellow (boosted)
    || hsla(189.0, 75.0, 60.0, 1.0),  // sapphire (boosted)
    || hsla(316.0, 72.0, 72.0, 1.0),  // pink (boosted)
    || hsla(10.0, 70.0, 75.0, 1.0),   // rosewater (boosted)
];

pub fn lane_color(index: usize) -> Hsla {
    GRAPH_LANE_COLORS[index % GRAPH_LANE_COLORS.len()]()
}
