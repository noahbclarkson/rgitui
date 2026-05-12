use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FontWeight, KeyDownEvent, Render, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, Label, LabelSize, TextInput};

#[derive(Debug, Clone)]
pub enum RepoCloneEvent {
    CloneRepo { url: String, path: PathBuf },
    Dismissed,
}

pub struct RepoCloneDialog {
    pub visible: bool,
    url_editor: Entity<TextInput>,
    path_editor: Entity<TextInput>,
}

impl RepoCloneDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let url_editor = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Repository URL (e.g. https://github.com/user/repo.git)");
            input
        });
        let path_editor = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Destination Path");
            input
        });

        Self {
            visible: false,
            url_editor,
            path_editor,
        }
    }

    pub fn show_visible(&mut self, default_path: Option<String>, cx: &mut Context<Self>) {
        self.visible = true;
        self.url_editor.update(cx, |e, cx| e.clear(cx));

        self.path_editor.update(cx, |e, cx| {
            if let Some(path) = default_path {
                e.set_text(path, cx);
            } else {
                e.clear(cx);
            }
        });

        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.visible = false;
            cx.emit(RepoCloneEvent::Dismissed);
            cx.notify();
        }
    }

    fn clone_repo(&mut self, cx: &mut Context<Self>) {
        let url = self.url_editor.read(cx).text();
        let path = self.path_editor.read(cx).text();

        if !url.is_empty() && !path.is_empty() {
            let path_buf = PathBuf::from(&path);
            cx.emit(RepoCloneEvent::CloneRepo {
                url: url.to_string(),
                path: path_buf,
            });
            self.visible = false;
            cx.notify();
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.hide(cx);
            }
            "enter" => {
                self.clone_repo(cx);
            }
            _ => {}
        }
    }
}

impl EventEmitter<RepoCloneEvent> for RepoCloneDialog {}

impl Render for RepoCloneDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("repo-clone-dialog-hidden");
        }

        let colors = cx.colors();

        div()
            .id("repo-clone-overlay")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(colors.surface_background)
            .flex()
            .items_center()
            .justify_center()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .id("repo-clone-dialog")
                    .w(px(500.))
                    .bg(colors.background)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_lg()
                    .shadow_lg()
                    .p(px(16.))
                    .v_flex()
                    .gap(px(16.))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div().h_flex().justify_between().items_center().child(
                            Label::new("Clone Repository")
                                .weight(FontWeight::BOLD)
                                .size(LabelSize::Large),
                        ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap(px(8.))
                            .child(Label::new("URL"))
                            .child(self.url_editor.clone()),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap(px(8.))
                            .child(Label::new("Path"))
                            .child(self.path_editor.clone()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                Button::new("cancel", "Cancel")
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.hide(cx);
                                    })),
                            )
                            .child(
                                Button::new("clone", "Clone")
                                    .style(ButtonStyle::Filled)
                                    .color(Color::Accent)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.clone_repo(cx);
                                    })),
                            ),
                    ),
            )
    }
}
