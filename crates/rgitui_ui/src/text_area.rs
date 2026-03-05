use gpui::prelude::*;
use gpui::{div, px, App, Context, FocusHandle, Focusable, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::Label;

/// A multi-line text area (for commit messages, etc.).
pub struct TextArea {
    text: String,
    placeholder: SharedString,
    focus_handle: FocusHandle,
    min_rows: usize,
}

impl TextArea {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            placeholder: "Enter commit message...".into(),
            focus_handle: cx.focus_handle(),
            min_rows: 3,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
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

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl Focusable for TextArea {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextArea {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let is_focused = self.focus_handle.is_focused(_window);

        let border_color = if is_focused {
            colors.border_focused
        } else {
            colors.border
        };

        let min_height = px(self.min_rows as f32 * 20.0 + 16.0);

        let content = if self.text.is_empty() {
            Label::new(self.placeholder.clone()).color(Color::Placeholder)
        } else {
            Label::new(self.text.clone())
        };

        div()
            .track_focus(&self.focus_handle)
            .v_flex()
            .min_h(min_height)
            .w_full()
            .p_2()
            .bg(colors.editor_background)
            .border_1()
            .border_color(border_color)
            .rounded_md()
            .child(content)
    }
}
