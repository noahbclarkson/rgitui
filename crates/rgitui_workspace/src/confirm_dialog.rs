use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, EventEmitter, FocusHandle, Render, SharedString, Window};
use rgitui_git::RepoState;
use rgitui_i18n::t;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize};

use crate::keymap;
use crate::CommandId;

/// The action that was confirmed (so the workspace knows what to do).
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    DiscardFile(String),
    DiscardAll,
    CleanUntracked,
    ForcePush,
    StashDrop(usize),
    BranchDelete(String),
    TagDelete(String),
    RemoveRemote(String),
    ResetHard(String),
    ResetSoft(String),
    ResetMixed(String),
    /// Abort the merge, rebase or other operation the repository is in.
    AbortOperation(RepoState),
    WorktreeRemove(String),
}

/// Events emitted by the confirmation dialog.
#[derive(Debug, Clone)]
pub enum ConfirmDialogEvent {
    Confirmed(ConfirmAction),
    Cancelled,
}

/// A modal confirmation dialog for destructive operations. Its title and
/// warning come from the action and are translated when drawn.
pub struct ConfirmDialog {
    visible: bool,
    action: Option<ConfirmAction>,
    focus_handle: FocusHandle,
    pending_focus: bool,
}

impl EventEmitter<ConfirmDialogEvent> for ConfirmDialog {}

impl ConfirmDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            action: None,
            focus_handle: cx.focus_handle(),
            pending_focus: false,
        }
    }

    /// Ask to confirm `action`. Focus moves to the dialog on its next draw.
    pub fn show_visible(&mut self, action: ConfirmAction, cx: &mut Context<Self>) {
        self.action = Some(action);
        self.visible = true;
        self.pending_focus = true;
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(action) = self.action.take() {
            self.visible = false;
            cx.emit(ConfirmDialogEvent::Confirmed(action));
            cx.notify();
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.action = None;
        cx.emit(ConfirmDialogEvent::Cancelled);
        cx.notify();
    }

    /// Runs a keyboard command scoped to `ConfirmDialog`.
    fn dispatch_command(&mut self, cmd: CommandId, _window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            CommandId::Cancel => self.cancel(cx),
            CommandId::Confirm => self.confirm(cx),
            _ => cx.propagate(),
        }
    }

    fn is_destructive(&self) -> bool {
        matches!(
            &self.action,
            Some(
                ConfirmAction::DiscardFile(_)
                    | ConfirmAction::DiscardAll
                    | ConfirmAction::CleanUntracked
                    | ConfirmAction::BranchDelete(_)
                    | ConfirmAction::TagDelete(_)
                    | ConfirmAction::RemoveRemote(_)
                    | ConfirmAction::StashDrop(_)
                    | ConfirmAction::ResetHard(_)
                    | ConfirmAction::AbortOperation(_)
                    | ConfirmAction::WorktreeRemove(_)
            )
        )
    }

    fn severity_icon(&self) -> IconName {
        match &self.action {
            Some(ConfirmAction::ForcePush) => IconName::AlertTriangle,
            Some(_) => IconName::Trash,
            None => IconName::Check,
        }
    }

    fn severity_color(&self) -> Color {
        match &self.action {
            Some(ConfirmAction::ForcePush) => Color::Warning,
            Some(_) if self.is_destructive() => Color::Error,
            _ => Color::Accent,
        }
    }

    /// The confirm button's label for `action`, in the active language.
    fn confirm_label(action: &Option<ConfirmAction>) -> SharedString {
        match action {
            Some(ConfirmAction::DiscardFile(_) | ConfirmAction::DiscardAll) => {
                t!("confirm.discard")
            }
            Some(ConfirmAction::CleanUntracked) => t!("confirm.clean"),
            Some(ConfirmAction::BranchDelete(_)) => t!("confirm.delete_branch"),
            Some(ConfirmAction::TagDelete(_)) => t!("confirm.delete_tag"),
            Some(ConfirmAction::RemoveRemote(_)) => t!("confirm.remove"),
            Some(ConfirmAction::StashDrop(_)) => t!("confirm.drop_stash"),
            Some(ConfirmAction::ResetHard(_)) => t!("confirm.reset"),
            Some(ConfirmAction::ResetSoft(_)) => t!("confirm.reset"),
            Some(ConfirmAction::ResetMixed(_)) => t!("confirm.reset"),
            Some(ConfirmAction::AbortOperation(_)) => t!("common.abort"),
            Some(ConfirmAction::ForcePush) => t!("confirm.force_push"),
            Some(ConfirmAction::WorktreeRemove(_)) => t!("confirm.remove_worktree"),
            None => t!("confirm.confirm"),
        }
    }

    /// The title and warning for `action`, in the active language.
    fn text(action: &ConfirmAction) -> (SharedString, SharedString) {
        match action {
            ConfirmAction::DiscardFile(path) => (
                t!("confirm.title.discard_file"),
                t!("confirm.body.discard_file", path = path),
            ),
            ConfirmAction::DiscardAll => (
                t!("confirm.title.discard_all"),
                t!("confirm.body.discard_all"),
            ),
            ConfirmAction::CleanUntracked => (
                t!("confirm.title.clean_untracked"),
                t!("confirm.body.clean_untracked"),
            ),
            ConfirmAction::ForcePush => (
                t!("confirm.title.force_push"),
                t!("confirm.body.force_push"),
            ),
            ConfirmAction::StashDrop(index) => (
                t!("confirm.title.drop_stash"),
                t!(
                    "confirm.body.drop_stash",
                    stash = format!("stash@{{{index}}}")
                ),
            ),
            ConfirmAction::BranchDelete(name) => (
                t!("confirm.title.delete_branch"),
                t!("confirm.body.delete_branch", branch = name),
            ),
            ConfirmAction::TagDelete(name) => (
                t!("confirm.title.delete_tag"),
                t!("confirm.body.delete_tag", tag = name),
            ),
            ConfirmAction::RemoveRemote(name) => (
                t!("confirm.title.remove_remote"),
                t!("confirm.body.remove_remote", remote = name),
            ),
            ConfirmAction::ResetHard(target) if target == "HEAD" => (
                t!("confirm.title.reset_hard"),
                t!("confirm.body.reset_hard_head"),
            ),
            ConfirmAction::ResetHard(target) => (
                t!("confirm.title.reset_hard"),
                t!("confirm.body.reset_hard", commit = short_target(target)),
            ),
            ConfirmAction::ResetSoft(target) => (
                t!("confirm.title.reset_soft"),
                t!("confirm.body.reset_soft", commit = short_target(target)),
            ),
            ConfirmAction::ResetMixed(target) => (
                t!("confirm.title.reset_mixed"),
                t!("confirm.body.reset_mixed", commit = short_target(target)),
            ),
            ConfirmAction::AbortOperation(state) => (abort_title(*state), t!("confirm.body.abort")),
            ConfirmAction::WorktreeRemove(path) => (
                t!("confirm.title.remove_worktree"),
                t!("confirm.body.remove_worktree", path = path),
            ),
        }
    }
}

