use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, ElementId, EventEmitter, Render, SharedString, Window};
use rgitui_git::{CommitDiff, CommitInfo, FileDiff};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Label, LabelSize};

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
        let message: SharedString = commit.message.clone().into();

        let mut panel = div()
            .id("detail-panel")
            .v_flex()
            .size_full()
            .bg(colors.panel_background)
            .overflow_y_scroll();

        // Commit header
        panel = panel.child(
            div()
                .v_flex()
                .w_full()
                .px_3()
                .py_2()
                .gap_1()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                // SHA
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Label::new("SHA")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(full_sha)
                                .size(LabelSize::XSmall)
                                .color(Color::Accent),
                        ),
                )
                // Author
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Label::new("Author")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        )
                        .child(Label::new(author_name).size(LabelSize::XSmall))
                        .child(
                            Label::new(author_email)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                )
                // Date
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Label::new("Date")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        )
                        .child(Label::new(date).size(LabelSize::XSmall)),
                )
                // Parents
                .when(!commit.parent_oids.is_empty(), |el| {
                    let parents_text: SharedString = commit
                        .parent_oids
                        .iter()
                        .map(|oid| format!("{:.7}", oid))
                        .collect::<Vec<_>>()
                        .join(", ")
                        .into();
                    el.child(
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
                    )
                }),
        );

        // Commit message
        panel = panel.child(
            div()
                .w_full()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    Label::new(message)
                        .size(LabelSize::Small),
                ),
        );

        // Changed files list
        if let Some(diff) = &self.commit_diff {
            let stats: SharedString =
                format!(
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

                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("detail-file".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
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
