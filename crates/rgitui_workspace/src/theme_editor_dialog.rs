use gpui::prelude::*;
use gpui::{
    div, px, relative, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Window,
};
use rgitui_theme::{
    hex_to_hsla_strict, hsla_to_hex, json_theme::save_theme_to_file, ActiveTheme, Appearance,
    Color, StyledExt, Theme, ThemeState,
};
use rgitui_theme::{StatusColors, ThemeColors};
use rgitui_ui::{Button, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ThemeEditorEvent {
    Saved(Arc<Theme>),
    Closed,
}

struct ColorFieldEntry {
    label: String,
    getter: fn(&ThemeColors) -> Hsla,
    setter: fn(&mut ThemeColors, Hsla),
}

type ColorFieldSpec<'a> = (
    &'a str,
    fn(&ThemeColors) -> Hsla,
    fn(&mut ThemeColors, Hsla),
);

struct StatusFieldEntry {
    label: String,
    getter: fn(&StatusColors) -> Hsla,
    setter: fn(&mut StatusColors, Hsla),
}

type StatusFieldSpec<'a> = (
    &'a str,
    fn(&StatusColors) -> Hsla,
    fn(&mut StatusColors, Hsla),
);

pub struct ThemeEditorDialog {
    visible: bool,
    editable_theme: Arc<Theme>,
    focus_handle: FocusHandle,
    color_fields: Vec<ColorFieldEntry>,
    status_fields: Vec<StatusFieldEntry>,
    color_inputs: Vec<Entity<TextInput>>,
    status_inputs: Vec<Entity<TextInput>>,
    save_status: Option<SaveStatus>,
    invalid_color_fields: Vec<usize>,
    invalid_status_fields: Vec<usize>,
    needs_focus: bool,
}

enum SaveStatus {
    Saved {
        message: String,
        invalid_count: usize,
    },
    Failed(String),
}

impl ThemeEditorDialog {
    pub fn new_for_active_theme(cx: &mut Context<Self>) -> Self {
        let theme = cx.global::<ThemeState>().theme().clone();
        Self::new(theme, cx)
    }

    pub fn show_for_active_theme(&mut self, cx: &mut Context<Self>) {
        let theme = cx.global::<ThemeState>().theme().clone();
        self.editable_theme = theme;
        self.rebuild_inputs(cx);
        self.save_status = None;
        self.invalid_color_fields.clear();
        self.invalid_status_fields.clear();
        self.needs_focus = true;
        self.visible = true;
        cx.notify();
    }

