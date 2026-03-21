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
    stash_count: usize,
    operation_message: Option<String>,
    is_loading: bool,
    is_error: bool,
    head_detached: bool,
    repo_state_label: Option<SharedString>,
    repo_path: Option<SharedString>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            branch_name: "".into(),
            ahead: 0,
            behind: 0,
            staged_count: 0,
            unstaged_count: 0,
            stash_count: 0,
            operation_message: None,
            is_loading: false,
            is_error: false,
            head_detached: false,
            repo_state_label: None,
            repo_path: None,
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

    pub fn stash_count(mut self, count: usize) -> Self {
        self.stash_count = count;
        self
    }

    pub fn repo_path(mut self, path: impl Into<SharedString>) -> Self {
        self.repo_path = Some(path.into());
        self
    }

    pub fn operation_message(mut self, msg: impl Into<String>) -> Self {
        self.operation_message = Some(msg.into());
        self
    }

    pub fn loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    pub fn error(mut self, is_error: bool) -> Self {
        self.is_error = is_error;
        self
    }

    pub fn head_detached(mut self, detached: bool) -> Self {
        self.head_detached = detached;
        self
    }

    pub fn repo_state_label(mut self, label: impl Into<SharedString>) -> Self {
        self.repo_state_label = Some(label.into());
        self
    }

    fn render_left_section(&self, colors: &rgitui_theme::ThemeColors) -> gpui::Div {
        let has_branch = !self.branch_name.is_empty();
        let branch_color = if self.head_detached {
            Color::Warning
        } else {
            Color::Accent
        };

        let mut left = div().h_flex().items_center().gap(px(10.));

        if has_branch {
            let mut branch_row = div()
                .h_flex()
                .gap(px(4.))
                .items_center()
                .child(
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::XSmall)
                        .color(branch_color),
                )
                .child(
                    Label::new(self.branch_name.clone())
                        .size(LabelSize::XSmall)
                        .color(branch_color)
                        .weight(gpui::FontWeight::SEMIBOLD),
                );
            if self.head_detached {
                branch_row = branch_row.child(
                    Label::new("DETACHED")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                );
            }
            left = left.child(branch_row);
        }

        if let Some(ref state_label) = self.repo_state_label {
            left = left.child(
                div()
                    .h_flex()
                    .h(px(18.))
                    .px(px(6.))
                    .gap(px(3.))
                    .items_center()
                    .rounded(px(3.))
                    .bg(colors.ghost_element_selected)
                    .child(
                        Label::new(state_label.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Warning)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            );
        }

        if self.ahead > 0 || self.behind > 0 {
            left = left.child(
                div()
                    .h_flex()
                    .gap(px(6.))
                    .items_center()
                    .when(self.ahead > 0, |el| {
                        let ahead_text: SharedString = self.ahead.to_string().into();
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
                        let behind_text: SharedString = self.behind.to_string().into();
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

        if has_branch
            && (self.operation_message.is_some() || self.stash_count > 0)
        {
            left = left.child(div().w(px(1.)).h(px(14.)).bg(colors.border_variant));
        }

        if let Some(ref msg) = self.operation_message {
            let (msg_color, msg_icon) = if self.is_error {
                (Color::Error, Some(IconName::X))
            } else if self.is_loading {
                (Color::Info, Some(IconName::Refresh))
            } else {
                (Color::Muted, None)
            };

            let mut msg_row = div().h_flex().gap(px(4.)).items_center();

            if let Some(icon) = msg_icon {
                msg_row = msg_row.child(
                    Icon::new(icon).size(IconSize::XSmall).color(msg_color),
                );
            }

            msg_row = msg_row.child(
                Label::new(SharedString::from(msg.clone()))
                    .size(LabelSize::XSmall)
                    .color(msg_color),
            );

            left = left.child(msg_row);
        }

        if self.stash_count > 0 {
            let stash_text: SharedString = self.stash_count.to_string().into();
            left = left.child(
                div()
                    .h_flex()
                    .gap(px(3.))
                    .items_center()
                    .child(
                        Icon::new(IconName::Stash)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(stash_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        left
    }

    fn render_right_section(&self, colors: &rgitui_theme::ThemeColors) -> gpui::Div {
        let has_branch = !self.branch_name.is_empty();

        let mut right = div().h_flex().items_center().gap(px(10.));

        if self.staged_count > 0 {
            let staged_text: SharedString = format!("{} staged", self.staged_count).into();
            right = right.child(
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
            right = right.child(
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

        if self.staged_count == 0 && self.unstaged_count == 0 && has_branch {
            right = right.child(
                div()
                    .h_flex()
                    .gap(px(3.))
                    .items_center()
                    .child(
                        Icon::new(IconName::Check)
                            .size(IconSize::XSmall)
                            .color(Color::Success),
                    )
                    .child(
                        Label::new("Clean")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        if let Some(ref path) = self.repo_path {
            right = right
                .child(div().w(px(1.)).h(px(14.)).bg(colors.border_variant))
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .overflow_hidden()
                        .child(
                            Icon::new(IconName::Folder)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(path.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                );
        }

        right
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        let left = self.render_left_section(colors);
        let right = self.render_right_section(colors);

        div()
            .h_flex()
            .w_full()
            .h(px(28.))
            .bg(colors.status_bar_background)
            .border_t_1()
            .border_color(colors.border_variant)
            .px(px(12.))
            .items_center()
            .justify_between()
            .child(left)
            .child(right)
    }
}
