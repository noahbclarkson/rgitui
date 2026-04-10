//! Custom Theme Editor — create and edit custom color themes.
//!
//! Users edit hex color values and save themes to `~/.config/rgitui/themes/{name}.json`.

use gpui::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    div, px, App, ClickEvent, Context, ElementId, Entity, EventEmitter, Hsla, InteractiveElement,
    KeyDownEvent, ParentElement, Render, SharedString, Window,
};
use rgitui_theme::json_theme::load_theme_from_json;
use rgitui_theme::{hex_to_hsla, ActiveTheme, Appearance, Color, StyledExt, Theme, ThemeState};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput,
    TextInputEvent,
};

use crate::SettingsModalEvent;

// ── Color category definitions ────────────────────────────────────────────────

type ColorCategory = (
    &'static str,
    &'static str,
    Vec<(&'static str, &'static str)>,
);

const COLOR_CATEGORIES: &[ColorCategory] = &[
    (
        "backgrounds",
        "Backgrounds",
        &[
            ("background", "Background"),
            ("surface_background", "Surface"),
            ("elevated_surface_background", "Elevated Surface"),
            ("editor_background", "Editor"),
        ],
    ),
    (
        "borders",
        "Borders",
        &[
            ("border", "Border"),
            ("border_variant", "Border Variant"),
            ("border_focused", "Border Focused"),
            ("border_selected", "Border Selected"),
            ("border_disabled", "Border Disabled"),
            ("border_transparent", "Border Transparent"),
        ],
    ),
    (
        "elements",
        "Element States",
        &[
            ("element_background", "Element Background"),
            ("element_hover", "Element Hover"),
            ("element_active", "Element Active"),
            ("element_selected", "Element Selected"),
            ("element_disabled", "Element Disabled"),
            ("ghost_element_background", "Ghost Background"),
            ("ghost_element_hover", "Ghost Hover"),
            ("ghost_element_active", "Ghost Active"),
            ("ghost_element_selected", "Ghost Selected"),
        ],
    ),
    (
        "text",
        "Text",
        &[
            ("text", "Text"),
            ("text_muted", "Text Muted"),
            ("text_placeholder", "Placeholder"),
            ("text_disabled", "Text Disabled"),
            ("text_accent", "Text Accent"),
        ],
    ),
    (
        "icons",
        "Icons",
        &[
            ("icon", "Icon"),
            ("icon_muted", "Icon Muted"),
            ("icon_disabled", "Icon Disabled"),
            ("icon_accent", "Icon Accent"),
        ],
    ),
    (
        "ui_chrome",
        "UI Chrome",
        &[
            ("title_bar_background", "Title Bar"),
            ("toolbar_background", "Toolbar"),
            ("tab_bar_background", "Tab Bar"),
            ("tab_active_background", "Tab Active"),
            ("tab_inactive_background", "Tab Inactive"),
            ("status_bar_background", "Status Bar"),
            ("panel_background", "Panel"),
            ("scrollbar_thumb_background", "Scrollbar"),
            ("scrollbar_thumb_hover_background", "Scrollbar Hover"),
            ("hint_background", "Hint"),
        ],
    ),
    (
        "version_control",
        "Version Control",
        &[
            ("vc_added", "Added"),
            ("vc_modified", "Modified"),
            ("vc_deleted", "Deleted"),
            ("vc_conflict", "Conflict"),
            ("vc_renamed", "Renamed"),
            ("vc_untracked", "Untracked"),
        ],
    ),
    (
        "status",
        "Status",
        &[
            ("status_error", "Error"),
            ("status_error_bg", "Error Background"),
            ("status_warning", "Warning"),
            ("status_warning_bg", "Warning Background"),
            ("status_success", "Success"),
            ("status_success_bg", "Success Background"),
            ("status_info", "Info"),
            ("status_info_bg", "Info Background"),
            ("status_hint", "Hint"),
        ],
    ),
];

