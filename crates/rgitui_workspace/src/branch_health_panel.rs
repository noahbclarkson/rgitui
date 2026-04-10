//! Branch Health Panel — shows unmerged, stale, and diverged branches.
//!
//! Displays statistics and a filterable list of local branches that need attention:
//! - **Unmerged**: branches not yet merged into main/master
//! - **Stale**: branches with no new commits in 30+ days
//! - **Diverged**: branches ahead AND behind their upstream

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, FocusHandle, Render, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{BranchInfo, GitProject};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Badge, Icon, IconName, IconSize, Label, LabelSize};

use crate::time::format_relative_time_full;

const STALE_DAYS: i64 = 30;
const STALE_SECONDS: i64 = STALE_DAYS * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub enum BranchHealthFilter {
    All,
    Unmerged,
    Stale,
    Diverged,
}

impl BranchHealthFilter {
    fn label(&self) -> &'static str {
        match self {
            BranchHealthFilter::All => "All",
            BranchHealthFilter::Unmerged => "Unmerged",
            BranchHealthFilter::Stale => "Stale",
            BranchHealthFilter::Diverged => "Diverged",
        }
    }
}

pub struct BranchHealthPanel {
    filter: BranchHealthFilter,
    branches: Vec<BranchInfo>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    project: WeakEntity<GitProject>,
}

impl BranchHealthPanel {
    pub fn new(cx: &mut Context<Self>, project: WeakEntity<GitProject>) -> Self {
        Self {
            filter: BranchHealthFilter::All,
            branches: Vec::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            project,
        }
    }

    /// Refresh the branch list from the project.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(proj) = self.project.upgrade() else {
            return;
        };
        let project = proj.read(cx);
        self.branches = project.branches().to_vec();
        cx.notify();
    }

    fn filtered_branches(&self) -> Vec<&BranchInfo> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.branches
            .iter()
            .filter(|b| {
                if b.is_remote {
                    return false;
                }
                match self.filter {
                    BranchHealthFilter::All => true,
                    BranchHealthFilter::Unmerged => {
                        b.is_merged_into_main == Some(false) && !b.is_head
                    }
                    BranchHealthFilter::Stale => {
                        if let Some(time) = b.last_commit_time {
                            (now - time) > STALE_SECONDS
                        } else {
                            false
                        }
                    }
                    BranchHealthFilter::Diverged => b.ahead > 0 && b.behind > 0,
                }
            })
            .collect()
    }

    fn stats(&self) -> (usize, usize, usize, usize) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let local: Vec<&BranchInfo> = self.branches.iter().filter(|b| !b.is_remote).collect();
        let total = local.len();
        let unmerged = local
            .iter()
            .filter(|b| b.is_merged_into_main == Some(false) && !b.is_head)
            .count();
        let stale = local
            .iter()
            .filter(|b| {
                if let Some(time) = b.last_commit_time {
                    (now - time) > STALE_SECONDS
                } else {
                    false
                }
            })
            .count();
        let diverged = local.iter().filter(|b| b.ahead > 0 && b.behind > 0).count();
        (total, unmerged, stale, diverged)
    }

    fn set_filter(&mut self, filter: BranchHealthFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }
}

