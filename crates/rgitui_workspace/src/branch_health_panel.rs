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
        filter_branches(&self.branches, &self.filter)
    }

    fn stats(&self) -> (usize, usize, usize, usize) {
        compute_branch_stats(&self.branches)
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

/// Filter branches by the given filter.
fn filter_branches<'a>(
    branches: &'a [BranchInfo],
    filter: &'a BranchHealthFilter,
) -> Vec<&'a BranchInfo> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    branches
        .iter()
        .filter(|b| {
            if b.is_remote {
                return false;
            }
            match filter {
                BranchHealthFilter::All => true,
                BranchHealthFilter::Unmerged => b.is_merged_into_main == Some(false) && !b.is_head,
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

/// Compute branch health statistics: (total, unmerged, stale, diverged).
fn compute_branch_stats(branches: &[BranchInfo]) -> (usize, usize, usize, usize) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let local: Vec<&BranchInfo> = branches.iter().filter(|b| !b.is_remote).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use rgitui_git::BranchInfo;

    fn make_branch(
        name: &str,
        is_head: bool,
        is_remote: bool,
        is_merged_into_main: Option<bool>,
        last_commit_time: Option<i64>,
        ahead: usize,
        behind: usize,
    ) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_head,
            is_remote,
            upstream: None,
            ahead,
            behind,
            tip_oid: None,
            author_email: None,
            last_commit_time,
            is_merged_into_main,
        }
    }

    // BranchHealthFilter::label

    #[test]
    fn branch_health_filter_label_all() {
        assert_eq!(BranchHealthFilter::All.label(), "All");
    }

    #[test]
    fn branch_health_filter_label_unmerged() {
        assert_eq!(BranchHealthFilter::Unmerged.label(), "Unmerged");
    }

    #[test]
    fn branch_health_filter_label_stale() {
        assert_eq!(BranchHealthFilter::Stale.label(), "Stale");
    }

    #[test]
    fn branch_health_filter_label_diverged() {
        assert_eq!(BranchHealthFilter::Diverged.label(), "Diverged");
    }

    // BranchHealthFilter equality

    #[test]
    fn branch_health_filter_eq() {
        assert_eq!(BranchHealthFilter::All, BranchHealthFilter::All);
        assert_eq!(BranchHealthFilter::Unmerged, BranchHealthFilter::Unmerged);
        assert_eq!(BranchHealthFilter::Stale, BranchHealthFilter::Stale);
        assert_eq!(BranchHealthFilter::Diverged, BranchHealthFilter::Diverged);
        assert_ne!(BranchHealthFilter::All, BranchHealthFilter::Unmerged);
    }

    // filtered_branches — All filter returns all local non-remote branches
    #[test]
    fn filtered_branches_all_includes_local_branches() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let branches = vec![
            make_branch("main", false, false, Some(true), Some(now - 100), 0, 0),
            make_branch("feature", false, false, Some(false), Some(now - 100), 0, 0),
            make_branch("origin/main", false, true, None, Some(now - 100), 0, 0), // remote — excluded
        ];

        let filtered: Vec<_> = filter_branches(&branches, &BranchHealthFilter::All);
        assert_eq!(filtered.len(), 2, "All filter returns only local branches");
        assert!(filtered.iter().all(|b| !b.is_remote));
    }

    // filtered_branches — Unmerged filter excludes HEAD
    #[test]
    fn filtered_branches_unmerged_excludes_head() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let branches = vec![
            make_branch("main", false, false, Some(false), Some(now - 100), 0, 0),
            make_branch("HEAD", true, false, Some(false), Some(now - 100), 0, 0), // is_head — excluded
        ];

        let filtered: Vec<_> = filter_branches(&branches, &BranchHealthFilter::Unmerged);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "main");
        assert!(!filtered[0].is_head);
    }

    // filtered_branches — Stale filter excludes recent branches
    #[test]
    fn filtered_branches_stale_excludes_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // 10 days ago — not stale (STALE_DAYS = 30)
        let recent_time = now - (10 * 24 * 60 * 60);
        // 40 days ago — stale
        let stale_time = now - (40 * 24 * 60 * 60);

        let branches = vec![
            make_branch("recent", false, false, Some(false), Some(recent_time), 0, 0),
            make_branch("old", false, false, Some(false), Some(stale_time), 0, 0),
        ];

        let filtered: Vec<_> = filter_branches(&branches, &BranchHealthFilter::Stale);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "old");
    }

    // filtered_branches — Diverged filter requires ahead AND behind
    #[test]
    fn filtered_branches_diverged_requires_ahead_and_behind() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let branches = vec![
            make_branch("ahead-only", false, false, Some(false), Some(now), 3, 0),
            make_branch("behind-only", false, false, Some(false), Some(now), 0, 2),
            make_branch("diverged", false, false, Some(false), Some(now), 3, 2),
            make_branch("clean", false, false, Some(false), Some(now), 0, 0),
        ];

        let filtered: Vec<_> = filter_branches(&branches, &BranchHealthFilter::Diverged);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "diverged");
    }

    // stats — counts match expected
    #[test]
    fn stats_counts_correctly() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let recent_time = now - (10 * 24 * 60 * 60);
        let stale_time = now - (40 * 24 * 60 * 60);

        let branches = vec![
            // main — merged, recent
            make_branch("main", false, false, Some(true), Some(recent_time), 0, 0),
            // feature — unmerged, not head, recent
            make_branch(
                "feature",
                false,
                false,
                Some(false),
                Some(recent_time),
                0,
                0,
            ),
            // stale-branch — stale, not merged
            make_branch("stale", false, false, Some(false), Some(stale_time), 0, 0),
            // origin/main — remote, excluded
            make_branch("origin/main", false, true, None, Some(stale_time), 0, 0),
            // diverged — diverged
            make_branch(
                "diverged",
                false,
                false,
                Some(false),
                Some(recent_time),
                2,
                1,
            ),
        ];

        let (total, unmerged, stale, diverged) = compute_branch_stats(&branches);
        assert_eq!(total, 4, "4 local branches");
        assert_eq!(
            unmerged, 3,
            "3 unmerged (feature, stale, diverged — HEAD excluded)"
        );
        assert_eq!(stale, 1, "1 stale (stale)");
        assert_eq!(diverged, 1, "1 diverged");
    }

    // stats — empty branch list
    #[test]
    fn stats_empty_branches() {
        let (total, unmerged, stale, diverged) = compute_branch_stats(&[]);
        assert_eq!(total, 0);
        assert_eq!(unmerged, 0);
        assert_eq!(stale, 0);
        assert_eq!(diverged, 0);
    }

    // stats — HEAD branch is counted in total but not in unmerged
    #[test]
    fn stats_head_not_in_unmerged() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let branches = vec![
            make_branch("HEAD", true, false, Some(false), Some(now), 0, 0),
            make_branch("other", false, false, Some(false), Some(now), 0, 0),
        ];

        let (total, unmerged, _, _) = compute_branch_stats(&branches);
        assert_eq!(total, 2, "HEAD counts in total");
        assert_eq!(unmerged, 1, "HEAD excluded from unmerged");
    }
}