/// Maps editor color keys to JSON keys in the theme file.
fn json_key(color_key: &str) -> &str {
    match color_key {
        "status_error" => "error",
        "status_error_bg" => "error_background",
        "status_warning" => "warning",
        "status_warning_bg" => "warning_background",
        "status_success" => "success",
        "status_success_bg" => "success_background",
        "status_info" => "info",
        "status_info_bg" => "info_background",
        "status_hint" => "hint",
        k => k,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn hsla_to_hex(c: &Hsla) -> String {
    let r = (c.r * 255.0).round() as u8;
    let g = (c.g * 255.0).round() as u8;
    let b = (c.b * 255.0).round() as u8;
    if (c.a - 1.0).abs() < 0.01 {
        format!("{:02x}{:02x}{:02x}", r, g, b)
    } else {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            r,
            g,
            b,
            (c.a * 255.0).round() as u8
        )
    }
}

fn swatch_border(bg: Hsla) -> Hsla {
    Hsla {
        h: bg.h,
        s: bg.s,
        l: (bg.l - 0.15).max(0.0),
        a: bg.a,
    }
}

fn swatch_text(bg: Hsla) -> Hsla {
    if bg.l > 0.5 {
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 1.0,
        }
    } else {
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 1.0,
        }
    }
}

fn eid(s: &str) -> ElementId {
    ElementId::Name(SharedString::from(s.to_string()))
}

// ── CustomThemeEditor ────────────────────────────────────────────────────────

/// The custom theme editor component.
pub struct CustomThemeEditor {
    name_editor: Entity<TextInput>,
    appearance: Appearance,
    /// color_key → hex string (no # prefix)
    color_map: HashMap<String, String>,
    /// Per-color hex text input entities.
    hex_inputs: HashMap<String, Entity<TextInput>>,
    /// Collapsed category section keys.
    collapsed: HashMap<String, bool>,
    /// Error message to display (if any).
    error: Option<String>,
}

