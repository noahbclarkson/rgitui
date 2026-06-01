use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, FontWeight, KeyDownEvent,
    Render, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent,
};

#[derive(Debug, Clone)]
pub enum RepoCloneEvent {
    CloneRepo { url: String, path: PathBuf },
    Dismissed,
}

pub struct RepoCloneDialog {
    visible: bool,
    url_editor: Entity<TextInput>,
    path_editor: Entity<TextInput>,
    focus_handle: FocusHandle,
    /// Set when the dialog is shown so the next render focuses the URL field.
    /// Lets us focus from call sites that have no `Window` without leaving the
    /// user to click in first.
    pending_focus: bool,
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

        cx.subscribe(
            &url_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if matches!(event, TextInputEvent::Submit) {
                    this.clone_repo(cx);
                }
            },
        )
        .detach();
        cx.subscribe(
            &path_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if matches!(event, TextInputEvent::Submit) {
                    this.clone_repo(cx);
                }
            },
        )
        .detach();

        Self {
            visible: false,
            url_editor,
            path_editor,
            focus_handle: cx.focus_handle(),
            pending_focus: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show_visible(&mut self, default_path: Option<String>, cx: &mut Context<Self>) {
        self.visible = true;
        self.pending_focus = true;
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
        let url = self.url_editor.read(cx).text().trim().to_string();
        let path = self.path_editor.read(cx).text().trim().to_string();

        if url.is_empty() || path.is_empty() {
            return;
        }

        // TODO(audit): UX-08 keep the dialog open in a loading state and report
        // clone success/failure back here. The detached clone task lives in the
        // workspace (workspace/events.rs subscribe_repo_clone_dialog ->
        // Project::clone_repo), so wiring completion/error back to this dialog
        // requires a result channel from those files, which are out of scope for
        // this single-file change.
        let path_buf = PathBuf::from(&path);
        cx.emit(RepoCloneEvent::CloneRepo {
            url,
            path: path_buf,
        });
        self.visible = false;
        cx.notify();
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
        // Enter is handled solely via each editor's `Submit` event so it fires
        // exactly once; here we only need the modal-level Escape-to-dismiss.
        if event.keystroke.key.as_str() == "escape" {
            self.hide(cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("repo-clone-dialog-hidden").into_any_element();
        }

        if self.pending_focus {
            self.pending_focus = false;
            self.url_editor.update(cx, |e, cx| e.focus(window, cx));
        }

        let colors = cx.colors();
        let url = self.url_editor.read(cx).text().trim().to_string();
        let path = self.path_editor.read(cx).text().trim().to_string();
        let can_clone = !url.is_empty() && !path.is_empty();

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let mut modal = div()
            .id("repo-clone-dialog-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(500.))
            .elevation_3(cx)
            .p(px(20.))
            .gap(px(16.))
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        modal = modal.child(
            div()
                .h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .rounded(px(10.))
                        .bg(icon_bg)
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(IconSize::Medium)
                                .color(Color::Accent),
                        ),
                )
                .child(
                    Label::new("Clone Repository")
                        .size(LabelSize::Large)
                        .weight(FontWeight::BOLD),
                ),
        );

        modal = modal.child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(
                    Label::new("URL")
                        .size(LabelSize::Small)
                        .weight(FontWeight::MEDIUM)
                        .color(Color::Muted),
                )
                .child(self.url_editor.clone()),
        );

        modal = modal.child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(
                    Label::new("Path")
                        .size(LabelSize::Small)
                        .weight(FontWeight::MEDIUM)
                        .color(Color::Muted),
                )
                .child(
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
        );

        if !can_clone {
            modal = modal.child(
                Label::new("Enter a repository URL and a destination path to clone.")
                    .size(LabelSize::XSmall)
                    .color(Color::Placeholder),
            );
        }

        modal = modal.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(colors.border_variant)
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
                        .disabled(!can_clone)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.clone_repo(cx);
                        })),
                ),
        );

        div()
            .id("repo-clone-backdrop")
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
                a: 0.5,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.hide(cx);
            }))
            .child(modal)
            .into_any_element()
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
