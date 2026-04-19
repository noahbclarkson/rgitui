use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Window,
};
use rgitui_theme::{
    hex_to_hsla, hsla_to_hex, json_theme::save_theme_to_file, ActiveTheme, Appearance, Color,
    StyledExt, Theme, ThemeState,
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
    save_status: Option<String>,
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
        self.visible = true;
        cx.notify();
    }

    pub fn new(theme: Arc<Theme>, cx: &mut Context<Self>) -> Self {
        let (color_fields, color_inputs) = Self::build_color_fields(&theme.colors, cx);
        let (status_fields, status_inputs) = Self::build_status_fields(&theme.status, cx);
        Self {
            visible: true,
            editable_theme: theme,
            focus_handle: cx.focus_handle(),
            color_fields,
            status_fields,
            color_inputs,
            status_inputs,
            save_status: None,
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
            ("Icon", |c: &ThemeColors| c.icon, |c, v| c.icon = v),
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
        ];

        let mut inputs = vec![];
        let mut entries = vec![];
        for (label, getter, setter) in field_specs {
            let hex = hsla_to_hex(getter(colors));
            let input = cx.new(|cx| {
                let mut ti = TextInput::new(cx);
                ti.set_placeholder(&hex);
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
                ti.set_placeholder(&hex);
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
                ti.set_placeholder(&hex);
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
                ti.set_placeholder(&hex);
                ti
            });
            status_inputs.push(input);
        }
        self.status_inputs = status_inputs;
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.dismiss(cx);
            }
            "enter" | "cmd+enter" => {
                self.save(cx);
            }
            _ => {}
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let mut colors = self.editable_theme.colors.clone();
        let mut status = self.editable_theme.status.clone();

        for (i, field) in self.color_fields.iter().enumerate() {
            let text = self.color_inputs[i].read(cx).text();
            let hsla = hex_to_hsla(text);
            ((field.setter)(&mut colors, hsla));
        }

        for (i, field) in self.status_fields.iter().enumerate() {
            let text = self.status_inputs[i].read(cx).text();
            let hsla = hex_to_hsla(text);
            ((field.setter)(&mut status, hsla));
        }

        let mut theme = (*self.editable_theme).clone();
        theme.colors = colors;
        theme.status = status;

        match save_theme_to_file(&theme) {
            Ok(path) => {
                log::info!("Theme saved to {}", path.display());
                self.save_status = Some("Saved!".to_string());
                let new_theme = Arc::new(theme);
                cx.update_global::<ThemeState, _>(|state, _cx| {
                    state.insert_theme(new_theme.clone());
                });
                cx.emit(ThemeEditorEvent::Saved(new_theme.clone()));
            }
            Err(e) => {
                self.save_status = Some(format!("Save failed: {}", e));
            }
        }
        cx.notify();
    }
}

impl EventEmitter<ThemeEditorEvent> for ThemeEditorDialog {}

impl Render for ThemeEditorDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("theme-editor").into_any_element();
        }

        let colors = cx.colors();
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
                    .h(px(580.))
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
                                                let hex_val = input_ent.read(cx).text();
                                                let label_text = self.color_fields[i].label.clone();
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .rounded(px(6.))
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
                                                            .bg(Self::parse_hex_or_default(
                                                                hex_val,
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
                                                let hex_val = input_ent.read(cx).text();
                                                let label_text =
                                                    self.status_fields[i].label.clone();
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .rounded(px(6.))
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
                                                            .bg(Self::parse_hex_or_default(
                                                                hex_val,
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
                            .child(
                                Label::new("Enter to save · Esc to close")
                                    .size(LabelSize::Small)
                                    .color(Color::Placeholder),
                            )
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
    fn parse_hex_or_default(hex: &str) -> Hsla {
        hex_to_hsla(hex)
    }
}