impl CustomThemeEditor {
    /// Create an editor pre-populated from `base` theme.
    pub fn from_theme(base: &Theme, cx: &mut Context<Self>) -> Self {
        let color_map = Self::extract(base);
        let name_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_text(format!("{} (Copy)", base.name), cx);
            ti
        });
        Self {
            name_editor,
            appearance: base.appearance,
            color_map,
            hex_inputs: HashMap::new(),
            collapsed: HashMap::new(),
            error: None,
        }
    }

    /// Create an editor for a new empty dark theme.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let color_map = Self::default_dark_map();
        let name_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_text("My Custom Theme", cx);
            ti
        });
        Self {
            name_editor,
            appearance: Appearance::Dark,
            color_map,
            hex_inputs: HashMap::new(),
            collapsed: HashMap::new(),
            error: None,
        }
    }

    fn default_dark_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        let s = |h: &str| h.to_string();
        m.insert("background".into(), s("#1e1e2e"));
        m.insert("surface_background".into(), s("#181825"));
        m.insert("elevated_surface_background".into(), s("#313244"));
        m.insert("editor_background".into(), s("#1e1e2e"));
        m.insert("border".into(), s("#45475a"));
        m.insert("border_variant".into(), s("#313244"));
        m.insert("border_focused".into(), s("#89b4fa"));
        m.insert("border_selected".into(), s("#89b4fa"));
        m.insert("border_disabled".into(), s("#313244"));
        m.insert("border_transparent".into(), s("#00000000"));
        m.insert("element_background".into(), s("#313244"));
        m.insert("element_hover".into(), s("#45475a"));
        m.insert("element_active".into(), s("#585b70"));
        m.insert("element_selected".into(), s("#45475a"));
        m.insert("element_disabled".into(), s("#313244"));
        m.insert("ghost_element_background".into(), s("#00000000"));
        m.insert("ghost_element_hover".into(), s("#31324480"));
        m.insert("ghost_element_active".into(), s("#45475a80"));
        m.insert("ghost_element_selected".into(), s("#45475ab3"));
        m.insert("text".into(), s("#cdd6f4"));
        m.insert("text_muted".into(), s("#a6adc8"));
        m.insert("text_placeholder".into(), s("#7f849c"));
        m.insert("text_disabled".into(), s("#6c7086"));
        m.insert("text_accent".into(), s("#89b4fa"));
        m.insert("icon".into(), s("#cdd6f4"));
        m.insert("icon_muted".into(), s("#a6adc8"));
        m.insert("icon_disabled".into(), s("#6c7086"));
        m.insert("icon_accent".into(), s("#89b4fa"));
        m.insert("title_bar_background".into(), s("#181825"));
        m.insert("toolbar_background".into(), s("#1e1e2e"));
        m.insert("tab_bar_background".into(), s("#181825"));
        m.insert("tab_active_background".into(), s("#1e1e2e"));
        m.insert("tab_inactive_background".into(), s("#181825"));
        m.insert("status_bar_background".into(), s("#181825"));
        m.insert("panel_background".into(), s("#1e1e2e"));
        m.insert("scrollbar_thumb_background".into(), s("#45475a"));
        m.insert("scrollbar_thumb_hover_background".into(), s("#585b70"));
        m.insert("hint_background".into(), s("#89b4fa1f"));
        m.insert("vc_added".into(), s("#a6e3a1"));
        m.insert("vc_modified".into(), s("#f9e2af"));
        m.insert("vc_deleted".into(), s("#f38ba8"));
        m.insert("vc_conflict".into(), s("#fab387"));
        m.insert("vc_renamed".into(), s("#89b4fa"));
        m.insert("vc_untracked".into(), s("#94e2d5"));
        m.insert("status_error".into(), s("#f38ba8"));
        m.insert("status_error_bg".into(), s("#f38ba826"));
        m.insert("status_warning".into(), s("#f9e2af"));
        m.insert("status_warning_bg".into(), s("#f9e2af26"));
        m.insert("status_success".into(), s("#a6e3a1"));
        m.insert("status_success_bg".into(), s("#a6e3a126"));
        m.insert("status_info".into(), s("#89b4fa"));
        m.insert("status_info_bg".into(), s("#89b4fa26"));
        m.insert("status_hint".into(), s("#94e2d5"));
        m
    }

    fn extract(theme: &Theme) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let c = &theme.colors;
        let put = |m: &mut HashMap<_, _>, k: &'static str, v: Hsla| {
            m.insert(k.to_string(), hsla_to_hex(&v));
        };
        put(&mut map, "background", c.background);
        put(&mut map, "surface_background", c.surface_background);
        put(
            &mut map,
            "elevated_surface_background",
            c.elevated_surface_background,
        );
        put(&mut map, "editor_background", c.editor_background);
        put(&mut map, "border", c.border);
        put(&mut map, "border_variant", c.border_variant);
        put(&mut map, "border_focused", c.border_focused);
        put(&mut map, "border_selected", c.border_selected);
        put(&mut map, "border_disabled", c.border_disabled);
        put(&mut map, "border_transparent", c.border_transparent);
        put(&mut map, "element_background", c.element_background);
        put(&mut map, "element_hover", c.element_hover);
        put(&mut map, "element_active", c.element_active);
        put(&mut map, "element_selected", c.element_selected);
        put(&mut map, "element_disabled", c.element_disabled);
        put(
            &mut map,
            "ghost_element_background",
            c.ghost_element_background,
        );
        put(&mut map, "ghost_element_hover", c.ghost_element_hover);
        put(&mut map, "ghost_element_active", c.ghost_element_active);
        put(&mut map, "ghost_element_selected", c.ghost_element_selected);
        put(&mut map, "text", c.text);
        put(&mut map, "text_muted", c.text_muted);
        put(&mut map, "text_placeholder", c.text_placeholder);
        put(&mut map, "text_disabled", c.text_disabled);
        put(&mut map, "text_accent", c.text_accent);
        put(&mut map, "icon", c.icon);
        put(&mut map, "icon_muted", c.icon_muted);
        put(&mut map, "icon_disabled", c.icon_disabled);
        put(&mut map, "icon_accent", c.icon_accent);
        put(&mut map, "title_bar_background", c.title_bar_background);
        put(&mut map, "toolbar_background", c.toolbar_background);
        put(&mut map, "tab_bar_background", c.tab_bar_background);
        put(&mut map, "tab_active_background", c.tab_active_background);
        put(
            &mut map,
            "tab_inactive_background",
            c.tab_inactive_background,
        );
        put(&mut map, "status_bar_background", c.status_bar_background);
        put(&mut map, "panel_background", c.panel_background);
        put(
            &mut map,
            "scrollbar_thumb_background",
            c.scrollbar_thumb_background,
        );
        put(
            &mut map,
            "scrollbar_thumb_hover_background",
            c.scrollbar_thumb_hover_background,
        );
        put(&mut map, "hint_background", c.hint_background);
        put(&mut map, "vc_added", c.vc_added);
        put(&mut map, "vc_modified", c.vc_modified);
        put(&mut map, "vc_deleted", c.vc_deleted);
        put(&mut map, "vc_conflict", c.vc_conflict);
        put(&mut map, "vc_renamed", c.vc_renamed);
        put(&mut map, "vc_untracked", c.vc_untracked);

        let s = &theme.status;
        put(&mut map, "status_error", s.error);
        put(&mut map, "status_error_bg", s.error_background);
        put(&mut map, "status_warning", s.warning);
        put(&mut map, "status_warning_bg", s.warning_background);
        put(&mut map, "status_success", s.success);
        put(&mut map, "status_success_bg", s.success_background);
        put(&mut map, "status_info", s.info);
        put(&mut map, "status_info_bg", s.info_background);
        put(&mut map, "status_hint", s.hint);
        map
    }

    fn get_or_create_hex_input(
        &mut self,
        color_key: &str,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(e) = self.hex_inputs.get(color_key) {
            return e.clone();
        }

        let hex = self.color_map.get(color_key).cloned().unwrap_or_default();
        let entity = cx.new(move |cx| {
            let mut ti = TextInput::new(cx);
            ti.set_text(format!("#{}", hex), cx);
            ti.set_placeholder("000000");
            ti
        });

        let key_clone = color_key.to_owned();
        let entity_for_sub = entity.clone();
        cx.subscribe(
            &entity_for_sub.clone(),
            move |this, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(raw) = event {
                    let cleaned = raw.trim().trim_start_matches('#').to_lowercase();
                    let valid = (cleaned.len() == 6 || cleaned.len() == 8)
                        && cleaned.chars().all(|c| c.is_ascii_hexdigit());
                    if valid || cleaned.is_empty() {
                        this.color_map.insert(key_clone.clone(), cleaned.clone());
                        entity_for_sub.update(cx, |ti, cx| {
                            ti.set_text(format!("#{}", cleaned), cx);
                        });
                        cx.notify();
                    }
                }
            },
        );

        self.hex_inputs.insert(color_key.to_owned(), entity.clone());
        entity
    }

    fn toggle_section(&mut self, key: &str) {
        let e = self.collapsed.entry(key.to_owned()).or_insert(false);
        *e = !*e;
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.error = None;

        let name = self.name_editor.read(cx).text().trim().to_string();
        if name.is_empty() {
            self.error = Some("Theme name cannot be empty.".to_string());
            cx.notify();
            return;
        }

        // Validate
        for (k, v) in &self.color_map {
            if !v.is_empty() && v.len() != 6 && v.len() != 8 {
                self.error = Some(format!("Invalid color '{}': '{}'", k, v));
                cx.notify();
                return;
            }
            if !v.is_empty() && !v.chars().all(|c| c.is_ascii_hexdigit()) {
                self.error = Some(format!("Non-hex chars in '{}': '{}'", k, v));
                cx.notify();
                return;
            }
        }

        // Build color + status maps for JSON
        let mut color_keys: Vec<(&str, &str)> = Vec::new();
        let mut status_keys: Vec<(&str, &str)> = Vec::new();
        let status_set = [
            "error",
            "error_background",
            "warning",
            "warning_background",
            "success",
            "success_background",
            "info",
            "info_background",
            "hint",
        ];

        for (_, _, keys) in COLOR_CATEGORIES {
            for (k, _) in *keys {
                let jk = json_key(k);
                if status_set.contains(&jk) {
                    status_keys.push((jk, *k));
                } else {
                    color_keys.push((jk, *k));
                }
            }
        }

        let colors_json = color_keys
            .iter()
            .enumerate()
            .map(|(i, (jk, k))| {
                let hex = self
                    .color_map
                    .get(*k)
                    .cloned()
                    .unwrap_or_else(|| "000000".to_string());
                let comma = if i + 1 < color_keys.len() { "," } else { "" };
                format!("      \"{jk}\": \"#{hex}\"{comma}")
            })
            .collect::<Vec<_>>()
            .join(",\n");

        let status_json = status_keys
            .iter()
            .enumerate()
            .map(|(i, (jk, k))| {
                let hex = self
                    .color_map
                    .get(*k)
                    .cloned()
                    .unwrap_or_else(|| "000000".to_string());
                let comma = if i + 1 < status_keys.len() { "," } else { "" };
                format!("      \"{jk}\": \"#{hex}\"{comma}")
            })
            .collect::<Vec<_>>()
            .join(",\n");

        let appearance_str = match self.appearance {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        };

        let json = format!(
            r#"{{
  "name": "{name}",
  "appearance": "{appearance}",
  "colors": {{
{colors}
  }},
  "status": {{
{status}
  }}
}}"#,
            name = name,
            appearance = appearance_str,
            colors = colors_json,
            status = status_json,
        );

        // Write to themes dir
        let themes_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rgitui")
            .join("themes");

        if let Err(e) = std::fs::create_dir_all(&themes_dir) {
            self.error = Some(format!("Failed to create themes directory: {}", e));
            cx.notify();
            return;
        }

        let filename = name
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
            .collect::<String>()
            .replace(' ', "-");
        let path = themes_dir.join(format!("{}.json", filename));

        if let Err(e) = std::fs::write(&path, &json) {
            self.error = Some(format!("Failed to write theme: {}", e));
            cx.notify();
            return;
        }

        // Reload and register
        match load_theme_from_json(&json) {
            Ok(theme) => {
                let theme_name = theme.name.clone();
                cx.update_global::<ThemeState, _>(|state, _cx| {
                    state.set_theme_by_name(&theme_name);
                    state.register_theme(theme);
                });
                cx.emit(SettingsModalEvent::ThemeChanged(theme_name));
            }
            Err(e) => {
                self.error = Some(format!(
                    "Saved to {} but failed to reload: {}",
                    path.display(),
                    e
                ));
                cx.notify();
            }
        }
    }
}

