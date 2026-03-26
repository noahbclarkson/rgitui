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

/// Events emitted by the rename dialog.
#[derive(Debug, Clone)]
pub enum RenameDialogEvent {
    Rename { old_name: String, new_name: String },
    Dismissed,
}

/// A modal dialog for renaming a Git branch.
pub struct RenameDialog {
    old_name: String,
    editor: Entity<TextInput>,
    error_message: Option<String>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<RenameDialogEvent> for RenameDialog {}

impl RenameDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("new-branch-name");
            ti
        });
        cx.subscribe(
            &editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_rename(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = if text.is_empty() {
                        None
                    } else {
                        Self::validate(text)
                    };
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            old_name: String::new(),
            editor,
            error_message: None,
            visible: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog for renaming the given branch.
    pub fn show_visible(&mut self, old_name: String, cx: &mut Context<Self>) {
        self.old_name = old_name.clone();
        self.editor.update(cx, |e, cx| e.set_text(old_name, cx));
        self.error_message = None;
        self.visible = true;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.old_name.clear();
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(RenameDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn validate(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Branch name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Branch name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") || name.contains("//") {
            return Some("Cannot contain '..' or '//'".to_string());
        }
        if name.contains('~')
            || name.contains('^')
            || name.contains(':')
            || name.contains('\\')
            || name.contains('?')
            || name.contains('*')
            || name.contains('[')
        {
            return Some("Contains invalid characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Contains control characters".to_string());
        }
        if name.contains("@{") || name == "@" {
            return Some("Invalid ref name".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Cannot end with '.lock'".to_string());
        }
        None
    }

    fn try_rename(&mut self, cx: &mut Context<Self>) {
        let new_name = self.editor.read(cx).text().to_string();
        if new_name == self.old_name {
            self.dismiss(cx);
            return;
        }
        if let Some(err) = Self::validate(&new_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        let old = self.old_name.clone();
        self.visible = false;
        self.old_name.clear();
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        cx.emit(RenameDialogEvent::Rename {
            old_name: old,
            new_name,
        });
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

impl Render for RenameDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("rename-dialog").into_any_element();
        }

        let colors = cx.colors();
        let new_name = self.editor.read(cx).text().to_string();
        let is_empty = new_name.is_empty();
        let has_error = self.error_message.is_some();
        let is_unchanged = new_name == self.old_name;
        let can_rename = !is_empty && !has_error && !is_unchanged;

        let old_name: SharedString = self.old_name.clone().into();

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let mut modal = div()
            .id("rename-dialog-modal")
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
                            Icon::new(IconName::Edit)
                                .size(IconSize::Medium)
                                .color(Color::Accent),
                        ),
                )
                .child(
                    Label::new("Rename Branch")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD),
                ),
        );

        modal = modal.child(
            div()
                .h_flex()
                .gap(px(8.))
                .items_center()
                .child(
                    Label::new("Current name")
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
                            Icon::new(IconName::GitBranch)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(old_name)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::MEDIUM)
                                .color(Color::Muted),
                        ),
                ),
        );

        modal = modal.child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(
                    Label::new("New name")
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

        modal = modal.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(colors.border_variant)
                .h_flex()
                .justify_between()
                .items_center()
                .child(
                    Label::new("Enter to rename · Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("cancel-rename", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("do-rename", "Rename")
                                .icon(IconName::Edit)
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Filled)
                                .color(Color::Accent)
                                .disabled(!can_rename)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.try_rename(cx);
                                })),
                        ),
                ),
        );

        div()
            .id("rename-dialog-backdrop")
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
    use super::*;

    // ── valid names ─────────────────────────────────────────────

    #[test]
    fn validate_accepts_simple_branch_name() {
        assert!(RenameDialog::validate("main").is_none());
    }

    #[test]
    fn validate_accepts_branch_with_slash() {
        assert!(RenameDialog::validate("feature/test").is_none());
    }

    #[test]
    fn validate_accepts_version_tag() {
        assert!(RenameDialog::validate("v1.0.0").is_none());
    }

    #[test]
    fn validate_accepts_underscore_separated() {
        assert!(RenameDialog::validate("branch_name").is_none());
    }

    #[test]
    fn validate_accepts_single_char() {
        assert!(RenameDialog::validate("a").is_none());
    }

    #[test]
    fn validate_accepts_hyphen_separated() {
        assert!(RenameDialog::validate("foo-bar").is_none());
    }

    #[test]
    fn validate_accepts_dot_separated() {
        assert!(RenameDialog::validate("foo.bar").is_none());
    }

    // ── empty ────────────────────────────────────────────────────

    #[test]
    fn validate_rejects_empty() {
        assert_eq!(
            RenameDialog::validate(""),
            Some("Branch name cannot be empty".to_string())
        );
    }

    // ── spaces ───────────────────────────────────────────────────

    #[test]
    fn validate_rejects_spaces() {
        assert_eq!(
            RenameDialog::validate("branch name"),
            Some("Branch name cannot contain spaces".to_string())
        );
    }

    // ── leading dot / dash ───────────────────────────────────────

    #[test]
    fn validate_rejects_leading_dot() {
        assert_eq!(
            RenameDialog::validate(".name"),
            Some("Cannot start with '.' or '-'".to_string())
        );
    }

    #[test]
    fn validate_rejects_leading_dash() {
        assert_eq!(
            RenameDialog::validate("-name"),
            Some("Cannot start with '.' or '-'".to_string())
        );
    }

    // ── trailing dot / slash ─────────────────────────────────────

    #[test]
    fn validate_rejects_trailing_dot() {
        assert_eq!(
            RenameDialog::validate("name."),
            Some("Cannot end with '.' or '/'".to_string())
        );
    }

    #[test]
    fn validate_rejects_trailing_slash() {
        assert_eq!(
            RenameDialog::validate("name/"),
            Some("Cannot end with '.' or '/'".to_string())
        );
    }

    // ── double dots / slashes ───────────────────────────────────

    #[test]
    fn validate_rejects_double_dots() {
        assert_eq!(
            RenameDialog::validate("na..me"),
            Some("Cannot contain '..' or '//'".to_string())
        );
    }

    #[test]
    fn validate_rejects_double_slashes() {
        assert_eq!(
            RenameDialog::validate("na//me"),
            Some("Cannot contain '..' or '//'".to_string())
        );
    }

    // ── invalid git ref characters ─────────────────────────────

    #[test]
    fn validate_rejects_tilde() {
        assert_eq!(
            RenameDialog::validate("na~me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_caret() {
        assert_eq!(
            RenameDialog::validate("na^me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_colon() {
        assert_eq!(
            RenameDialog::validate("na:me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_backslash() {
        assert_eq!(
            RenameDialog::validate("na\\me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_question_mark() {
        assert_eq!(
            RenameDialog::validate("na?me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_asterisk() {
        assert_eq!(
            RenameDialog::validate("na*me"),
            Some("Contains invalid characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_bracket() {
        assert_eq!(
            RenameDialog::validate("na[me"),
            Some("Contains invalid characters".to_string())
        );
    }

    // ── control characters ──────────────────────────────────────

    #[test]
    fn validate_rejects_del_char() {
        assert_eq!(
            RenameDialog::validate("na\x7fme"),
            Some("Contains control characters".to_string())
        );
    }

    #[test]
    fn validate_rejects_embedded_null() {
        // \0 is a control character
        assert_eq!(
            RenameDialog::validate("na\0me"),
            Some("Contains control characters".to_string())
        );
    }

    // ── @ ref syntax ────────────────────────────────────────────

    #[test]
    fn validate_rejects_at_curly() {
        assert_eq!(
            RenameDialog::validate("na@{me"),
            Some("Invalid ref name".to_string())
        );
    }

    #[test]
    fn validate_rejects_at_alone() {
        assert_eq!(
            RenameDialog::validate("@"),
            Some("Invalid ref name".to_string())
        );
    }

    // ── .lock suffix ─────────────────────────────────────────────

    #[test]
    fn validate_rejects_lock_suffix() {
        assert_eq!(
            RenameDialog::validate("name.lock"),
            Some("Cannot end with '.lock'".to_string())
        );
    }
}
