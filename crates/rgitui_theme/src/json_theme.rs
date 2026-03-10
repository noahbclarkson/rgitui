use std::collections::HashMap;

use anyhow::{anyhow, Result};
use gpui::Hsla;
use serde::Deserialize;

use crate::colors::{hex_to_hsla, StatusColors, ThemeColors};
use crate::theme::{Appearance, Theme};

/// Raw JSON representation of a theme file.
#[derive(Debug, Deserialize)]
struct JsonTheme {
    name: String,
    appearance: String,
    colors: HashMap<String, String>,
    status: HashMap<String, String>,
}

/// Parse a hex color from the map, returning an error if missing.
fn get_color(map: &HashMap<String, String>, key: &str) -> Result<Hsla> {
    let hex = map
        .get(key)
        .ok_or_else(|| anyhow!("missing color key: {}", key))?;
    Ok(hex_to_hsla(hex))
}

/// Load a complete Theme from a JSON string.
pub fn load_theme_from_json(json: &str) -> Result<Theme> {
    let raw: JsonTheme = serde_json::from_str(json)?;

    let appearance = match raw.appearance.as_str() {
        "light" => Appearance::Light,
        "dark" => Appearance::Dark,
        other => return Err(anyhow!("unknown appearance: {}", other)),
    };

    let c = &raw.colors;
    let s = &raw.status;

    let colors = ThemeColors {
        background: get_color(c, "background")?,
        surface_background: get_color(c, "surface_background")?,
        elevated_surface_background: get_color(c, "elevated_surface_background")?,
        editor_background: get_color(c, "editor_background")?,

        border: get_color(c, "border")?,
        border_variant: get_color(c, "border_variant")?,
        border_focused: get_color(c, "border_focused")?,
        border_selected: get_color(c, "border_selected")?,
        border_disabled: get_color(c, "border_disabled")?,
        border_transparent: get_color(c, "border_transparent")?,

        element_background: get_color(c, "element_background")?,
        element_hover: get_color(c, "element_hover")?,
        element_active: get_color(c, "element_active")?,
        element_selected: get_color(c, "element_selected")?,
        element_disabled: get_color(c, "element_disabled")?,
        ghost_element_background: get_color(c, "ghost_element_background")?,
        ghost_element_hover: get_color(c, "ghost_element_hover")?,
        ghost_element_active: get_color(c, "ghost_element_active")?,
        ghost_element_selected: get_color(c, "ghost_element_selected")?,

        text: get_color(c, "text")?,
        text_muted: get_color(c, "text_muted")?,
        text_placeholder: get_color(c, "text_placeholder")?,
        text_disabled: get_color(c, "text_disabled")?,
        text_accent: get_color(c, "text_accent")?,

        icon: get_color(c, "icon")?,
        icon_muted: get_color(c, "icon_muted")?,
        icon_disabled: get_color(c, "icon_disabled")?,
        icon_accent: get_color(c, "icon_accent")?,

        title_bar_background: get_color(c, "title_bar_background")?,
        toolbar_background: get_color(c, "toolbar_background")?,
        tab_bar_background: get_color(c, "tab_bar_background")?,
        tab_active_background: get_color(c, "tab_active_background")?,
        tab_inactive_background: get_color(c, "tab_inactive_background")?,
        status_bar_background: get_color(c, "status_bar_background")?,
        panel_background: get_color(c, "panel_background")?,
        scrollbar_thumb_background: get_color(c, "scrollbar_thumb_background")?,
        scrollbar_thumb_hover_background: get_color(c, "scrollbar_thumb_hover_background")?,

        vc_added: get_color(c, "vc_added")?,
        vc_modified: get_color(c, "vc_modified")?,
        vc_deleted: get_color(c, "vc_deleted")?,
        vc_conflict: get_color(c, "vc_conflict")?,
        vc_renamed: get_color(c, "vc_renamed")?,
        vc_untracked: get_color(c, "vc_untracked")?,
    };

    let status = StatusColors {
        error: get_color(s, "error")?,
        error_background: get_color(s, "error_background")?,
        warning: get_color(s, "warning")?,
        warning_background: get_color(s, "warning_background")?,
        success: get_color(s, "success")?,
        success_background: get_color(s, "success_background")?,
        info: get_color(s, "info")?,
        info_background: get_color(s, "info_background")?,
        hint: get_color(s, "hint")?,
    };

    Ok(Theme {
        name: raw.name,
        appearance,
        colors,
        status,
    })
}

/// Embedded JSON theme files (compiled into the binary).
const BUILTIN_THEME_FILES: &[(&str, &str)] = &[
    (
        "catppuccin-mocha",
        include_str!("../../../assets/themes/catppuccin-mocha.json"),
    ),
    (
        "catppuccin-latte",
        include_str!("../../../assets/themes/catppuccin-latte.json"),
    ),
    (
        "one-dark",
        include_str!("../../../assets/themes/one-dark.json"),
    ),
    (
        "github-dark",
        include_str!("../../../assets/themes/github-dark.json"),
    ),
    (
        "dracula",
        include_str!("../../../assets/themes/dracula.json"),
    ),
];

/// Load all JSON-based built-in themes. Falls back to hardcoded themes on parse errors.
pub fn load_json_themes() -> Vec<Theme> {
    let mut themes = Vec::new();
    for (_slug, json) in BUILTIN_THEME_FILES {
        match load_theme_from_json(json) {
            Ok(theme) => themes.push(theme),
            Err(e) => {
                eprintln!("warning: failed to load built-in theme: {}", e);
            }
        }
    }
    themes
}

/// Return the names of all available built-in themes.
pub fn available_themes() -> Vec<String> {
    BUILTIN_THEME_FILES
        .iter()
        .filter_map(|(_slug, json)| load_theme_from_json(json).ok().map(|t| t.name))
        .collect()
}

/// Load a specific theme by name. Falls back to Catppuccin Mocha if not found.
pub fn load_theme_by_name(name: &str) -> Theme {
    for (_slug, json) in BUILTIN_THEME_FILES {
        if let Ok(theme) = load_theme_from_json(json) {
            if theme.name == name {
                return theme;
            }
        }
    }
    // Fallback: load first theme (Catppuccin Mocha)
    load_theme_from_json(BUILTIN_THEME_FILES[0].1).expect("default theme must parse")
}
