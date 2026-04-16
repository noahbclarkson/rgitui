//! Dialog for creating a branch from a stash entry.
//!
//! Implements `git stash branch <branchname>` — creates a new branch at the stash's
//! commit, then applies the stash.

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput,
    TextInputEvent,
};

/// Events emitted by the stash branch dialog.
#[derive(Debug, Clone)]
pub enum StashBranchDialogEvent {
    /// Create a branch from the stash with the given name.
    CreateBranch {
        name: String,
        stash_index: usize,
    },
    Dismissed,
}

/// A modal dialog for creating a branch from a stash entry.
pub struct StashBranchDialog {
    editor: Entity<TextInput>,
    stash_index: usize,
    error_message: Option<String>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<StashBranchDialogEvent> for StashBranchDialog {}

impl StashBranchDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Enter branch name...");
            ti
        });
        cx.subscribe(
            &editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_create(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = Self::validate_branch_name(text);
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            editor,
            stash_index: 0,
            error_message: None,
            visible: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog for creating a branch from the stash at the given index.
    pub fn show(&mut self, stash_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.stash_index = stash_index;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        self.editor.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    /// Show the dialog without focusing (for use where Window is unavailable).
    pub fn show_visible(&mut self, stash_index: usize, cx: &mut Context<Self>) {
        self.visible = true;
        self.stash_index = stash_index;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(StashBranchDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn validate_branch_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Branch name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Branch name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Branch name cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Branch name cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") {
            return Some("Branch name cannot contain '..'".to_string());
        }
        if name.contains('~') || name.contains('^') || name.contains(':') || name.contains('\\') {
            return Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string());
        }
        if name.contains('?') || name.contains('*') || name.contains('[') {
            return Some("Branch name cannot contain glob characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Branch name cannot contain control characters".to_string());
        }
        if name.contains("@{") {
            return Some("Branch name cannot contain '@{'".to_string());
        }
        if name == "@" {
            return Some("Branch name cannot be '@'".to_string());
        }
        if name.contains("//") {
            return Some("Branch name cannot contain consecutive slashes".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Branch name cannot end with '.lock'".to_string());
        }
        None
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key.as_str() == "escape" {
            self.dismiss(cx);
        }
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        let branch_name = self.editor.read(cx).text().to_string();
        if let Some(err) = Self::validate_branch_name(&branch_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }
        let name = branch_name.trim();
        if name.is_empty() {
            return;
        }
        let idx = self.stash_index;
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(StashBranchDialogEvent::CreateBranch {
            name: name.to_string(),
            stash_index: idx,
        });
        cx.notify();
    }
}

impl Render for StashBranchDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }

        let colors = cx.colors();
        let branch_name = self.editor.read(cx).text().to_string();
        let has_error = self.error_message.is_some();
        let can_create = !branch_name.is_empty() && !has_error;

        let stash_ref_str: SharedString = if self.stash_index == 0 {
            "stash@{0}".to_string().into()
        } else {
            format!("stash@{{{}}}", self.stash_index).into()
        };

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let mut modal = div()
            .id("stash-branch-dialog-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(440.))
            .elevation_3(cx)
            .p(px(20.))
            .gap(px(16.))
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            });

        // Header
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
                    Label::new("Create Branch from Stash")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD),
                ),
        );

        // Stash reference display
        modal = modal.child(
            div()
                .h_flex()
                .gap(px(6.))
                .items_center()
                .child(
                    Label::new("From")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .h(px(22.))
                        .px(px(8.))
                        .gap(px(4.))
                        .rounded(px(6.))
                        .bg(colors.ghost_element_selected)
                        .items_center()
                        .child(
                            Icon::new(IconName::GitCommit)
                                .size(IconSize::XSmall)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(stash_ref_str)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::MEDIUM)
                                .color(Color::Accent),
                        ),
                ),
        );

        // Branch name input
        modal = modal.child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(
                    Label::new("Branch name")
                        .size(LabelSize::Small)
                        .weight(gpui::FontWeight::MEDIUM)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(div().flex_1().child(self.editor.clone())),
                ),
        );

        // Error message
        if let Some(ref err) = self.error_message {
            modal = modal.child(
                div()
                    .h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        Icon::new(IconName::XCircle)
                            .size(IconSize::XSmall)
                            .color(Color::Error),
                    )
                    .child(
                        Label::new(SharedString::from(err.clone()))
                            .size(LabelSize::XSmall)
                            .color(Color::Error),
                    ),
            );
        }

        // Footer
        modal = modal.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(colors.border_variant)
                .h_flex()
                .justify_between()
                .items_center()
                .child(
                    Label::new("Enter to create | Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("cancel-stash-branch", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("create-stash-branch", "Create Branch")
                                .icon(IconName::GitBranch)
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Filled)
                                .color(Color::Accent)
                                .disabled(!can_create)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.try_create(cx);
                                })),
                        ),
                ),
        );

        div()
            .id("stash-branch-dialog-backdrop")
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

