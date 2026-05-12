use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FontWeight, KeyDownEvent, Render, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, IconName, Label, LabelSize, TextInput};

#[derive(Debug, Clone)]
pub enum RepoCloneEvent {
    CloneRepo { url: String, path: PathBuf },
    Dismissed,
}

pub struct RepoCloneDialog {
    visible: bool,
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

    pub fn is_visible(&self) -> bool {
        self.visible
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

    fn browse_path(&mut self, cx: &mut Context<Self>) {
        let current_url = self.url_editor.read(cx).text().to_string();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Select Parent Folder")
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf());
            let Some(folder) = picked else { return };
            let destination = match repo_name_from_url(&current_url) {
                Some(name) => folder.join(name),
                None => folder,
            };
            cx.update(|cx| {
                let _ = this.update(cx, |this, cx| {
                    this.path_editor.update(cx, |e, cx| {
                        e.set_text(destination.to_string_lossy().into_owned(), cx);
                    });
                    cx.notify();
                });
            });
        })
        .detach();
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

fn repo_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let last = trimmed
        .rsplit_once(['/', ':'])
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

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
                        div().v_flex().gap(px(8.)).child(Label::new("Path")).child(
                            div()
                                .h_flex()
                                .gap(px(8.))
                                .items_center()
                                .child(div().flex_1().child(self.path_editor.clone()))
                                .child(
                                    Button::new("browse-path", "Browse")
                                        .style(ButtonStyle::Subtle)
                                        .icon(IconName::Folder)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.browse_path(cx);
                                        })),
                                ),
                        ),
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

#[cfg(test)]
mod tests {
    use super::repo_name_from_url;

    #[test]
    fn https_url_with_git_suffix() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn https_url_without_git_suffix() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn https_url_trailing_slash() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo/"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn scp_style_ssh_url() {
        assert_eq!(
            repo_name_from_url("git@github.com:user/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn ssh_url_scheme() {
        assert_eq!(
            repo_name_from_url("ssh://git@github.com/user/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn empty_url_returns_none() {
        assert_eq!(repo_name_from_url(""), None);
        assert_eq!(repo_name_from_url("   "), None);
    }

    #[test]
    fn bare_repo_name() {
        assert_eq!(repo_name_from_url("repo.git"), Some("repo".to_string()));
    }
}
