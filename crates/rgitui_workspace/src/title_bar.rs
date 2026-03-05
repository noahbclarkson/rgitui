use gpui::prelude::*;
use gpui::{div, px, App, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Label, LabelSize};

/// The application title bar.
#[derive(IntoElement)]
pub struct TitleBar {
    repo_name: SharedString,
    branch_name: SharedString,
    has_changes: bool,
}

impl TitleBar {
    pub fn new(repo_name: impl Into<SharedString>, branch_name: impl Into<SharedString>) -> Self {
        Self {
            repo_name: repo_name.into(),
            branch_name: branch_name.into(),
            has_changes: false,
        }
    }

    pub fn has_changes(mut self, has_changes: bool) -> Self {
        self.has_changes = has_changes;
        self
    }
}

impl RenderOnce for TitleBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        div()
            .h_flex()
            .w_full()
            .h(px(38.))
            .bg(colors.title_bar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .px_3()
            .gap_2()
            .items_center()
            // App name
            .child(
                Label::new("rgitui")
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .weight(gpui::FontWeight::BOLD),
            )
            // Separator
            .child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .bg(colors.border_variant),
            )
            // Repo name
            .child(Label::new(self.repo_name).size(LabelSize::Small))
            // Branch indicator
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Label::new(self.branch_name)
                            .size(LabelSize::Small)
                            .color(Color::Accent),
                    )
                    .when(self.has_changes, |el| {
                        el.child(
                            div()
                                .w(px(6.))
                                .h(px(6.))
                                .rounded_full()
                                .bg(colors.vc_modified),
                        )
                    }),
            )
            // Spacer
            .child(div().flex_1())
    }
}