#[cfg(test)]
mod tests {
    use super::StashBranchDialog;

    // --- validate_branch_name tests ---

    #[test]
    fn validate_branch_name_valid_simple() {
        assert!(StashBranchDialog::validate_branch_name("main").is_none());
        assert!(StashBranchDialog::validate_branch_name("feature-xyz").is_none());
        assert!(StashBranchDialog::validate_branch_name("feature_xyz").is_none());
        assert!(StashBranchDialog::validate_branch_name("user/feature").is_none());
    }

    #[test]
    fn validate_branch_name_empty_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name(""),
            Some("Branch name cannot be empty".to_string())
        );
    }

    #[test]
    fn validate_branch_name_spaces_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature branch"),
            Some("Branch name cannot contain spaces".to_string())
        );
    }

    #[test]
    fn validate_branch_name_starts_with_dot_or_hyphen_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name(".hidden"),
            Some("Branch name cannot start with '.' or '-'".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("-feature"),
            Some("Branch name cannot start with '.' or '-'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_ends_with_dot_or_slash_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature."),
            Some("Branch name cannot end with '.' or '/'".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature/"),
            Some("Branch name cannot end with '.' or '/'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_double_dot_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature..main"),
            Some("Branch name cannot contain '..'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_tilde_caret_colon_backslash_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature~1"),
            Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature^1"),
            Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("origin:feature"),
            Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat\\ure"),
            Some("Branch name cannot contain '~', '^', ':', or '\\'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_glob_chars_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat?ure"),
            Some("Branch name cannot contain glob characters".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat*ure"),
            Some("Branch name cannot contain glob characters".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat[ure]"),
            Some("Branch name cannot contain glob characters".to_string())
        );
    }

    #[test]
    fn validate_branch_name_control_char_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat\x7fure"),
            Some("Branch name cannot contain control characters".to_string())
        );
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat\x00ure"),
            Some("Branch name cannot contain control characters".to_string())
        );
    }

    #[test]
    fn validate_branch_name_at_brace_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feat@{ure"),
            Some("Branch name cannot contain '@{'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_at_alone_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("@"),
            Some("Branch name cannot be '@'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_consecutive_slashes_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("user//feature"),
            Some("Branch name cannot contain consecutive slashes".to_string())
        );
    }

    #[test]
    fn validate_branch_name_lock_suffix_returns_error() {
        assert_eq!(
            StashBranchDialog::validate_branch_name("feature.lock"),
            Some("Branch name cannot end with '.lock'".to_string())
        );
    }

    #[test]
    fn validate_branch_name_unicode_valid() {
        assert!(StashBranchDialog::validate_branch_name("feature-日本語").is_none());
        assert!(StashBranchDialog::validate_branch_name("功能分支").is_none());
        assert!(StashBranchDialog::validate_branch_name("branche-française").is_none());
    }

    #[test]
    fn validate_branch_name_numbers_and_hyphens_valid() {
        assert!(StashBranchDialog::validate_branch_name("v1.0.0").is_none());
        assert!(StashBranchDialog::validate_branch_name("release-2024-01").is_none());
        assert!(StashBranchDialog::validate_branch_name("feature-123").is_none());
    }

    #[test]
    fn validate_branch_name_path_like_valid() {
        assert!(StashBranchDialog::validate_branch_name("user/feature").is_none());
        assert!(StashBranchDialog::validate_branch_name("owner/sub/feature").is_none());
        assert!(StashBranchDialog::validate_branch_name("a/b/c/d/e").is_none());
    }

    // --- StashBranchDialogEvent tests ---

    #[test]
    fn stash_branch_dialog_event_debug() {
        let event = super::StashBranchDialogEvent::Dismissed;
        assert_eq!(format!("{:?}", event), "Dismissed");
    }

    #[test]
    fn stash_branch_dialog_event_create_branch() {
        let event = super::StashBranchDialogEvent::CreateBranch {
            name: "feature-x".to_string(),
            stash_index: 2,
        };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("CreateBranch"));
        assert!(debug_str.contains("feature-x"));
        assert!(debug_str.contains("2"));
    }

    #[test]
    fn stash_branch_dialog_event_clone() {
        let event = super::StashBranchDialogEvent::CreateBranch {
            name: "test".to_string(),
            stash_index: 5,
        };
        let cloned = event.clone();
        if let super::StashBranchDialogEvent::CreateBranch { name, stash_index } = cloned {
            assert_eq!(name, "test");
            assert_eq!(stash_index, 5);
        } else {
            panic!("Clone should produce CreateBranch variant");
        }
    }
}
