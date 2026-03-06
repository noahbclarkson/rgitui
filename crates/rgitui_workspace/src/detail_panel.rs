use gpui::prelude::*;
use gpui::{div, img, px, ClickEvent, Context, ElementId, EventEmitter, ObjectFit, Render, SharedString, Window};
use rgitui_git::{CommitDiff, CommitInfo, FileDiff};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{AvatarCache, Icon, IconName, IconSize, Label, LabelSize};

/// Events from the detail panel.
#[derive(Debug, Clone)]
pub enum DetailPanelEvent {
    FileSelected(FileDiff, String),
}

/// Displays commit metadata and changed files list.
pub struct DetailPanel {
    commit: Option<CommitInfo>,
    commit_diff: Option<CommitDiff>,
    selected_file_index: Option<usize>,
}

impl EventEmitter<DetailPanelEvent> for DetailPanel {}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            commit: None,
            commit_diff: None,
            selected_file_index: None,
        }
    }

    pub fn set_commit(
        &mut self,
        commit: CommitInfo,
        diff: CommitDiff,
        cx: &mut Context<Self>,
    ) {
        self.commit = Some(commit);
        self.commit_diff = Some(diff);
        self.selected_file_index = None;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.commit = None;
        self.commit_diff = None;
        self.selected_file_index = None;
        cx.notify();
    }
}

impl Render for DetailPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let Some(commit) = &self.commit else {
            return div()
                .id("detail-panel")
                .size_full()
                .bg(colors.panel_background)
                .into_any_element();
        };

        let full_sha: SharedString = format!("{}", commit.oid).into();
        let author_name: SharedString = commit.author.name.clone().into();
        let author_email: SharedString = format!("<{}>", commit.author.email).into();
        let date: SharedString = commit.time.format("%Y-%m-%d %H:%M:%S UTC").to_string().into();

        // Split message into summary (first line) and description (rest)
        let (summary, description) = {
            let msg = &commit.message;
            match msg.find('\n') {
                Some(idx) => (
                    msg[..idx].trim().to_string(),
                    msg[idx + 1..].trim().to_string(),
                ),
                None => (msg.trim().to_string(), String::new()),
            }
        };
        let summary: SharedString = summary.into();

        // Author initials for avatar
        let initials: SharedString = commit
            .author
            .name
            .split_whitespace()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase()
            .into();

        let mut panel = div()
            .id("detail-panel")
            .v_flex()
            .size_full()
            .bg(colors.panel_background)
            .overflow_y_scroll();

        // Commit header card
        let mut card = div()
            .v_flex()
            .w_full()
            .px_3()
            .py_3()
            .gap_2()
            .bg(colors.elevated_surface_background)
            .border_b_1()
            .border_color(colors.border_variant);

        // Summary line (commit title — normal size, bold)
        card = card.child(
            Label::new(summary)
                .size(LabelSize::Small)
                .weight(gpui::FontWeight::BOLD),
        );

        // Description (if multi-line commit message)
        if !description.is_empty() {
            let desc: SharedString = description.into();
            card = card.child(
                div()
                    .w_full()
                    .pt_1()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new(desc)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        // Author row with avatar
        let avatar_url = cx
            .try_global::<AvatarCache>()
            .and_then(|cache| cache.avatar_url(&commit.author.email))
            .map(|s| s.to_string());
        let avatar_bg = colors.border_focused;
        let avatar_text_color = colors.background;
        let mut avatar_circle = div()
            .w(px(22.))
            .h(px(22.))
            .rounded_full()
            .bg(avatar_bg)
            .flex()
            .items_center()
            .justify_center();

        if let Some(url) = avatar_url {
            let fb_initials = initials.clone();
            avatar_circle = avatar_circle.child(
                img(url)
                    .rounded_full()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_color(avatar_text_color)
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(fb_initials.clone()),
                            )
                            .into_any_element()
                    }),
            );
        } else {
            avatar_circle = avatar_circle.child(
                div()
                    .text_color(avatar_text_color)
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(initials.clone()),
            );
        }

        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(avatar_circle)
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .h_flex()
                                .gap_1()
                                .child(
                                    Label::new(author_name)
                                        .size(LabelSize::XSmall)
                                        .weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(author_email)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            Label::new(date)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        // SHA row
        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    Label::new(full_sha)
                        .size(LabelSize::XSmall)
                        .color(Color::Accent),
                ),
        );

        // Parents
        if !commit.parent_oids.is_empty() {
            let parents_text: SharedString = commit
                .parent_oids
                .iter()
                .map(|oid| format!("{:.7}", oid))
                .collect::<Vec<_>>()
                .join(", ")
                .into();
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Label::new("Parents")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(parents_text)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        panel = panel.child(card);

        // Changed files list
        if let Some(diff) = &self.commit_diff {
            let stats: SharedString = format!(
                "{} file{} changed, +{} -{}",
                diff.files.len(),
                if diff.files.len() == 1 { "" } else { "s" },
                diff.total_additions,
                diff.total_deletions
            )
            .into();

            panel = panel.child(
                div()
                    .h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(colors.surface_background)
                    .child(
                        Label::new(stats)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

            let files = diff.files.clone();
            for (i, file) in files.iter().enumerate() {
                let path_str = file.path.display().to_string();
                let path_label: SharedString = path_str.clone().into();
                let file_stats: SharedString =
                    format!("+{} -{}", file.additions, file.deletions).into();
                let selected = self.selected_file_index == Some(i);
                let file_diff = file.clone();
                let path_for_event = path_str.clone();

                let (icon_name, icon_color) = if file.deletions == 0 && file.additions > 0 {
                    (IconName::FileAdded, Color::Added)
                } else if file.additions == 0 && file.deletions > 0 {
                    (IconName::FileDeleted, Color::Deleted)
                } else {
                    (IconName::FileModified, Color::Modified)
                };

                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("detail-file".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(28.))
                        .px_3()
                        .gap_2()
                        .items_center()
                        .when(selected, |el| el.bg(colors.ghost_element_selected))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_file_index = Some(i);
                            cx.emit(DetailPanelEvent::FileSelected(
                                file_diff.clone(),
                                path_for_event.clone(),
                            ));
                            cx.notify();
                        }))
                        .child(
                            Icon::new(icon_name)
                                .size(IconSize::Small)
                                .color(icon_color),
                        )
                        .child(
                            Label::new(path_label)
                                .size(LabelSize::XSmall)
                                .truncate(),
                        )
                        .child(div().flex_1())
                        .child(
                            Label::new(file_stats)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                );
            }
        }

        panel.into_any_element()
    }
}