impl Render for BranchHealthPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let panel_bg = colors.panel_background;

        // Refresh data on each render
        self.refresh(cx);

        let (total, unmerged, stale, diverged) = self.stats();
        let filtered: Vec<BranchInfo> = self.filtered_branches().into_iter().cloned().collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        div()
            .id("branch-health-panel")
            .track_focus(&self.focus_handle)
            .v_flex()
            .size_full()
            .bg(panel_bg)
            .child(self.render_toolbar(cx))
            // Stats row
            .child(
                div()
                    .id("branch-health-stats")
                    .h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .flex_wrap()
                    .child(self.stat_badge("Total", total, Color::Default, cx))
                    .child(self.stat_badge("Unmerged", unmerged, Color::Warning, cx))
                    .child(self.stat_badge("Stale", stale, Color::Muted, cx))
                    .child(self.stat_badge("Diverged", diverged, Color::Accent, cx)),
            )
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .px_3()
                    .child(div().flex_1().h_px().bg(colors.border_variant)),
            )
            // Branch list
            .child(
                uniform_list(
                    "branch-health-list",
                    filtered.len(),
                    move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|ix| {
                                let branch = &filtered[ix];
                                let is_head = branch.is_head;
                                let name: SharedString = branch.name.clone().into();
                                let ahead = branch.ahead;
                                let behind = branch.behind;
                                let is_merged = branch.is_merged_into_main;
                                let last_time = branch.last_commit_time;
                                let upstream = branch.upstream.clone();
                                let relative = last_time.map(format_relative_time_full);
                                let relative_str: SharedString =
                                    relative.unwrap_or_else(|| "unknown".into()).into();

                                let row_bg = if is_head {
                                    colors.ghost_element_selected
                                } else {
                                    colors.panel_background
                                };

                                let mut row = div()
                                    .id(ElementId::NamedInteger("bh-row".into(), ix as u64))
                                    .h_flex()
                                    .w_full()
                                    .h(px(40.))
                                    .px_3()
                                    .gap_2()
                                    .items_center()
                                    .bg(row_bg)
                                    .hover(|s| s.bg(colors.ghost_element_hover));

                                // HEAD indicator
                                if is_head {
                                    row = row.child(
                                        Icon::new(IconName::GitBranch)
                                            .size(IconSize::XSmall)
                                            .color(Color::Success),
                                    );
                                }

                                // Branch name
                                row = row.child(
                                    Label::new(name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::SEMIBOLD)
                                        .color(if is_head {
                                            Color::Success
                                        } else {
                                            Color::Default
                                        }),
                                );

                                // Merged badge
                                if !is_head {
                                    if is_merged == Some(true) {
                                        row = row.child(Badge::new("Merged").color(Color::Muted));
                                    } else if is_merged == Some(false) {
                                        row =
                                            row.child(Badge::new("Unmerged").color(Color::Warning));
                                    }
                                }

                                // Stale badge
                                if let Some(time) = last_time {
                                    if (now - time) > STALE_SECONDS && !is_head {
                                        row = row.child(Badge::new("Stale").color(Color::Muted));
                                    }
                                }

                                row = row.child(div().flex_1());

                                // Upstream indicator
                                if let Some(up) = &upstream {
                                    let up_label: SharedString = up.clone().into();
                                    row = row.child(
                                        div()
                                            .h_flex()
                                            .gap_px()
                                            .items_center()
                                            .child(
                                                Icon::new(IconName::GitBranch)
                                                    .size(IconSize::XSmall)
                                                    .color(Color::Muted),
                                            )
                                            .child(
                                                Label::new(up_label)
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted),
                                            ),
                                    );
                                }

                                // Ahead/behind
                                if ahead > 0 || behind > 0 {
                                    let mut ab = div().h_flex().gap_px().items_center();
                                    if ahead > 0 {
                                        let a: SharedString = format!("↑{}", ahead).into();
                                        ab = ab.child(
                                            Label::new(a)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Success),
                                        );
                                    }
                                    if behind > 0 {
                                        let b: SharedString = format!("↓{}", behind).into();
                                        ab = ab.child(
                                            Label::new(b)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Error),
                                        );
                                    }
                                    row = row.child(ab);
                                }

                                // Last commit time
                                row = row.child(
                                    Label::new(relative_str)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Placeholder),
                                );

                                row.into_any_element()
                            })
                            .collect()
                    },
                )
                .flex_1()
                .size_full()
                .with_sizing_behavior(gpui::ListSizingBehavior::Infer)
                .track_scroll(&self.scroll_handle),
            )
            .into_any_element()
    }
}

impl BranchHealthPanel {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();
        let filters = vec![
            BranchHealthFilter::All,
            BranchHealthFilter::Unmerged,
            BranchHealthFilter::Stale,
            BranchHealthFilter::Diverged,
        ];

        div()
            .id("branch-health-toolbar")
            .h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_px()
            .border_b_1()
            .border_color(colors.border_variant)
            .children(filters.into_iter().map(|f| {
                let active = self.filter == f;
                let label: SharedString = f.label().into();
                let filter_name = f.label();
                div()
                    .id(ElementId::Name(format!("bh-filter-{}", filter_name).into()))
                    .px_3()
                    .py_1()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .bg(if active {
                        colors.ghost_element_selected
                    } else {
                        gpui::Hsla::default()
                    })
                    .border_1()
                    .border_color(if active {
                        colors.border_focused
                    } else {
                        colors.border_transparent
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_filter(f.clone(), cx);
                    }))
                    .child(
                        Label::new(label)
                            .size(LabelSize::XSmall)
                            .weight(if active {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::MEDIUM
                            })
                            .color(if active { Color::Default } else { Color::Muted }),
                    )
            }))
            .into_any_element()
    }

    fn stat_badge(
        &self,
        label: &'static str,
        count: usize,
        color: Color,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors();
        let count_str: SharedString = format!("{}", count).into();
        let label_str: SharedString = label.into();

        div()
            .id(ElementId::Name(format!("bh-stat-{}", label).into()))
            .h_flex()
            .gap_px()
            .items_center()
            .px_2()
            .py_px()
            .rounded(px(4.))
            .bg(colors.ghost_element_background)
            .child(
                Label::new(count_str)
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::BOLD)
                    .color(color),
            )
            .child(
                Label::new(label_str)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .into_any_element()
    }
}
