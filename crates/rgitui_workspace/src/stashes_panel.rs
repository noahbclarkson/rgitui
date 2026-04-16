//! Stashes Panel — shows all git stash entries with Apply/Pop/Drop/Branch actions.
//!
//! Mirrors the design language of `branch_health_panel.rs` and `prs_panel.rs`.
//! Stash operations delegate to `GitProject` stash methods.

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, FocusHandle, Render, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::{GitProject, StashEntry};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize,
};

use crate::workspace::Workspace;

/// The main stashes panel.
pub struct StashesPanel {
    stashes: Vec<StashEntry>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    project: WeakEntity<GitProject>,
    workspace: WeakEntity<Workspace>,
}

impl StashesPanel {
    pub fn new(
        cx: &mut Context<Self>,
        project: WeakEntity<GitProject>,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            stashes: Vec::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            project,
            workspace,
        }
    }

    /// Refresh the stash list from the project.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(proj) = self.project.upgrade() else {
            return;
        };
        let project = proj.read(cx);
        self.stashes = project.stashes().to_vec();
        cx.notify();
    }

    fn apply_stash(&self, index: usize, cx: &mut Context<Self>) {
        let Some(proj) = self.project.upgrade() else {
            return;
        };
        proj.update(cx, |proj, cx| {
            proj.stash_apply(index, cx).detach();
        });
    }

    fn pop_stash(&self, index: usize, cx: &mut Context<Self>) {
        let Some(proj) = self.project.upgrade() else {
            return;
        };
        proj.update(cx, |proj, cx| {
            proj.stash_pop(index, cx).detach();
        });
    }

    fn drop_stash(&self, index: usize, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.upgrade() else {
            return;
        };
        ws.update(cx, |ws, cx| {
            ws.dialogs.confirm_dialog.update(cx, |cd, cx| {
                cd.show_visible(
                    "Drop Stash",
                    format!(
                        "Are you sure you want to drop stash@{{{}}}? This cannot be undone.",
                        index
                    ),
                    crate::ConfirmAction::StashDrop(index),
                    cx,
                );
            });
        });
    }
}

impl Render for StashesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Refresh on every render to keep stash list in sync
        self.refresh(cx);

        let colors = cx.colors().clone();
        let panel_bg = colors.panel_background;
        let stashes = self.stashes.clone();

        let empty = stashes.is_empty();

        div()
            .id("stashes-panel")
            .track_focus(&self.focus_handle)
            .v_flex()
            .size_full()
            .bg(panel_bg)
            .child(self.render_toolbar(cx, empty))
            .child({
                if empty {
                    self.render_empty_state(cx)
                } else {
                    self.render_stash_list(&stashes, cx)
                }
            })
            .into_any_element()
    }
}

