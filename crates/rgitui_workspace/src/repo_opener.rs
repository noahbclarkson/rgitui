use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, KeyDownEvent, Render, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, IconName, Label, LabelSize, TextInput, TextInputEvent};

/// Events emitted by the repository opener dialog.
#[derive(Debug, Clone)]
pub enum RepoOpenerEvent {
    OpenRepo(PathBuf),
    Dismissed,
}

/// A modal dialog for opening a Git repository by path or from recent repos.
pub struct RepoOpener {
    editor: Entity<TextInput>,
    recent_repos: Vec<PathBuf>,
    filtered_indices: Vec<usize>,
    selected_index: Option<usize>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<RepoOpenerEvent> for RepoOpener {}

impl RepoOpener {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("/path/to/repository");
            ti
        });
        cx.subscribe(&editor, |this: &mut Self, _, event: &TextInputEvent, cx| {
            match event {
                TextInputEvent::Submit => {
                    this.try_open(cx);
                }
                TextInputEvent::Changed(_) => {
                    this.update_filter(cx);
                    cx.notify();
                }
            }
        })
        .detach();

        Self {
            editor,
            recent_repos: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: None,
            visible: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.editor.update(cx, |e, cx| e.clear(cx));
            self.selected_index = None;
            self.recent_repos = cx
                .global::<rgitui_settings::SettingsState>()
                .settings()
                .recent_repos
                .clone();
            self.update_filter(cx);
            self.editor.update(cx, |e, cx| e.focus(window, cx));
        }
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Toggle visibility without focusing (for use from command palette).
    pub fn toggle_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.editor.update(cx, |e, cx| e.clear(cx));
            self.selected_index = None;
            self.recent_repos = cx
                .global::<rgitui_settings::SettingsState>()
                .settings()
                .recent_repos
                .clone();
            self.update_filter(cx);
        }
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        cx.emit(RepoOpenerEvent::Dismissed);
        cx.notify();
    }

    fn update_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.editor.read(cx).text().to_lowercase();
        if query.is_empty() {
            self.filtered_indices = (0..self.recent_repos.len()).collect();
        } else {
            self.filtered_indices = self
                .recent_repos
                .iter()
                .enumerate()
                .filter(|(_, path)| {
                    let path_str = path.to_string_lossy().to_lowercase();
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    path_str.contains(&query) || name.contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_index = None;
    }

    fn try_open(&mut self, cx: &mut Context<Self>) {
        let query = self.editor.read(cx).text().to_string();
        let path = if !query.is_empty() {
            if let Some(stripped) = query.strip_prefix('~') {
                if let Some(home) = dirs::home_dir() {
                    home.join(stripped.trim_start_matches('/'))
                } else {
                    PathBuf::from(&query)
                }
            } else {
                PathBuf::from(&query)
            }
        } else if let Some(selected_index) = self.selected_index {
            if let Some(&idx) = self.filtered_indices.get(selected_index) {
                self.recent_repos[idx].clone()
            } else {
                return;
            }
        } else {
            return;
        };

        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        cx.emit(RepoOpenerEvent::OpenRepo(path));
        cx.notify();
    }

    fn browse_folder(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let folder = async {
                rfd::AsyncFileDialog::new()
                    .set_title("Select Git Repository")
                    .pick_folder()
                    .await
                    .map(|handle| handle.path().to_path_buf())
            }.await;
            if let Some(path) = folder {
                cx.update(|cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.visible = false;
                        this.editor.update(cx, |e, cx| e.clear(cx));
                        cx.emit(RepoOpenerEvent::OpenRepo(path));
                        cx.notify();
                    });
                });
            }
        }).detach();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        match key {
            "up" => {
                if self.filtered_indices.is_empty() {
                    return;
                }
                self.selected_index = Some(match self.selected_index {
                    Some(index) if index > 0 => index - 1,
                    Some(index) => index,
                    None => self.filtered_indices.len().saturating_sub(1),
                });
                cx.notify();
            }
            "down" => {
                if self.filtered_indices.is_empty() {
                    return;
                }
                self.selected_index = Some(match self.selected_index {
                    Some(index) if index + 1 < self.filtered_indices.len() => index + 1,
                    Some(index) => index,
                    None => 0,
                });
                cx.notify();
            }
            _ => {}
        }
    }
}

impl Render for RepoOpener {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("repo-opener").into_any_element();
        }

        // Build the modal content
        let mut modal = div()
            .id("repo-opener-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(500.))
            .max_h(px(450.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .elevation_3(cx)
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        // Title
        modal = modal.child(
            div().px_4().pt_4().pb_2().child(
                Label::new("Open Repository")
                    .size(LabelSize::Large)
                    .weight(gpui::FontWeight::BOLD)
                    .color(Color::Default),
            ),
        );

        // Path input with browse button
        modal = modal.child(
            div()
                .px_4()
                .pb_2()
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Repository path")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(self.editor.clone()),
                        )
                        .child(
                            Button::new("browse-folder", "Browse")
                                .style(ButtonStyle::Subtle)
                                .icon(IconName::Folder)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.browse_folder(cx);
                                })),
                        ),
                ),
        );

        // Hint text
        modal = modal.child(
            div().px_4().pb_1().child(
                Label::new("Type a path, browse for a folder, or select from recent repositories")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            ),
        );

        // Recent repos section
        if !self.recent_repos.is_empty() {
            modal = modal.child(
                div()
                    .px_4()
                    .pt_1()
                    .pb_1()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new("Recent Repositories")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    ),
            );

            let mut results = div()
                .id("repo-opener-results")
                .v_flex()
                .w_full()
                .overflow_y_scroll()
                .max_h(px(260.));

            for (display_idx, &repo_idx) in self.filtered_indices.iter().enumerate() {
                let repo_path = &self.recent_repos[repo_idx];
                let repo_name: SharedString = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| repo_path.to_string_lossy().to_string())
                    .into();
                let repo_path_display: SharedString =
                    repo_path.to_string_lossy().to_string().into();
                let is_selected = self.selected_index == Some(display_idx);
                let path_clone = repo_path.clone();

                let row = div()
                    .id(ElementId::NamedInteger(
                        "repo-opener-item".into(),
                        display_idx as u64,
                    ))
                    .v_flex()
                    .w_full()
                    .px(px(12.))
                    .py(px(6.))
                    .mx(px(4.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(colors.ghost_element_selected))
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let path = path_clone.clone();
                        this.visible = false;
                        this.editor.update(cx, |e, cx| e.clear(cx));
                        cx.emit(RepoOpenerEvent::OpenRepo(path));
                        cx.notify();
                    }))
                    .child(
                        Label::new(repo_name)
                            .size(LabelSize::Small)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(repo_path_display)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    );

                results = results.child(row);
            }

            if self.filtered_indices.is_empty() {
                results = results.child(
                    div().w_full().py_4().flex().justify_center().child(
                        Label::new("No matching repositories")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                );
            }

            modal = modal.child(results);
        } else {
            modal = modal.child(
                div()
                    .px_4()
                    .py_4()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .flex()
                    .justify_center()
                    .child(
                        Label::new("No recent repositories")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        }

        // Buttons
        modal = modal.child(
            div()
                .h_flex()
                .justify_end()
                .gap_2()
                .px_4()
                .py_3()
                .border_t_1()
                .border_color(colors.border_variant)
                .child(
                    Button::new("cancel-open", "Cancel")
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.dismiss(cx);
                        })),
                )
                .child(Button::new("open-repo", "Open").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.try_open(cx);
                    },
                ))),
        );

        // Backdrop + modal
        div()
            .id("repo-opener-backdrop")
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
                this.dismiss(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}
