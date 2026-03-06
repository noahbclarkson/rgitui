use gpui::prelude::*;
use gpui::{div, px, App, Context, FocusHandle, Focusable, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::Label;

/// A single-line text input field.
pub struct TextInput {
    text: String,
    placeholder: SharedString,
    focus_handle: FocusHandle,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            placeholder: "".into(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.text = text;
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.text.clear();
        cx.notify();
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let is_focused = self.focus_handle.is_focused(_window);

        let border_color = if is_focused {
            colors.border_focused
        } else {
            colors.border
        };

        let display_text = if self.text.is_empty() {
            Label::new(self.placeholder.clone()).color(Color::Placeholder)
        } else {
            Label::new(self.text.clone())
        };

        div()
            .track_focus(&self.focus_handle)
            .h_flex()
            .h(px(28.))
            .px_2()
            .bg(colors.editor_background)
            .border_1()
            .border_color(border_color)
            .rounded_md()
            .child(display_text)
    }
}