impl EventEmitter<SettingsModalEvent> for CustomThemeEditor {}

impl Render for CustomThemeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thm = cx.colors();
        let st = cx.status();
        let thm_error_bg = st.error_background.clone();
        let thm_border_variant = thm.border_variant.clone();
        let thm_surface_bg = thm.surface_background.clone();
        let thm_element_selected = thm.element_selected.clone();
        let thm_muted = Color::Muted;

        div()
            .id("custom-theme-editor")
            .v_flex()
            .w_full()
            .h_full()
            .gap(px(16.))
            // Header
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        Label::new("Custom Theme")
                            .size(LabelSize::Large)
                            .weight(gpui::FontWeight::BOLD),
                    )
                    .child(
                        // Dark / Light toggle
                        div()
                            .h_flex()
                            .gap(px(1.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(thm_border_variant.clone())
                            .p(px(1.))
                            .child(
                                div()
                                    .px(px(12.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(self.appearance == Appearance::Dark, |el| {
                                        el.bg(thm_element_selected.clone())
                                    })
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, _cx| {
                                        this.appearance = Appearance::Dark;
                                    }))
                                    .child(Label::new("Dark").size(LabelSize::XSmall)),
                            )
                            .child(
                                div()
                                    .px(px(12.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(self.appearance == Appearance::Light, |el| {
                                        el.bg(thm_element_selected.clone())
                                    })
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, _cx| {
                                        this.appearance = Appearance::Light;
                                    }))
                                    .child(Label::new("Light").size(LabelSize::XSmall)),
                            ),
                    ),
            )
            // Name + actions
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(12.))
                    .child(Label::new("Name:").size(LabelSize::Small).color(thm_muted))
                    .child(self.name_editor.clone())
                    .child(div())
                    .child(
                        Button::new("cancel-editor", "Cancel")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .icon(IconName::X)
                            .on_click(cx.listener(|_, _: &gpui::ClickEvent, _, _cx| {})),
                    )
                    .child(
                        Button::new("save-editor", "Save")
                            .style(ButtonStyle::Primary)
                            .size(ButtonSize::Compact)
                            .icon(IconName::Check)
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.save(cx);
                            })),
                    ),
            )
            // Error
            .when(self.error.is_some(), |el| {
                el.child(
                    div()
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(thm_error_bg.clone())
                        .child(
                            Label::new(self.error.as_deref().unwrap_or(""))
                                .size(LabelSize::XSmall)
                                .color(Color::Error),
                        ),
                )
            })
            // Body
            .child(
                div()
                    .h_flex()
                    .gap(px(16.))
                    .overflow_hidden()
                    // Left: sections
                    .child(
                        div()
                            .overflow_y_scroll()
                            .v_flex()
                            .gap(px(8.))
                            .child(self.render_sections(cx)),
                    )
                    // Right: preview
                    .child(
                        div()
                            .w(px(220.))
                            .overflow_y_scroll()
                            .v_flex()
                            .gap(px(12.))
                            .p(px(12.))
                            .rounded(px(8.))
                            .bg(thm_surface_bg.clone())
                            .border_1()
                            .border_color(thm_border_variant.clone())
                            .child(
                                Label::new("Preview")
                                    .size(LabelSize::XSmall)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .color(thm_muted),
                            )
                            .child(self.render_preview(cx)),
                    ),
            )
    }
}