    pub fn new(theme: Arc<Theme>, cx: &mut Context<Self>) -> Self {
        let (color_fields, color_inputs) = Self::build_color_fields(&theme.colors, cx);
        let (status_fields, status_inputs) = Self::build_status_fields(&theme.status, cx);
        Self {
            visible: false,
            editable_theme: theme,
            focus_handle: cx.focus_handle(),
            color_fields,
            status_fields,
            color_inputs,
            status_inputs,
            save_status: None,
            invalid_color_fields: Vec::new(),
            invalid_status_fields: Vec::new(),
            needs_focus: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(ThemeEditorEvent::Closed);
        cx.notify();
    }

    fn build_color_fields(
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> (Vec<ColorFieldEntry>, Vec<Entity<TextInput>>) {
        let field_specs: Vec<ColorFieldSpec<'_>> = vec![
            (
                "Background",
                |c: &ThemeColors| c.background,
                |c: &mut ThemeColors, v| c.background = v,
            ),
            (
                "Surface",
                |c: &ThemeColors| c.surface_background,
                |c, v| c.surface_background = v,
            ),
            (
                "Elevated Surface",
                |c: &ThemeColors| c.elevated_surface_background,
                |c, v| c.elevated_surface_background = v,
            ),
            ("Border", |c: &ThemeColors| c.border, |c, v| c.border = v),
            ("Text", |c: &ThemeColors| c.text, |c, v| c.text = v),
            (
                "Text Muted",
                |c: &ThemeColors| c.text_muted,
                |c, v| c.text_muted = v,
            ),
            (
                "Text Accent",
                |c: &ThemeColors| c.text_accent,
                |c, v| c.text_accent = v,
            ),
            (
                "Text Placeholder",
                |c: &ThemeColors| c.text_placeholder,
                |c, v| c.text_placeholder = v,
            ),
            ("Icon", |c: &ThemeColors| c.icon, |c, v| c.icon = v),
            (
                "Focus Ring",
                |c: &ThemeColors| c.border_focused,
                |c, v| c.border_focused = v,
            ),
            (
                "Selected Border",
                |c: &ThemeColors| c.border_selected,
                |c, v| c.border_selected = v,
            ),
            ("Added", |c: &ThemeColors| c.vc_added, |c, v| c.vc_added = v),
            (
                "Modified",
                |c: &ThemeColors| c.vc_modified,
                |c, v| c.vc_modified = v,
            ),
            (
                "Deleted",
                |c: &ThemeColors| c.vc_deleted,
                |c, v| c.vc_deleted = v,
            ),
            (
                "Untracked",
                |c: &ThemeColors| c.vc_untracked,
                |c, v| c.vc_untracked = v,
            ),
            (
                "Conflict",
                |c: &ThemeColors| c.vc_conflict,
                |c, v| c.vc_conflict = v,
            ),
            (
                "Renamed",
                |c: &ThemeColors| c.vc_renamed,
                |c, v| c.vc_renamed = v,
            ),
        ];

        let mut inputs = vec![];
        let mut entries = vec![];
        for (label, getter, setter) in field_specs {
            let hex = hsla_to_hex(getter(colors));
            let input = cx.new(|cx| {
                let mut ti = TextInput::new(cx);
                ti.set_text(hex, cx);
                ti
            });
            inputs.push(input);
            entries.push(ColorFieldEntry {
                label: label.to_string(),
                getter,
                setter,
            });
        }
        (entries, inputs)
    }

    fn build_status_fields(
        status: &StatusColors,
        cx: &mut Context<Self>,
    ) -> (Vec<StatusFieldEntry>, Vec<Entity<TextInput>>) {
        let field_specs: Vec<StatusFieldSpec<'_>> = vec![
            (
                "Error",
                |c: &StatusColors| c.error,
                |c: &mut StatusColors, v| c.error = v,
            ),
            (
                "Warning",
                |c: &StatusColors| c.warning,
                |c, v| c.warning = v,
            ),
            (
                "Success",
                |c: &StatusColors| c.success,
                |c, v| c.success = v,
            ),
            ("Info", |c: &StatusColors| c.info, |c, v| c.info = v),
        ];

        let mut inputs = vec![];
        let mut entries = vec![];
        for (label, getter, setter) in field_specs {
            let hex = hsla_to_hex(getter(status));
            let input = cx.new(|cx| {
                let mut ti = TextInput::new(cx);
                ti.set_text(hex, cx);
                ti
            });
            inputs.push(input);
            entries.push(StatusFieldEntry {
                label: label.to_string(),
                getter,
                setter,
            });
        }
        (entries, inputs)
    }

    fn rebuild_inputs(&mut self, cx: &mut Context<Self>) {
        let mut color_inputs = vec![];
        for field in &self.color_fields {
            let hex = hsla_to_hex((field.getter)(&self.editable_theme.colors));
            let input = cx.new(|cx| {
                let mut ti = TextInput::new(cx);
                ti.set_text(hex, cx);
                ti
            });
            color_inputs.push(input);
        }
        self.color_inputs = color_inputs;

        let mut status_inputs = vec![];
        for field in &self.status_fields {
            let hex = hsla_to_hex((field.getter)(&self.editable_theme.status));
            let input = cx.new(|cx| {
                let mut ti = TextInput::new(cx);
                ti.set_text(hex, cx);
                ti
            });
            status_inputs.push(input);
        }
        self.status_inputs = status_inputs;
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.dismiss(cx);
            }
            "enter" => {
                self.save(cx);
            }
            "tab" => {
                self.advance_focus(event.keystroke.modifiers.shift, window, cx);
            }
            _ => {}
        }
    }

    /// Cycle keyboard focus across the color and status inputs, wrapping at the
    /// ends. Tab bubbles up from the focused input (which consumes it without
    /// stopping propagation) to the modal's key handler, so this is the dialog's
    /// own focus traversal.
    fn advance_focus(&self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handles: Vec<FocusHandle> = self
            .color_inputs
            .iter()
            .chain(self.status_inputs.iter())
            .map(|input| input.read(cx).focus_handle(cx))
            .collect();
        if handles.is_empty() {
            return;
        }

        let current = handles.iter().position(|h| h.is_focused(window));
        let len = handles.len();
        let next = match current {
            Some(idx) if reverse => (idx + len - 1) % len,
            Some(idx) => (idx + 1) % len,
            None => 0,
        };
        handles[next].focus(window, cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let mut colors = self.editable_theme.colors.clone();
        let mut status = self.editable_theme.status.clone();
        let mut invalid_color_fields = Vec::new();
        let mut invalid_status_fields = Vec::new();

        for (i, field) in self.color_fields.iter().enumerate() {
            let text = self.color_inputs[i].read(cx).text().trim().to_string();
            if text.is_empty() {
                continue;
            }
            // Use strict parser: invalid hex is flagged for feedback instead of
            // silently reverting to the current color.
            match hex_to_hsla_strict(&text) {
                Some(hsla) => (field.setter)(&mut colors, hsla),
                None => invalid_color_fields.push(i),
            }
        }

        for (i, field) in self.status_fields.iter().enumerate() {
            let text = self.status_inputs[i].read(cx).text().trim().to_string();
            if text.is_empty() {
                continue;
            }
            match hex_to_hsla_strict(&text) {
                Some(hsla) => (field.setter)(&mut status, hsla),
                None => invalid_status_fields.push(i),
            }
        }

        let invalid_count = invalid_color_fields.len() + invalid_status_fields.len();
        self.invalid_color_fields = invalid_color_fields;
        self.invalid_status_fields = invalid_status_fields;

        let mut theme = (*self.editable_theme).clone();
        theme.colors = colors;
        theme.status = status;

        match save_theme_to_file(&theme) {
            Ok(path) => {
                log::info!("Theme saved to {}", path.display());
                let message = if invalid_count == 0 {
                    format!("Saved to {}", path.display())
                } else {
                    format!(
                        "Saved to {} · {} field{} had invalid hex and {} not changed",
                        path.display(),
                        invalid_count,
                        if invalid_count == 1 { "" } else { "s" },
                        if invalid_count == 1 { "was" } else { "were" }
                    )
                };
                self.save_status = Some(SaveStatus::Saved {
                    message,
                    invalid_count,
                });
                let new_theme = Arc::new(theme);
                cx.update_global::<ThemeState, _>(|state, _cx| {
                    state.insert_theme(new_theme.clone());
                });
                // Update editable_theme so subsequent saves start from the last-saved state,
                // not the pre-save state. This matters if the user keeps the dialog open
                // and edits another color field before saving again.
                self.editable_theme = new_theme.clone();
                cx.emit(ThemeEditorEvent::Saved(new_theme.clone()));
            }
            Err(e) => {
                self.save_status = Some(SaveStatus::Failed(format!("Save failed: {}", e)));
            }
        }
        cx.notify();
    }
}