impl StashesPanel {
    fn render_toolbar(&self, cx: &mut Context<Self>, empty: bool) -> gpui::AnyElement {
        let colors = cx.colors();
        let count: SharedString = format!("{}", self.stashes.len()).into();

        div()
            .id("stashes-toolbar")
            .h_flex()
            .w_full()
            .h(px(36.))
            .px(px(12.))
            .gap(px(6.))
            .items_center()
            .bg(colors.surface_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                Icon::new(IconName::Stash)
                    .size(IconSize::XSmall)
                    .color(Color::Accent),
            )
            .child(
                Label::new("Stashes")
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Default),
            )
            .when(!empty, |el| el.child(Badge::new(count).color(Color::Muted)))
            .child(div().flex_1())
            .child(
                IconButton::new("stashes-refresh", IconName::Refresh)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.refresh(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(32.))
                    .py(px(24.))
                    .rounded(px(12.))
                    .bg(colors.ghost_element_background)
                    .child(
                        Icon::new(IconName::Stash)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new("No stashes")
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Use the toolbar to stash changes")
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .into_any_element()
    }

    fn render_stash_list(
        &self,
        stashes: &[StashEntry],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors();
        // Clone everything the uniform_list closure needs — it requires 'static
        let stash_bg = colors.panel_background;
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;
        let stashes: Vec<StashEntry> = stashes.to_vec();
        let item_h = px(52.);
        let weak = cx.weak_entity();

        uniform_list(
            "stashes-list",
            stashes.len(),
            move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let stash = &stashes[i];
                        let idx = stash.index;
                        let msg: SharedString = stash.message.clone().into();
                        let oid_str: SharedString = format!("{:.7}", stash.oid).into();

                        let mut item = div()
                            .id(ElementId::NamedInteger("stash-item".into(), i as u64))
                            .h_flex()
                            .w_full()
                            .min_h(item_h)
                            .px_3()
                            .py_2()
                            .gap_2()
                            .items_start()
                            .bg(stash_bg)
                            .hover(|s| s.bg(hover_bg))
                            .active(|s| s.bg(active_bg));

                        // Icon + message column
                        item = item
                            .child(
                                Icon::new(IconName::Stash)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_px()
                                    .min_w_0()
                                    .child(
                                        Label::new(msg)
                                            .size(LabelSize::Small)
                                            .color(Color::Default)
                                            .truncate(),
                                    )
                                    .child(
                                        Label::new(oid_str)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Placeholder),
                                    ),
                            );

                        // Action buttons
                        let btn_style = ButtonStyle::Subtle;
                        let btn_size = ButtonSize::Compact;

                        // Apply
                        {
                            let w = weak.clone();
                            item = item.child(
                                IconButton::new(
                                    ElementId::NamedInteger("apply-stash".into(), i as u64),
                                    IconName::Check,
                                )
                                .size(btn_size)
                                .style(btn_style)
                                .color(Color::Success)
                                .tooltip("Apply stash (keep)")
                                .on_click(
                                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        let _ = w.update(cx, |this: &mut StashesPanel, cx| {
                                            this.apply_stash(idx, cx);
                                        });
                                    },
                                ),
                            );
                        }

                        // Pop
                        {
                            let w = weak.clone();
                            item = item.child(
                                IconButton::new(
                                    ElementId::NamedInteger("pop-stash".into(), i as u64),
                                    IconName::ArrowDown,
                                )
                                .size(btn_size)
                                .style(btn_style)
                                .color(Color::Accent)
                                .tooltip("Pop stash (remove)")
                                .on_click(
                                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        let _ = w.update(cx, |this: &mut StashesPanel, cx| {
                                            this.pop_stash(idx, cx);
                                        });
                                    },
                                ),
                            );
                        }

                        // Drop
                        {
                            let w = weak.clone();
                            item = item.child(
                                IconButton::new(
                                    ElementId::NamedInteger("drop-stash".into(), i as u64),
                                    IconName::Trash,
                                )
                                .size(btn_size)
                                .style(btn_style)
                                .color(Color::Error)
                                .tooltip("Drop stash")
                                .on_click(
                                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        let _ = w.update(cx, |this: &mut StashesPanel, cx| {
                                            this.drop_stash(idx, cx);
                                        });
                                    },
                                ),
                            );
                        }

                        item.into_any_element()
                    })
                    .collect()
            },
        )
        .flex_1()
        .size_full()
        .with_sizing_behavior(gpui::ListSizingBehavior::Infer)
        .track_scroll(&self.scroll_handle)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_entry_clone_equal() {
        let entry = StashEntry {
            index: 0,
            message: "WIP on main: abc1234".into(),
            oid: git2::Oid::zero(),
        };
        let clone = entry.clone();
        assert_eq!(entry, clone);
    }

    #[test]
    fn stash_entry_index_different() {
        let e0 = StashEntry {
            index: 0,
            message: "stash 0".into(),
            oid: git2::Oid::zero(),
        };
        let e1 = StashEntry {
            index: 1,
            message: "stash 0".into(),
            oid: git2::Oid::zero(),
        };
        assert_ne!(e0.index, e1.index);
        assert_ne!(e0, e1);
    }

    #[test]
    fn stash_entry_message_different() {
        let e0 = StashEntry {
            index: 0,
            message: "WIP on main".into(),
            oid: git2::Oid::zero(),
        };
        let e1 = StashEntry {
            index: 0,
            message: "WIP on feature".into(),
            oid: git2::Oid::zero(),
        };
        assert_ne!(e0.message, e1.message);
        assert_ne!(e0, e1);
    }

    #[test]
    fn stash_entry_debug() {
        let entry = StashEntry {
            index: 3,
            message: "test".into(),
            oid: git2::Oid::zero(),
        };
        let debug = format!("{:?}", entry);
        assert!(debug.contains("3"));
        assert!(debug.contains("test"));
    }
}
