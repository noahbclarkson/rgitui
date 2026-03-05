use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Label, LabelSize};

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
            .h(px(24.))
            .bg(colors.status_bar_background)
            .border_t_1()
            .border_color(colors.border_variant)
            .px_2()
            .gap_3()
            .items_center();

        // Branch name
        if !self.branch_name.is_empty() {
            bar = bar.child(
                Label::new(self.branch_name)
                    .size(LabelSize::XSmall)
                    .color(Color::Accent),
            );
        }

        // Ahead/behind
        if self.ahead > 0 || self.behind > 0 {
            let sync_text: SharedString =
                format!("↑{} ↓{}", self.ahead, self.behind).into();
            bar = bar.child(
                Label::new(sync_text)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }

        // Operation message (in center)
        if let Some(msg) = self.operation_message {
            bar = bar.child(
                Label::new(SharedString::from(msg))
                    .size(LabelSize::XSmall)
                    .color(Color::Info),
            );
        }

        // Spacer
        bar = bar.child(div().flex_1());

        // Changes
        if self.staged_count > 0 {
            let staged_text: SharedString = format!("{} staged", self.staged_count).into();
            bar = bar.child(
                Label::new(staged_text)
                    .size(LabelSize::XSmall)
                    .color(Color::Added),
            );
        }
        if self.unstaged_count > 0 {
            let unstaged_text: SharedString =
                format!("{} changed", self.unstaged_count).into();
            bar = bar.child(
                Label::new(unstaged_text)
                    .size(LabelSize::XSmall)
                    .color(Color::Modified),
            );
        }

        bar
    }
}