impl CustomThemeEditor {
    fn render_sections(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut v = div().v_flex().gap(px(8.));
        let thm = cx.colors();
        let thm_border_variant = thm.border_variant.clone();
        let thm_surface_bg = thm.surface_background.clone();
        let thm_muted = Color::Muted;

        for (section_key, section_label, color_keys) in COLOR_CATEGORIES {
            let is_collapsed = *self.collapsed.get(*section_key).unwrap_or(&false);

            v = v.child(
                div()
                    .id(eid(&format!("section-{}", section_key)))
                    .h_flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(thm_surface_bg.clone()))
                    .child(
                        Icon::new(if is_collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(IconSize::XSmall)
                        .color(thm_muted),
                    )
                    .child(
                        Label::new(*section_label)
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(thm_muted),
                    )
                    .on_click({
                        let key = (*section_key).to_string();
                        move |this, _, _, cx| {
                            this.toggle_section(&key);
                            cx.notify();
                        }
                    }),
            );

            if !is_collapsed {
                let mut rows = div().v_flex().gap(px(4.)).pl(px(24.));
                for (color_key, color_label) in *color_keys {
                    let hex = self.color_map.get(*color_key).cloned().unwrap_or_default();
                    let swatch_bg = hex_to_hsla(&hex);

                    rows = rows.child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap(px(12.))
                            .py(px(1.))
                            .child(
                                div()
                                    .w(px(14.))
                                    .h(px(14.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(swatch_border(swatch_bg))
                                    .bg(swatch_bg),
                            )
                            .child(Label::new(*color_label).size(LabelSize::XSmall))
                            .child(self.get_or_create_hex_input(color_key, cx)),
                    );
                }
                v = v.child(rows);
            }
        }

        v
    }

    fn render_preview(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut v = div().v_flex().gap(px(12.));

        for (_, section_label, color_keys) in COLOR_CATEGORIES {
            v = v.child(
                Label::new(*section_label)
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

            let mut row = div().h_flex().flex_wrap().gap(px(4.));
            for (color_key, _) in *color_keys {
                let hex = self.color_map.get(*color_key).cloned().unwrap_or_default();
                let swatch_bg = hex_to_hsla(&hex);
                let jk = json_key(color_key);

                row = row.child(
                    div()
                        .id(eid(&format!("sw-{}", color_key)))
                        .w(px(20.))
                        .h(px(20.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(swatch_border(swatch_bg))
                        .bg(swatch_bg)
                        .tooltip(div().child(
                            Label::new(format!("{}: #{}", jk, hex)).size(LabelSize::XSmall),
                        )),
                );
            }
            v = v.child(row);
        }

        v
    }
}
