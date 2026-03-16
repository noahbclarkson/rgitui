use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonStyle, IconName, Label, LabelSize};

/// Events emitted by the repository opener dialog.
#[derive(Debug, Clone)]
pub enum RepoOpenerEvent {
    OpenRepo(PathBuf),
    Dismissed,
}

/// A modal dialog for opening a Git repository by path or from recent repos.
pub struct RepoOpener {
    query: String,
    cursor_pos: usize,
    recent_repos: Vec<PathBuf>,
    filtered_indices: Vec<usize>,
    selected_index: Option<usize>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<RepoOpenerEvent> for RepoOpener {}

impl RepoOpener {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            cursor_pos: 0,
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
            self.query.clear();
            self.cursor_pos = 0;
            self.selected_index = None;
            // Load recent repos from settings
            self.recent_repos = cx
                .global::<rgitui_settings::SettingsState>()
                .settings()
                .recent_repos
                .clone();
            self.update_filter();
            self.focus_handle.focus(window, cx);
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
            self.query.clear();
            self.cursor_pos = 0;
            self.selected_index = None;
            self.recent_repos = cx
                .global::<rgitui_settings::SettingsState>()
                .settings()
                .recent_repos
                .clone();
            self.update_filter();
        }
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.query.clear();
        self.cursor_pos = 0;
        cx.emit(RepoOpenerEvent::Dismissed);
        cx.notify();
    }

    fn update_filter(&mut self) {
        let query = self.query.to_lowercase();
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
        let path = if !self.query.is_empty() {
            // Use the typed path
            let expanded = if self.query.starts_with('~') {
                if let Ok(home) = std::env::var("HOME") {
                    PathBuf::from(home).join(self.query[1..].trim_start_matches('/'))
                } else {
                    PathBuf::from(&self.query)
                }
            } else {
                PathBuf::from(&self.query)
            };
            expanded
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
        self.query.clear();
        self.cursor_pos = 0;
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
                        this.query.clear();
                        this.cursor_pos = 0;
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
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();

        if keystroke.modifiers.control || keystroke.modifiers.platform {
            match key {
                "v" => {
                    if let Some(clipboard) = cx.read_from_clipboard() {
                        if let Some(text) = clipboard.text() {
                            let line = text.lines().next().unwrap_or("").trim();
                            self.query = line.to_string();
                            self.cursor_pos = self.query.len();
                            self.update_filter();
                            cx.notify();
                        }
                    }
                    return;
                }
                "a" => {
                    self.cursor_pos = self.query.len();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match key {
            "escape" => {
                self.dismiss(cx);
            }
            "enter" => {
                self.try_open(cx);
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
            "backspace" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.query.remove(self.cursor_pos);
                    self.update_filter();
                    cx.notify();
                }
            }
            "delete" => {
                if self.cursor_pos < self.query.len() {
                    self.query.remove(self.cursor_pos);
                    self.update_filter();
                    cx.notify();
                }
            }
            "left" => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    cx.notify();
                }
            }
            "right" => {
                if self.cursor_pos < self.query.len() {
                    self.cursor_pos += 1;
                    cx.notify();
                }
            }
            "home" => {
                self.cursor_pos = 0;
                cx.notify();
            }
            "end" => {
                self.cursor_pos = self.query.len();
                cx.notify();
            }
            _ => {
                if let Some(key_char) = &keystroke.key_char {
                    self.query.insert_str(self.cursor_pos, key_char);
                    self.cursor_pos += key_char.len();
                    self.update_filter();
                    cx.notify();
                } else if key.len() == 1
                    && !keystroke.modifiers.control
                    && !keystroke.modifiers.platform
                {
                    let ch = key.chars().next().unwrap();
                    if ch.is_ascii_graphic() || ch == ' ' {
                        self.query.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                        self.update_filter();
                        cx.notify();
                    }
                }
            }
        }
    }
}

impl Render for RepoOpener {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("repo-opener").into_any_element();
        }

        // Build the input display with cursor
        let (before_cursor, cursor_char, after_cursor) = if self.query.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            let before = self.query[..self.cursor_pos].to_string();
            let cursor = if self.cursor_pos < self.query.len() {
                self.query[self.cursor_pos..self.cursor_pos + 1].to_string()
            } else {
                String::new()
            };
            let after = if self.cursor_pos + 1 < self.query.len() {
                self.query[self.cursor_pos + 1..].to_string()
            } else {
                String::new()
            };
            (before, cursor, after)
        };

        let is_empty = self.query.is_empty();

        // Text input field
        let mut input_row = div().h_flex().items_center().w_full();

        if is_empty {
            input_row = input_row
                .child(div().w(px(2.)).h(px(16.)).bg(colors.text))
                .child(
                    Label::new("/path/to/repository")
                        .size(LabelSize::Small)
                        .color(Color::Placeholder),
                );
        } else {
            if !before_cursor.is_empty() {
                input_row = input_row
                    .child(Label::new(SharedString::from(before_cursor)).size(LabelSize::Small));
            }
            if !cursor_char.is_empty() {
                input_row = input_row.child(
                    div().bg(colors.text).child(
                        Label::new(SharedString::from(cursor_char))
                            .size(LabelSize::Small)
                            .color(Color::Custom(gpui::Hsla {
                                h: 0.0,
                                s: 0.0,
                                l: 0.0,
                                a: 1.0,
                            })),
                    ),
                );
            } else {
                // Cursor at end
                input_row = input_row.child(div().w(px(2.)).h(px(16.)).bg(colors.text));
            }
            if !after_cursor.is_empty() {
                input_row = input_row
                    .child(Label::new(SharedString::from(after_cursor)).size(LabelSize::Small));
            }
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
                                .h_flex()
                                .flex_1()
                                .h(px(30.))
                                .px_2()
                                .bg(colors.editor_background)
                                .border_1()
                                .border_color(colors.border_variant)
                                .rounded_md()
                                .items_center()
                                .cursor_text()
                                .child(input_row),
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
                        this.query.clear();
                        this.cursor_pos = 0;
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
