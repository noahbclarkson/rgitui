use gpui::prelude::*;
use gpui::{div, px, App, ClickEvent, SharedString, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The application title bar.
#[derive(IntoElement)]
pub struct TitleBar {
    repo_name: SharedString,
    branch_name: SharedString,
    has_changes: bool,
    head_detached: bool,
    repo_state_label: Option<SharedString>,
    on_branch_click: Option<ClickHandler>,
}

impl TitleBar {
    pub fn new(repo_name: impl Into<SharedString>, branch_name: impl Into<SharedString>) -> Self {
        Self {
            repo_name: repo_name.into(),
            branch_name: branch_name.into(),
            has_changes: false,
            head_detached: false,
            repo_state_label: None,
            on_branch_click: None,
        }
    }

    pub fn has_changes(mut self, has_changes: bool) -> Self {
        self.has_changes = has_changes;
        self
    }

    pub fn head_detached(mut self, detached: bool) -> Self {
        self.head_detached = detached;
        self
    }

    pub fn repo_state(mut self, label: impl Into<SharedString>) -> Self {
        self.repo_state_label = Some(label.into());
        self
    }

    pub fn on_branch_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_branch_click = Some(Box::new(handler));
        self
    }

    fn render_separator(colors: &rgitui_theme::ThemeColors) -> gpui::Div {
        div()
            .w(px(1.))
            .h(px(14.))
            .rounded(px(0.5))
            .bg(colors.border_variant)
    }

    fn render_keyboard_hint(
        colors: &rgitui_theme::ThemeColors,
        key: &'static str,
        label_text: &'static str,
    ) -> gpui::Div {
        div()
            .h_flex()
            .gap(px(3.))
            .items_center()
            .child(
                div()
                    .h(px(16.))
                    .px(px(4.))
                    .rounded(px(3.))
                    .bg(colors.ghost_element_hover)
                    .flex()
                    .items_center()
                    .child(
                        Label::new(key)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::MEDIUM),
                    ),
            )
            .child(
                Label::new(label_text)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

impl RenderOnce for TitleBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();

        let branch_color = if self.head_detached {
            Color::Warning
        } else {
            Color::Accent
        };

        let hover_bg = colors.ghost_element_active;
        let branch_bg = colors.ghost_element_hover;

        let mut branch_pill = div()
            .id("title-branch-pill")
            .h_flex()
            .h(px(22.))
            .px(px(8.))
            .gap(px(4.))
            .items_center()
            .rounded(px(4.))
            .bg(branch_bg)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .tooltip(Tooltip::text(self.branch_name.clone()))
            .child(
                Icon::new(IconName::GitBranch)
                    .size(IconSize::Small)
                    .color(branch_color),
            )
            .child(
                Label::new(self.branch_name.clone())
                    .size(LabelSize::Small)
                    .color(branch_color)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .truncate(),
            )
            .when(self.head_detached, |el| {
                el.child(
                    Label::new("(detached)")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                )
            })
            .when(self.has_changes && !self.head_detached, |el| {
                el.child(
                    div()
                        .w(px(6.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(colors.vc_modified),
                )
            });

        if let Some(handler) = self.on_branch_click {
            branch_pill = branch_pill.on_click(handler);
        }

        let mut bar = div()
            .h_flex()
            .w_full()
            .h(px(34.))
            .bg(colors.title_bar_background)
            .border_b_1()
            .border_color(colors.border)
            .px(px(12.))
            .gap(px(8.))
            .items_center()
            .child(
                div()
                    .h_flex()
                    .gap(px(5.))
                    .items_center()
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("rgitui")
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(Self::render_separator(colors))
            .child(
                div()
                    .h_flex()
                    .gap(px(5.))
                    .items_center()
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(self.repo_name)
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            )
            .child(Self::render_separator(colors))
            .child(branch_pill);

        if let Some(state_label) = self.repo_state_label {
            bar = bar.child(
                div()
                    .h_flex()
                    .h(px(20.))
                    .px(px(6.))
                    .gap(px(4.))
                    .items_center()
                    .rounded(px(3.))
                    .bg(colors.ghost_element_selected)
                    .child(
                        Icon::new(IconName::FileConflict)
                            .size(IconSize::XSmall)
                            .color(Color::Warning),
                    )
                    .child(
                        Label::new(state_label)
                            .size(LabelSize::XSmall)
                            .color(Color::Warning)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    ),
            );
        }

        bar = bar.child(div().flex_1());

        bar.child(
            div()
                .h_flex()
                .gap(px(12.))
                .items_center()
                .child(Self::render_keyboard_hint(
                    colors,
                    "Ctrl+Shift+P",
                    "Commands",
                ))
                .child(Self::render_keyboard_hint(colors, "?", "Help")),
        )
    }
}