impl EventEmitter<ThemeEditorEvent> for ThemeEditorDialog {}

impl Render for ThemeEditorDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("theme-editor").into_any_element();
        }

        // Focus the first color input the first time the dialog renders after
        // opening, so Enter/Tab work immediately without a manual click.
        if self.needs_focus {
            self.needs_focus = false;
            if let Some(handle) = self
                .color_inputs
                .first()
                .map(|i| i.read(cx).focus_handle(cx))
            {
                handle.focus(window, cx);
            }
        }

        let colors = cx.colors();
        let invalid_border = cx.status().error;
        let valid_border = colors.border_transparent;
        let theme = self.editable_theme.clone();
        let theme_name = theme.name.clone();
        let appearance = theme.appearance;

        div()
            .id("theme-editor-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.6,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .child(
                div()
                    .id("theme-editor-modal")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .v_flex()
                    .w(px(860.))
                    .max_w(relative(0.92))
                    .h(px(580.))
                    .max_h(relative(0.9))
                    .elevation_3(cx)
                    .rounded(px(12.))
                    .overflow_hidden()
                    // Header
                    .child(
                        div()
                            .id("theme-editor-header")
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .px(px(20.))
                            .py(px(16.))
                            .border_b_1()
                            .border_color(colors.border)
                            .bg(colors.surface_background)
                            .rounded_t(px(12.))
                            .child(
                                Icon::new(IconName::Edit)
                                    .size(IconSize::Small)
                                    .color(Color::Accent),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .child(
                                        Label::new("Theme Editor")
                                            .size(LabelSize::Large)
                                            .weight(gpui::FontWeight::BOLD),
                                    )
                                    .child(
                                        Label::new(format!(
                                            "{} · {}",
                                            theme_name,
                                            if appearance == Appearance::Dark {
                                                "dark"
                                            } else {
                                                "light"
                                            }
                                        ))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            ),
                    )
                    // Body
                    .child(
                        div()
                            .id("theme-editor-body")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(20.))
                            .py(px(16.))
                            .v_flex()
                            .gap(px(24.))
                            // Theme Colors section
                            .child(
                                div()
                                    .v_flex()
                                    .gap_2()
                                    .child(
                                        Label::new("Colors")
                                            .size(LabelSize::Small)
                                            .weight(gpui::FontWeight::BOLD)
                                            .color(Color::Muted),
                                    )
                                    .child(div().v_flex().gap_1().children(
                                        self.color_inputs.iter().enumerate().map(
                                            |(i, input_ent)| {
                                                let hex_val = input_ent.read(cx).text().to_string();
                                                let field = &self.color_fields[i];
                                                let label_text = field.label.clone();
                                                let fallback =
                                                    (field.getter)(&self.editable_theme.colors);
                                                let invalid =
                                                    self.invalid_color_fields.contains(&i);
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(if invalid {
                                                        invalid_border
                                                    } else {
                                                        valid_border
                                                    })
                                                    .child(
                                                        div().w(px(180.)).overflow_hidden().child(
                                                            Label::new(label_text)
                                                                .size(LabelSize::Small)
                                                                .truncate(),
                                                        ),
                                                    )
                                                    .child(
                                                        div()
                                                            .size(px(28.))
                                                            .rounded(px(4.))
                                                            .border_1()
                                                            .border_color(colors.border)
                                                            .bg(Self::preview_color(
                                                                &hex_val, fallback,
                                                            )),
                                                    )
                                                    .child(input_ent.clone())
                                            },
                                        ),
                                    )),
                            )
                            // Status Colors section
                            .child(
                                div()
                                    .v_flex()
                                    .gap_2()
                                    .child(
                                        Label::new("Status Colors")
                                            .size(LabelSize::Small)
                                            .weight(gpui::FontWeight::BOLD)
                                            .color(Color::Muted),
                                    )
                                    .child(div().v_flex().gap_1().children(
                                        self.status_inputs.iter().enumerate().map(
                                            |(i, input_ent)| {
                                                let hex_val = input_ent.read(cx).text().to_string();
                                                let field = &self.status_fields[i];
                                                let label_text = field.label.clone();
                                                let fallback =
                                                    (field.getter)(&self.editable_theme.status);
                                                let invalid =
                                                    self.invalid_status_fields.contains(&i);
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(if invalid {
                                                        invalid_border
                                                    } else {
                                                        valid_border
                                                    })
                                                    .child(
                                                        div().w(px(180.)).overflow_hidden().child(
                                                            Label::new(label_text)
                                                                .size(LabelSize::Small)
                                                                .truncate(),
                                                        ),
                                                    )
                                                    .child(
                                                        div()
                                                            .size(px(28.))
                                                            .rounded(px(4.))
                                                            .border_1()
                                                            .border_color(colors.border)
                                                            .bg(Self::preview_color(
                                                                &hex_val, fallback,
                                                            )),
                                                    )
                                                    .child(input_ent.clone())
                                            },
                                        ),
                                    )),
                            ),
                    )
                    // Footer
                    .child(
                        div()
                            .id("theme-editor-footer")
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .px(px(20.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(colors.border)
                            .bg(colors.surface_background)
                            .rounded_b(px(12.))
                            .child({
                                let (text, color) = match &self.save_status {
                                    Some(SaveStatus::Failed(msg)) => (msg.clone(), Color::Error),
                                    Some(SaveStatus::Saved {
                                        message,
                                        invalid_count,
                                    }) => {
                                        let color = if *invalid_count == 0 {
                                            Color::Success
                                        } else {
                                            Color::Warning
                                        };
                                        (message.clone(), color)
                                    }
                                    None => (
                                        "Tab to move · Enter to save · Esc to close".to_string(),
                                        Color::Placeholder,
                                    ),
                                };
                                Label::new(text).size(LabelSize::Small).color(color)
                            })
                            .child(
                                div().h_flex().gap_2().children([
                                    Button::new("btn-cancel", "Cancel")
                                        .style(ButtonStyle::Subtle)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.dismiss(cx);
                                        })),
                                    Button::new("btn-save", "Save")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.save(cx);
                                        })),
                                ]),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl ThemeEditorDialog {
    fn preview_color(hex: &str, fallback: Hsla) -> Hsla {
        hex_to_hsla_strict(hex.trim()).unwrap_or(fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeEditorDialog;
    use gpui::Hsla;

    fn fallback() -> Hsla {
        Hsla {
            h: 0.5,
            s: 0.5,
            l: 0.5,
            a: 1.0,
        }
    }

    #[test]
    fn preview_color_empty_returns_fallback() {
        assert_eq!(ThemeEditorDialog::preview_color("", fallback()), fallback());
    }

    #[test]
    fn preview_color_partial_returns_fallback() {
        assert_eq!(
            ThemeEditorDialog::preview_color("#12", fallback()),
            fallback()
        );
    }

    #[test]
    fn preview_color_non_hex_returns_fallback() {
        assert_eq!(
            ThemeEditorDialog::preview_color("#zzzzzz", fallback()),
            fallback()
        );
    }

    #[test]
    fn preview_color_valid_hex_parses() {
        let parsed = ThemeEditorDialog::preview_color("#ff0000", fallback());
        assert!((parsed.l - 0.5).abs() < 1e-3);
        assert!((parsed.s - 1.0).abs() < 1e-3);
    }

    #[test]
    fn preview_color_valid_hex_with_alpha() {
        let parsed = ThemeEditorDialog::preview_color("#00ff0080", fallback());
        assert!(parsed.a < 1.0);
    }
}
