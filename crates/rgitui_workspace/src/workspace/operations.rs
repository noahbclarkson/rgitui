use std::time::Instant;

use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, SharedString};
use rgitui_git::{GitOperationKind, GitOperationUpdate};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize};

use crate::ToastKind;

use super::{Workspace, OPERATION_OUTPUT_AUTO_HIDE_SECS};

impl Workspace {
    pub(super) fn schedule_operation_output_auto_hide(
        &mut self,
        timestamp: Instant,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(
                    OPERATION_OUTPUT_AUTO_HIDE_SECS,
                ))
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if let Some(output) = &this.operations.last_operation_output {
                        if output.timestamp == timestamp && !output.expanded && !output.is_error {
                            this.operations.last_operation_output = None;
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
        })
        .detach();
    }

    pub(super) fn render_operation_output_bar(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let output = self.operations.last_operation_output.as_ref()?;
        let colors = cx.colors().clone();

        let accent = if output.is_error {
            cx.status().error
        } else {
            cx.status().success
        };
        let bg = if output.is_error {
            cx.status().error_background
        } else {
            cx.status().success_background
        };
        let icon = if output.is_error {
            IconName::FileConflict
        } else {
            IconName::Check
        };
        let icon_color = if output.is_error {
            Color::Error
        } else {
            Color::Success
        };

        let operation_label = output.operation.clone();
        let first_line: SharedString = output
            .output
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
            .into();
        let full_output: SharedString = output.output.clone().into();
        let expanded = output.expanded;
        let has_multiple_lines = output.output.lines().count() > 1;

        let mut bar = div()
            .id("operation-output-bar")
            .v_flex()
            .w_full()
            .bg(bg)
            .border_b_1()
            .border_color(accent);

        let mut header = div()
            .h_flex()
            .w_full()
            .min_h(px(28.))
            .px(px(10.))
            .py(px(2.))
            .gap(px(6.))
            .items_center()
            .child(Icon::new(icon).size(IconSize::XSmall).color(icon_color))
            .child(
                Label::new(operation_label)
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD),
            )
            .child(
                div().flex_1().min_w_0().overflow_hidden().child(
                    Label::new(first_line)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .truncate(),
                ),
            );

        if has_multiple_lines {
            header = header.child(
                IconButton::new(
                    "output-expand",
                    if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    },
                )
                .size(ButtonSize::Compact)
                .style(ButtonStyle::Transparent)
                .color(Color::Muted)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(op_output) = &mut this.operations.last_operation_output {
                        op_output.expanded = !op_output.expanded;
                    }
                    cx.notify();
                })),
            );
        }

        header = header.child(
            IconButton::new("output-dismiss", IconName::X)
                .size(ButtonSize::Compact)
                .style(ButtonStyle::Transparent)
                .color(Color::Muted)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.operations.last_operation_output = None;
                    cx.notify();
                })),
        );

        bar = bar.child(header);

        if expanded {
            bar = bar.child(
                div()
                    .id("operation-output-content")
                    .w_full()
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(6.))
                    .child(
                        div()
                            .font_family("monospace")
                            .text_xs()
                            .text_color(colors.text_muted)
                            .whitespace_nowrap()
                            .child(full_output),
                    ),
            );
        }

        Some(bar.into_any_element())
    }

    pub(super) fn retry_git_operation(
        &mut self,
        update: &GitOperationUpdate,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project().cloned() else {
            self.show_toast("No active repository to retry.", ToastKind::Warning, cx);
            return;
        };

        let handled = project.update(cx, |proj, cx| match update.kind {
            GitOperationKind::Fetch => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.fetch(remote_name, cx).detach();
                    true
                } else {
                    proj.fetch_default(cx).detach();
                    true
                }
            }
            GitOperationKind::Pull => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.pull(remote_name, cx).detach();
                    true
                } else {
                    proj.pull_default(cx).detach();
                    true
                }
            }
            GitOperationKind::Push => {
                let force = update.summary.to_ascii_lowercase().contains("force push");
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.push(remote_name, force, cx).detach();
                    true
                } else {
                    proj.push_default(force, cx).detach();
                    true
                }
            }
            GitOperationKind::Checkout => {
                if let Some(branch_name) = update.branch_name.as_deref() {
                    proj.checkout_branch(branch_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            GitOperationKind::Merge => {
                if let Some(branch_name) = update.branch_name.as_deref() {
                    proj.merge_branch(branch_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            GitOperationKind::RemoveRemote => {
                if let Some(remote_name) = update.remote_name.as_deref() {
                    proj.remove_remote(remote_name, cx).detach();
                    true
                } else {
                    false
                }
            }
            _ => false,
        });

        if !handled {
            self.show_toast(
                "This operation cannot be retried automatically.",
                ToastKind::Info,
                cx,
            );
        } else {
            self.operations.last_failed_git_operation = None;
        }
    }
}
