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

/// Events emitted by the tag creation dialog.
#[derive(Debug, Clone)]
pub enum TagDialogEvent {
    CreateTag { name: String, target_oid: git2::Oid },
    Dismissed,
}

/// A modal dialog for creating a new Git tag at a specific commit.
pub struct TagDialog {
    editor: Entity<TextInput>,
    target_oid: Option<git2::Oid>,
    target_sha_short: String,
    error_message: Option<String>,
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<TagDialogEvent> for TagDialog {}

impl TagDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("v1.0.0");
            ti
        });
        cx.subscribe(
            &editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submit => {
                    this.try_create(cx);
                }
                TextInputEvent::Changed(text) => {
                    this.error_message = if text.is_empty() {
                        None
                    } else {
                        Self::validate_tag_name(text)
                    };
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            editor,
            target_oid: None,
            target_sha_short: String::new(),
            error_message: None,
            visible: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog for creating a tag at the given commit.
    pub fn show_visible(&mut self, target_oid: git2::Oid, cx: &mut Context<Self>) {
        self.visible = true;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        self.target_sha_short = target_oid.to_string()[..7].to_string();
        self.target_oid = Some(target_oid);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.editor.update(cx, |e, cx| e.clear(cx));
        self.error_message = None;
        self.target_oid = None;
        cx.emit(TagDialogEvent::Dismissed);
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn validate_tag_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Tag name cannot be empty".to_string());
        }
        if name.contains(' ') {
            return Some("Tag name cannot contain spaces".to_string());
        }
        if name.starts_with('.') || name.starts_with('-') {
            return Some("Tag name cannot start with '.' or '-'".to_string());
        }
        if name.ends_with('.') || name.ends_with('/') {
            return Some("Tag name cannot end with '.' or '/'".to_string());
        }
        if name.contains("..") {
            return Some("Tag name cannot contain '..'".to_string());
        }
        if name.contains('~') || name.contains('^') || name.contains(':') || name.contains('\\') {
            return Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string());
        }
        if name.contains('?') || name.contains('*') || name.contains('[') {
            return Some("Tag name cannot contain glob characters".to_string());
        }
        if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
            return Some("Tag name cannot contain control characters".to_string());
        }
        if name.contains("@{") {
            return Some("Tag name cannot contain '@{'".to_string());
        }
        if name.contains("//") {
            return Some("Tag name cannot contain consecutive slashes".to_string());
        }
        if name.ends_with(".lock") {
            return Some("Tag name cannot end with '.lock'".to_string());
        }
        None
    }

    fn try_create(&mut self, cx: &mut Context<Self>) {
        let tag_name = self.editor.read(cx).text().to_string();
        if let Some(err) = Self::validate_tag_name(&tag_name) {
            self.error_message = Some(err);
            cx.notify();
            return;
        }

        if let Some(oid) = self.target_oid {
            self.visible = false;
            self.editor.update(cx, |e, cx| e.clear(cx));
            self.error_message = None;
            self.target_oid = None;
            cx.emit(TagDialogEvent::CreateTag {
                name: tag_name,
                target_oid: oid,
            });
            cx.notify();
        }
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

impl Render for TagDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("tag-dialog").into_any_element();
        }

        let colors = cx.colors();
        let tag_name = self.editor.read(cx).text().to_string();
        let has_error = self.error_message.is_some();
        let can_create = !tag_name.is_empty() && !has_error;

        let accent_color = Color::Accent.color(cx);
        let icon_bg = gpui::Hsla {
            a: 0.12,
            ..accent_color
        };

        let target_sha: SharedString = self.target_sha_short.clone().into();

        let mut modal = div()
            .id("tag-dialog-modal")
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
                            Icon::new(IconName::Tag)
                                .size(IconSize::Medium)
                                .color(Color::Accent),
                        ),
                )
                .child(
                    Label::new("Create Tag")
                        .size(LabelSize::Large)
                        .weight(gpui::FontWeight::BOLD),
                ),
        );

        modal = modal.child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(
                    Label::new("Tag name")
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
                            Icon::new(IconName::Tag)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(div().flex_1().child(self.editor.clone())),
                ),
        );

        modal = modal.child(
            div()
                .h_flex()
                .gap(px(8.))
                .items_center()
                .child(
                    Label::new("At commit")
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
                            Label::new(target_sha)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::MEDIUM)
                                .color(Color::Accent),
                        ),
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
                    Label::new("Enter to create | Esc to cancel")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("cancel-tag", "Cancel")
                                .size(ButtonSize::Default)
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            Button::new("create-tag", "Create Tag")
                                .icon(IconName::Tag)
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
            .id("tag-dialog-backdrop")
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
    fn validate_tag_name_accepts_simple_name() {
        assert!(TagDialog::validate_tag_name("v1.0.0").is_none());
    }

    #[test]
    fn validate_tag_name_accepts_version_tag() {
        assert!(TagDialog::validate_tag_name("1.2.3").is_none());
    }

    #[test]
    fn validate_tag_name_accepts_release_name() {
        assert!(TagDialog::validate_tag_name("release-candidate").is_none());
    }

    #[test]
    fn validate_tag_name_accepts_underscore() {
        assert!(TagDialog::validate_tag_name("tag_name").is_none());
    }

    #[test]
    fn validate_tag_name_accepts_single_char() {
        assert!(TagDialog::validate_tag_name("v").is_none());
    }

    #[test]
    fn validate_tag_name_accepts_dot_separated() {
        assert!(TagDialog::validate_tag_name("foo.bar").is_none());
    }

    // ── empty ────────────────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_empty() {
        assert_eq!(
            TagDialog::validate_tag_name(""),
            Some("Tag name cannot be empty".to_string())
        );
    }

    // ── spaces ───────────────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_spaces() {
        assert_eq!(
            TagDialog::validate_tag_name("tag name"),
            Some("Tag name cannot contain spaces".to_string())
        );
    }

    // ── leading dot / dash ───────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_leading_dot() {
        assert_eq!(
            TagDialog::validate_tag_name(".name"),
            Some("Tag name cannot start with '.' or '-'".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_leading_dash() {
        assert_eq!(
            TagDialog::validate_tag_name("-name"),
            Some("Tag name cannot start with '.' or '-'".to_string())
        );
    }

    // ── trailing dot / slash ─────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_trailing_dot() {
        assert_eq!(
            TagDialog::validate_tag_name("name."),
            Some("Tag name cannot end with '.' or '/'".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_trailing_slash() {
        assert_eq!(
            TagDialog::validate_tag_name("name/"),
            Some("Tag name cannot end with '.' or '/'".to_string())
        );
    }

    // ── double dots ─────────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_double_dots() {
        assert_eq!(
            TagDialog::validate_tag_name("na..me"),
            Some("Tag name cannot contain '..'".to_string())
        );
    }

    // ── double slashes ──────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_double_slashes() {
        assert_eq!(
            TagDialog::validate_tag_name("na//me"),
            Some("Tag name cannot contain consecutive slashes".to_string())
        );
    }

    // ── git ref characters ──────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_tilde() {
        assert_eq!(
            TagDialog::validate_tag_name("na~me"),
            Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_caret() {
        assert_eq!(
            TagDialog::validate_tag_name("na^me"),
            Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_colon() {
        assert_eq!(
            TagDialog::validate_tag_name("na:me"),
            Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_backslash() {
        assert_eq!(
            TagDialog::validate_tag_name("na\\me"),
            Some("Tag name cannot contain '~', '^', ':', or '\\'".to_string())
        );
    }

    // ── glob characters ─────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_question_mark() {
        assert_eq!(
            TagDialog::validate_tag_name("na?me"),
            Some("Tag name cannot contain glob characters".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_asterisk() {
        assert_eq!(
            TagDialog::validate_tag_name("na*me"),
            Some("Tag name cannot contain glob characters".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_bracket() {
        assert_eq!(
            TagDialog::validate_tag_name("na[me"),
            Some("Tag name cannot contain glob characters".to_string())
        );
    }

    // ── control characters ──────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_del_char() {
        assert_eq!(
            TagDialog::validate_tag_name("na\x7fme"),
            Some("Tag name cannot contain control characters".to_string())
        );
    }

    #[test]
    fn validate_tag_name_rejects_embedded_null() {
        assert_eq!(
            TagDialog::validate_tag_name("na\0me"),
            Some("Tag name cannot contain control characters".to_string())
        );
    }

    // ── @{-syntax ───────────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_at_curly() {
        assert_eq!(
            TagDialog::validate_tag_name("na@{me"),
            Some("Tag name cannot contain '@{'".to_string())
        );
    }

    // ── .lock suffix ─────────────────────────────────────────────

    #[test]
    fn validate_tag_name_rejects_lock_suffix() {
        assert_eq!(
            TagDialog::validate_tag_name("name.lock"),
            Some("Tag name cannot end with '.lock'".to_string())
        );
    }
}
