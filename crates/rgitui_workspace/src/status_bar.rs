use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

/// The bottom status bar.
#[derive(IntoElement)]
pub struct StatusBar {
    branch_name: SharedString,
    ahead: usize,
    behind: usize,
    staged_count: usize,
    unstaged_count: usize,
    operation_message: Option<String>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            branch_name: "".into(),
            ahead: 0,
            behind: 0,
            staged_count: 0,
            unstaged_count: 0,
            operation_message: None,
        }
    }

    pub fn branch(mut self, name: impl Into<SharedString>) -> Self {
        self.branch_name = name.into();
        self
    }

    pub fn ahead_behind(mut self, ahead: usize, behind: usize) -> Self {
        self.ahead = ahead;
        self.behind = behind;
        self
    }

    pub fn changes(mut self, staged: usize, unstaged: usize) -> Self {
        self.staged_count = staged;
        self.unstaged_count = unstaged;
        self
    }

    pub fn operation_message(mut self, msg: impl Into<String>) -> Self {
        self.operation_message = Some(msg.into());
        self
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        let mut bar = div()
            .h_flex()
            .w_full()
            .h(px(26.))
            .bg(colors.status_bar_background)
            .border_t_1()
            .border_color(colors.border_variant)
            .px(px(12.))
            .gap(px(12.))
            .items_center();

        // Branch name with icon
        let has_branch = !self.branch_name.is_empty();
        if has_branch {
            bar = bar.child(
                div()
                    .h_flex()
                    .gap(px(4.))
                    .items_center()
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new(self.branch_name)
                            .size(LabelSize::XSmall)
                            .color(Color::Accent)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            );
        }

        // Ahead/behind with individual indicators
        if self.ahead > 0 || self.behind > 0 {
            bar = bar.child(
                div()
                    .h_flex()
                    .gap(px(6.))
                    .items_center()
                    .when(self.ahead > 0, |el| {
                        let ahead_text: SharedString = format!("{}", self.ahead).into();
                        el.child(
                            div()
                                .h_flex()
                                .gap(px(2.))
                                .items_center()
                                .child(
                                    Icon::new(IconName::ArrowUp)
                                        .size(IconSize::XSmall)
                                        .color(Color::Success),
                                )
                                .child(
                                    Label::new(ahead_text)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Success),
                                ),
                        )
                    })
                    .when(self.behind > 0, |el| {
                        let behind_text: SharedString = format!("{}", self.behind).into();
                        el.child(
                            div()
                                .h_flex()
                                .gap(px(2.))
                                .items_center()
                                .child(
                                    Icon::new(IconName::ArrowDown)
                                        .size(IconSize::XSmall)
                                        .color(Color::Warning),
                                )
                                .child(
                                    Label::new(behind_text)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                ),
                        )
                    }),
            );
        }

        // Separator
        if has_branch {
            bar = bar.child(div().w(px(1.)).h(px(14.)).bg(colors.border_variant));
        }

        // Operation message
        if let Some(msg) = self.operation_message {
            bar = bar.child(
                Label::new(SharedString::from(msg))
                    .size(LabelSize::XSmall)
                    .color(Color::Info),
            );
        }

        // Spacer
        bar = bar.child(div().flex_1());

        // Right-aligned: Changes in pill badges
        if self.staged_count > 0 {
            let staged_text: SharedString = format!("{} staged", self.staged_count).into();
            bar = bar.child(
                div()
                    .h_flex()
                    .h(px(18.))
                    .px(px(6.))
                    .gap(px(3.))
                    .items_center()
                    .rounded(px(3.))
                    .bg(colors.ghost_element_hover)
                    .child(
                        Icon::new(IconName::Check)
                            .size(IconSize::XSmall)
                            .color(Color::Added),
                    )
                    .child(
                        Label::new(staged_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Added),
                    ),
            );
        }
        if self.unstaged_count > 0 {
            let unstaged_text: SharedString = format!("{} changed", self.unstaged_count).into();
            bar = bar.child(
                div()
                    .h_flex()
                    .h(px(18.))
                    .px(px(6.))
                    .gap(px(3.))
                    .items_center()
                    .rounded(px(3.))
                    .bg(colors.ghost_element_hover)
                    .child(
                        Icon::new(IconName::Edit)
                            .size(IconSize::XSmall)
                            .color(Color::Modified),
                    )
                    .child(
                        Label::new(unstaged_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Modified),
                    ),
            );
        }

        // Clean indicator when no changes
        if self.staged_count == 0 && self.unstaged_count == 0 && has_branch {
            bar = bar.child(
                Label::new("Clean")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }

        bar
    }
}