/// A commit id cut to the seven characters the graph shows.
fn short_target(target: &str) -> &str {
    target.get(..7).unwrap_or(target)
}

fn abort_title(state: RepoState) -> SharedString {
    match state {
        RepoState::Merge => t!("confirm.title.abort_merge"),
        RepoState::Rebase | RepoState::RebaseInteractive | RepoState::RebaseMerge => {
            t!("confirm.title.abort_rebase")
        }
        RepoState::CherryPick | RepoState::CherryPickSequence => {
            t!("confirm.title.abort_cherry_pick")
        }
        RepoState::Revert | RepoState::RevertSequence => t!("confirm.title.abort_revert"),
        RepoState::Bisect => t!("confirm.title.abort_bisect"),
        RepoState::ApplyMailbox | RepoState::ApplyMailboxOrRebase => {
            t!("confirm.title.abort_apply")
        }
        RepoState::Clean => t!("confirm.title.abort"),
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("confirm-dialog").into_any_element();
        }

        if self.pending_focus {
            self.pending_focus = false;
            self.focus_handle.focus(window, cx);
        }

        let colors = cx.colors().clone();
        let (title, message) = self.action.as_ref().map(Self::text).unwrap_or_default();
        let icon = self.severity_icon();
        let color = self.severity_color();
        let confirm_label = Self::confirm_label(&self.action);
        let is_destructive = self.is_destructive();

        let icon_bg = color.color(cx);
        let icon_bg_subtle = gpui::Hsla { a: 0.12, ..icon_bg };

        div()
            .id("confirm-dialog-backdrop")
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
                this.cancel(cx);
            }))
            .child(
                div()
                    .id("confirm-dialog-modal")
                    .track_focus(&self.focus_handle)
                    .map(|el| {
                        keymap::bind_actions(
                            el,
                            "ConfirmDialog",
                            &["Menu"],
                            cx,
                            Self::dispatch_command,
                        )
                    })
                    .v_flex()
                    .w(px(420.))
                    .elevation_3(cx)
                    .p(px(20.))
                    .gap(px(16.))
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
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
                                    .bg(icon_bg_subtle)
                                    .child(Icon::new(icon).size(IconSize::Medium).color(color)),
                            )
                            .child(
                                Label::new(title)
                                    .size(LabelSize::Large)
                                    .weight(gpui::FontWeight::BOLD),
                            ),
                    )
                    .child(
                        div().pl(px(48.)).child(
                            Label::new(message)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                    )
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(colors.border_variant)
                            .v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                Label::new(t!("common.confirm_hint"))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                            )
                            .child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .flex_nowrap()
                                    .justify_end()
                                    .w_full()
                                    .child(
                                        Button::new("confirm-cancel", t!("common.cancel"))
                                            .size(ButtonSize::Default)
                                            .style(ButtonStyle::Subtle)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.cancel(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("confirm-ok", confirm_label)
                                            .icon(icon)
                                            .size(ButtonSize::Default)
                                            .style(if is_destructive {
                                                ButtonStyle::Tinted(rgitui_ui::TintColor::Error)
                                            } else {
                                                ButtonStyle::Filled
                                            })
                                            .color(color)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.confirm(cx);
                                                },
                                            )),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ConfirmAction

    #[test]
    fn confirm_action_all_variants_clone() {
        let actions = vec![
            ConfirmAction::DiscardFile("foo.txt".into()),
            ConfirmAction::DiscardAll,
            ConfirmAction::CleanUntracked,
            ConfirmAction::ForcePush,
            ConfirmAction::StashDrop(0),
            ConfirmAction::BranchDelete("feature".into()),
            ConfirmAction::TagDelete("v1.0".into()),
            ConfirmAction::RemoveRemote("origin".into()),
            ConfirmAction::ResetHard("HEAD~1".into()),
            ConfirmAction::ResetSoft("HEAD~1".into()),
            ConfirmAction::ResetMixed("HEAD~1".into()),
            ConfirmAction::AbortOperation(RepoState::Merge),
            ConfirmAction::WorktreeRemove("/path/to/worktree".into()),
        ];
        for a in actions {
            let c = a.clone();
            assert_eq!(format!("{:?}", a), format!("{:?}", c));
        }
    }

    #[test]
    fn confirm_action_eq_different_variants() {
        assert!(ConfirmAction::StashDrop(0) != ConfirmAction::StashDrop(1));
        assert!(ConfirmAction::BranchDelete("a".into()) != ConfirmAction::BranchDelete("b".into()));
    }

    // ConfirmDialogEvent

    #[test]
    fn confirm_dialog_event_variants() {
        let confirmed = ConfirmDialogEvent::Confirmed(ConfirmAction::StashDrop(0));
        let cancelled = ConfirmDialogEvent::Cancelled;
        assert_eq!(format!("{:?}", confirmed), "Confirmed(StashDrop(0))");
        assert_eq!(format!("{:?}", cancelled), "Cancelled");
    }

    #[test]
    fn confirm_dialog_event_clone() {
        let evt = ConfirmDialogEvent::Confirmed(ConfirmAction::ForcePush);
        let clone = evt.clone();
        assert_eq!(format!("{:?}", evt), format!("{:?}", clone));
    }

    // is_destructive

    #[test]
    fn is_destructive_true_for_destructive_actions() {
        fn check_destructive(action: &ConfirmAction) -> bool {
            matches!(
                action,
                ConfirmAction::DiscardFile(_)
                    | ConfirmAction::DiscardAll
                    | ConfirmAction::CleanUntracked
                    | ConfirmAction::BranchDelete(_)
                    | ConfirmAction::TagDelete(_)
                    | ConfirmAction::RemoveRemote(_)
                    | ConfirmAction::StashDrop(_)
                    | ConfirmAction::ResetHard(_)
                    | ConfirmAction::AbortOperation(_)
                    | ConfirmAction::WorktreeRemove(_)
            )
        }
        assert!(check_destructive(&ConfirmAction::StashDrop(0)));
        assert!(check_destructive(&ConfirmAction::BranchDelete("x".into())));
        assert!(check_destructive(&ConfirmAction::ResetHard("HEAD".into())));
        assert!(check_destructive(&ConfirmAction::WorktreeRemove(
            "/tmp/wt".into()
        )));
        assert!(!check_destructive(&ConfirmAction::ForcePush));
    }

    // confirm_label

    #[test]
    fn confirm_label_matches_action() {
        let cases: Vec<(Option<ConfirmAction>, &'static str)> = vec![
            (Some(ConfirmAction::DiscardFile("a".into())), "Discard"),
            (Some(ConfirmAction::DiscardAll), "Discard"),
            (Some(ConfirmAction::CleanUntracked), "Clean"),
            (
                Some(ConfirmAction::BranchDelete("x".into())),
                "Delete Branch",
            ),
            (Some(ConfirmAction::TagDelete("v".into())), "Delete Tag"),
            (Some(ConfirmAction::RemoveRemote("o".into())), "Remove"),
            (Some(ConfirmAction::StashDrop(0)), "Drop Stash"),
            (Some(ConfirmAction::ResetHard("HEAD".into())), "Reset"),
            (Some(ConfirmAction::ResetSoft("HEAD".into())), "Reset"),
            (Some(ConfirmAction::ResetMixed("HEAD".into())), "Reset"),
            (
                Some(ConfirmAction::AbortOperation(RepoState::Rebase)),
                "Abort",
            ),
            (Some(ConfirmAction::ForcePush), "Force Push"),
            (
                Some(ConfirmAction::WorktreeRemove("/tmp/wt".into())),
                "Remove Worktree",
            ),
            (None, "Confirm"),
        ];
        for (action, expected) in cases {
            assert_eq!(ConfirmDialog::confirm_label(&action).as_ref(), expected);
        }
    }

    // severity_icon

    #[test]
    fn severity_icon_force_push_is_alert() {
        fn icon_for(action: &ConfirmAction) -> IconName {
            match action {
                ConfirmAction::ForcePush => IconName::AlertTriangle,
                _ => IconName::Trash,
            }
        }
        assert_eq!(icon_for(&ConfirmAction::ForcePush), IconName::AlertTriangle);
        assert_eq!(icon_for(&ConfirmAction::StashDrop(0)), IconName::Trash);
        assert_eq!(
            icon_for(&ConfirmAction::BranchDelete("x".into())),
            IconName::Trash
        );
    }

    // severity_color

    #[test]
    fn severity_color_force_push_is_warning() {
        fn color_for(action: &ConfirmAction) -> Color {
            match action {
                ConfirmAction::ForcePush => Color::Warning,
                a => {
                    if matches!(
                        a,
                        ConfirmAction::DiscardFile(_)
                            | ConfirmAction::DiscardAll
                            | ConfirmAction::CleanUntracked
                            | ConfirmAction::BranchDelete(_)
                            | ConfirmAction::TagDelete(_)
                            | ConfirmAction::RemoveRemote(_)
                            | ConfirmAction::StashDrop(_)
                            | ConfirmAction::ResetHard(_)
                            | ConfirmAction::AbortOperation(_)
                            | ConfirmAction::WorktreeRemove(_)
                    ) {
                        Color::Error
                    } else {
                        Color::Accent
                    }
                }
            }
        }
        assert_eq!(color_for(&ConfirmAction::ForcePush), Color::Warning);
        assert_eq!(color_for(&ConfirmAction::StashDrop(0)), Color::Error);
        assert_eq!(
            color_for(&ConfirmAction::AbortOperation(RepoState::Merge)),
            Color::Error
        );
    }

    // text

    #[test]
    fn text_names_what_the_action_touches() {
        let text = |action| ConfirmDialog::text(&action);
        assert_eq!(
            text(ConfirmAction::StashDrop(2)).1.as_ref(),
            "Are you sure you want to drop stash@{2}? This cannot be undone."
        );
        assert_eq!(
            text(ConfirmAction::ResetHard("HEAD".into())).1.as_ref(),
            "Hard reset to HEAD? All staged and unstaged changes will be permanently discarded."
        );
        assert_eq!(
            text(ConfirmAction::ResetHard("0123456789abcdef".into()))
                .1
                .as_ref(),
            "Hard reset the current branch to 0123456? All uncommitted changes and commits \
             after this point will be lost."
        );
        assert_eq!(
            text(ConfirmAction::AbortOperation(RepoState::RebaseInteractive))
                .0
                .as_ref(),
            "Abort Rebase"
        );
        assert_eq!(
            text(ConfirmAction::BranchDelete("feature".into()))
                .1
                .as_ref(),
            "Are you sure you want to delete branch 'feature'?"
        );
    }
}
