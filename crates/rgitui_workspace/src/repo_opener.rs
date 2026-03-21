use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, Render, SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

#[derive(Debug, Clone)]
pub enum RepoOpenerEvent {
    OpenRepo(PathBuf),
    Dismissed,
}

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
            ti.set_placeholder("Enter repository path...");
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
            }
            .await;
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
        })
        .detach();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        match key {
            "escape" => {
                self.dismiss(cx);
                cx.stop_propagation();
            }
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
                cx.stop_propagation();
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
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for RepoOpener {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("repo-opener").into_any_element();
        }

        let colors = cx.colors();

        let mut modal = div()
            .id("repo-opener-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(500.))
            .max_h(px(480.))
            .elevation_3(cx)
            .rounded(px(10.))
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            });

        modal = modal.child(
            div()
                .h_flex()
                .w_full()
                .h(px(48.))
                .px(px(16.))
                .items_center()
                .border_b_1()
                .border_color(colors.border_variant)
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            Icon::new(IconName::Folder)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Open Repository")
                                .size(LabelSize::Large)
                                .weight(FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    div()
                        .id("repo-opener-close-btn")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(28.))
                        .h(px(28.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.dismiss(cx);
                        }))
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        ),
                ),
        );

        let focused_border = colors.border_focused;
        modal = modal.child(
            div()
                .px(px(16.))
                .py(px(12.))
                .v_flex()
                .gap(px(8.))
                .child(
                    Label::new("Repository path")
                        .size(LabelSize::Small)
                        .weight(FontWeight::MEDIUM)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .h_flex()
                                .items_center()
                                .gap(px(8.))
                                .px(px(8.))
                                .border_1()
                                .border_color(focused_border)
                                .rounded(px(6.))
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(IconSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    div().flex_1().child(self.editor.clone()),
                                ),
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

        if !self.recent_repos.is_empty() {
            modal = modal.child(
                div()
                    .h_flex()
                    .w_full()
                    .px(px(16.))
                    .pt(px(4.))
                    .pb(px(8.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .items_center()
                    .child(
                        Label::new("Recent Repositories")
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    ),
            );

            let mut results = div()
                .id("repo-opener-results")
                .v_flex()
                .w_full()
                .px(px(8.))
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

                let hover_bg = colors.ghost_element_hover;
                let selected_bg = colors.element_selected;

                let row = div()
                    .id(ElementId::NamedInteger(
                        "repo-opener-item".into(),
                        display_idx as u64,
                    ))
                    .h_flex()
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .gap(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_selected, move |el| el.bg(selected_bg))
                    .hover(move |s| s.bg(hover_bg))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let path = path_clone.clone();
                        this.visible = false;
                        this.editor.update(cx, |e, cx| e.clear(cx));
                        cx.emit(RepoOpenerEvent::OpenRepo(path));
                        cx.notify();
                    }))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Small)
                            .color(if is_selected {
                                Color::Accent
                            } else {
                                Color::Muted
                            }),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .gap(px(2.))
                            .child(
                                Label::new(repo_name)
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::MEDIUM)
                                    .color(Color::Default),
                            )
                            .child(
                                Label::new(repo_path_display)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    );

                results = results.child(row);
            }

            if self.filtered_indices.is_empty() {
                results = results.child(
                    div()
                        .w_full()
                        .py(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .v_flex()
                        .gap(px(8.))
                        .child(
                            Icon::new(IconName::Search)
                                .size(IconSize::Large)
                                .color(Color::Placeholder),
                        )
                        .child(
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
                    .w_full()
                    .py(px(32.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .flex()
                    .items_center()
                    .justify_center()
                    .v_flex()
                    .gap(px(8.))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new("No recent repositories")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        }

        modal = modal.child(
            div()
                .h_flex()
                .justify_end()
                .gap(px(8.))
                .px(px(16.))
                .py(px(12.))
                .border_t_1()
                .border_color(colors.border_variant)
                .bg(colors.surface_background)
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
                a: 0.4,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}
