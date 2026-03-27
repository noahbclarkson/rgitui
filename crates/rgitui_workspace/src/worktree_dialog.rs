use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Render, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput,
    TextInputEvent, TintColor,
};

/// Events emitted by the worktree creation dialog.
#[derive(Debug, Clone)]
pub enum WorktreeDialogEvent {
    CreateWorktree {
        name: String,
        path: String,
        branch: Option<String>,
    },
    Dismissed,
}

/// A modal dialog for creating a new Git worktree.
pub struct WorktreeDialog {
    name_editor: Entity<TextInput>,
    path_editor: Entity<TextInput>,
    branch_editor: Entity<TextInput>,
    error_message: Option<String>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<WorktreeDialogEvent> for WorktreeDialog {}

impl WorktreeDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let name_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Worktree name (e.g. feature-x)...");
            ti
        });

        let path_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Directory path (e.g. /home/user/projects/myrepo-feature-x)...");
            ti
        });

        let branch_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Branch (optional, defaults to current branch)...");
            ti
        });

        let focus_handle = cx.focus_handle();

        cx.subscribe(
            &name_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_create(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = Self::validate_name(text);
                    cx.notify();
                }
            },
        )
        .detach();

        cx.subscribe(
            &path_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_create(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = Self::validate_path(text);
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            name_editor,
            path_editor,
            branch_editor,
            error_message: None,
            visible: false,
            focus_handle,
        }
    }

    /// Show the dialog, optionally pre-filling the branch.
    pub fn show(&mut self, branch: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.name_editor.update(cx, |e, cx| e.clear(cx));
        self.path_editor.update(cx, |e, cx| e.clear(cx));
        self.branch_editor.update(cx, |e, cx| e.clear(cx));
        if let Some(b) = branch {
            self.branch_editor.update(cx, |e, cx| e.set_text(&b, cx));
        }
        self.error_message = None;
        self.name_editor.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    /// Show the dialog without focusing.
    pub fn show_visible(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        self.visible = true;
        self.name_editor.update(cx, |e, cx| e.clear(cx));
        self.path_editor.update(cx, |e, cx| e.clear(cx));
        self.branch_editor.update(cx, |e, cx| e.clear(cx));
        if let Some(b) = branch {
            self.branch_editor.update(cx, |e, cx| e.set_text(&b, cx));
        }
        self.error_message = None;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.name_editor.update(cx, |e, cx| e.clear(cx));
        self.path_editor.update(cx, |e, cx| e.clear(cx));
        self.branch_editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(WorktreeDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn validate_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Worktree name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Worktree name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('/') {
            return Some("Worktree name cannot start with '.' or '/'".to_string());
        }
        if name.contains("//") {
            return Some("Worktree name cannot contain consecutive slashes".to_string());
        }
        None
    }

    fn validate_path(path: &str) -> Option<String> {
        if path.is_empty() {
            return Some("Directory path cannot be empty".to_string());
        }
        None
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        let name = self.name_editor.read(cx).text().to_string();
        let path = self.path_editor.read(cx).text().to_string();
        let branch_text = self.branch_editor.read(cx).text().to_string();
        let branch = if branch_text.is_empty() {
            None
        } else {
            Some(branch_text)
        };

        // Validate name
        if let Some(err) = Self::validate_name(&name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        // Validate path
        if let Some(err) = Self::validate_path(&path) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        self.visible = false;
        self.name_editor.update(cx, |e, cx| e.clear(cx));
        self.path_editor.update(cx, |e, cx| e.clear(cx));
        self.branch_editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(WorktreeDialogEvent::CreateWorktree { name, path, branch });
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.as_str() == "escape" {
            self.dismiss(cx);
        }
    }
}

impl Render for WorktreeDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("worktree-dialog").into_any_element();
        }

        let colors = cx.colors();
        let name_text = self.name_editor.read(cx).text().to_string();
        let path_text = self.path_editor.read(cx).text().to_string();
        let has_error = self.error_message.is_some();
        let can_create = !name_text.is_empty() && !path_text.is_empty() && !has_error;

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let mut modal = div()
            .id("worktree-dialog-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(480.))
            .elevation_3(cx)
            .p(px(20.))
            .gap(px(16.))
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        modal = modal.child(
            div()
                .id("worktree-dialog-header")
                .v_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(40.))
                        .h(px(40.))
                        .rounded(px(8.))
                        .bg(icon_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::GitBranch)
                                .color(Color::Custom(accent_color))
                                .size(IconSize::Medium),
                        ),
                )
                .child(
                    Label::new("New Worktree")
                        .size(LabelSize::Large)
                        .color(Color::Default),
                ),
        );

        // Name field
        modal = modal.child(
            div()
                .id("worktree-dialog-name-field")
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Name")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(self.name_editor.clone()),
        );

        // Path field
        modal = modal.child(
            div()
                .id("worktree-dialog-path-field")
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Directory Path")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(self.path_editor.clone()),
        );

        // Branch field
        modal = modal.child(
            div()
                .id("worktree-dialog-branch-field")
                .v_flex()
                .gap_1()
                .child(
                    Label::new("Branch (optional)")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(self.branch_editor.clone()),
        );

        // Error message
        if let Some(ref err) = self.error_message {
            modal = modal.child(
                Label::new(err.clone())
                    .size(LabelSize::Small)
                    .color(Color::Error),
            );
        }

        // Action buttons
        modal = modal.child(
            div()
                .id("worktree-dialog-actions")
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("worktree-dialog-cancel", "Cancel")
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.dismiss(cx);
                        })),
                )
                .child(
                    Button::new("worktree-dialog-create", "Create")
                        .style(if can_create {
                            ButtonStyle::Tinted(TintColor::Accent)
                        } else {
                            ButtonStyle::Subtle
                        })
                        .size(ButtonSize::Compact)
                        .disabled(!can_create)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let name = this.name_editor.read(cx).text().to_string();
                            let path = this.path_editor.read(cx).text().to_string();
                            let branch_text = this.branch_editor.read(cx).text().to_string();
                            let branch = if branch_text.is_empty() {
                                None
                            } else {
                                Some(branch_text)
                            };
                            this.visible = false;
                            this.name_editor.update(cx, |e, cx| e.clear(cx));
                            this.path_editor.update(cx, |e, cx| e.clear(cx));
                            this.branch_editor.update(cx, |e, cx| e.clear(cx));
                            this.error_message = None;
                            cx.emit(WorktreeDialogEvent::CreateWorktree { name, path, branch });
                            cx.notify();
                        })),
                ),
        );

        div()
            .id("worktree-dialog")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::Hsla {
                a: 0.4,
                ..colors.scrollbar_thumb_background
            })
            .when(self.visible, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(modal),
                )
            })
            .into_any_element()
    }
}
