use gpui::prelude::*;
use gpui::{
    canvas, div, font, px, ClickEvent, Context, DragMoveEvent, ElementId, FontFallbacks,
    MouseButton, MouseDownEvent, Render, SharedString, Window,
};
use rgitui_git::GitOperationState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize,
    Spinner, SpinnerSize, Tab, TabBar,
};

use crate::{CommandId, StatusBar, TitleBar, ToastKind};

use super::{
    BottomPanelMode, CommitInputResize, DetailPanelResize, DiffViewerResize, RightPanelMode,
    SidebarResize, Workspace,
};

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus.pending_focus_restore {
            self.focus.pending_focus_restore = false;
            self.restore_focus(window, cx);
        }

        let colors = cx.colors().clone();

        let ui_font = {
            let configured = cx
                .try_global::<rgitui_settings::SettingsState>()
                .and_then(|s| {
                    let f = &s.settings().ui_font;
                    if f.is_empty() {
                        None
                    } else {
                        Some(f.clone())
                    }
                });

            let primary = configured.unwrap_or_else(|| "JetBrainsMono Nerd Font".to_string());

            let candidates = [
                "JetBrainsMono Nerd Font",
                "JetBrains Mono",
                #[cfg(target_os = "windows")]
                "Cascadia Code",
                #[cfg(target_os = "macos")]
                "SF Mono",
                #[cfg(target_os = "linux")]
                "DejaVu Sans Mono",
                "monospace",
            ];
            let fallbacks: Vec<String> = candidates
                .iter()
                .filter(|c| **c != primary)
                .map(|c| c.to_string())
                .collect();

            let mut f = font(SharedString::from(primary));
            f.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
            f
        };

        // If no tabs, show welcome screen
        if self.tabs.is_empty() {
            return div()
                .id("workspace-root")
                .size_full()
                .font(ui_font.clone())
                .bg(colors.background)
                .on_key_down(cx.listener(Self::handle_key_down))
                .child(self.render_welcome_interactive(cx))
                .child(self.toast_layer.clone())
                .child(self.overlays.command_palette.clone())
                .child(self.overlays.interactive_rebase.clone())
                .child(self.overlays.settings_modal.clone())
                .child(self.dialogs.branch_dialog.clone())
                .child(self.dialogs.tag_dialog.clone())
                .child(self.dialogs.rename_dialog.clone())
                .child(self.overlays.repo_opener.clone())
                .child(self.overlays.shortcuts_help.clone())
                .into_any_element();
        }

        let Some(active_tab) = self.tabs.get(self.active_tab) else {
            self.active_tab = 0;
            cx.notify();
            return div().into_any_element();
        };
        let project = active_tab.project.read(cx);
        let repo_name: SharedString = project.repo_name().to_string().into();
        let branch_name: SharedString = project
            .head_branch()
            .unwrap_or("detached")
            .to_string()
            .into();
        let has_changes = project.has_changes();
        let head_detached = project.is_head_detached();
        let repo_state = project.repo_state();
        let staged_count = project.status().staged.len();
        let unstaged_count = project.status().unstaged.len();
        let stash_count = project.stashes().len();
        let repo_path_display: SharedString = project.repo_path().display().to_string().into();
        let overlays_active = self.overlays.command_palette.read(cx).is_visible()
            || self.overlays.interactive_rebase.read(cx).is_visible()
            || self.overlays.settings_modal.read(cx).is_visible()
            || self.dialogs.branch_dialog.read(cx).is_visible()
            || self.dialogs.tag_dialog.read(cx).is_visible()
            || self.dialogs.rename_dialog.read(cx).is_visible()
            || self.overlays.repo_opener.read(cx).is_visible()
            || self.dialogs.confirm_dialog.read(cx).is_visible()
            || self.overlays.shortcuts_help.read(cx).is_visible();

        // Detect which panel has keyboard focus for visual indicators
        let sidebar_focused = active_tab.sidebar.read(cx).is_focused(window);
        let graph_focused = active_tab.graph.read(cx).is_focused(window);
        let detail_focused = active_tab.detail_panel.read(cx).is_focused(window);
        let diff_focused = active_tab.diff_viewer.read(cx).is_focused(window)
            || active_tab.blame_view.read(cx).is_focused(window)
            || active_tab.file_history_view.read(cx).is_focused(window);
        let focus_accent = colors.border_focused;
        let bottom_panel_mode = active_tab.bottom_panel_mode;

        // Find head branch info for ahead/behind
        let (ahead, behind) = project
            .branches()
            .iter()
            .find(|b| b.is_head)
            .map(|b| (b.ahead, b.behind))
            .unwrap_or((0, 0));

        // Build tab bar
        let mut tab_bar = TabBar::new();
        let workspace_handle = cx.entity().downgrade();
        for (i, tab) in self.tabs.iter().enumerate() {
            let tab_name: SharedString = tab.name.clone().into();
            let ws = workspace_handle.clone();
            let ws_close = workspace_handle.clone();
            tab_bar = tab_bar.tab(
                Tab::new(
                    ElementId::NamedInteger("project-tab".into(), i as u64),
                    tab_name,
                )
                .active(i == self.active_tab)
                .closeable(true)
                .on_click(move |_event, _window, cx| {
                    ws.update(cx, |ws, cx| {
                        if i < ws.tabs.len() {
                            ws.active_tab = i;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .on_close(move |_event, _window, cx| {
                    ws_close
                        .update(cx, |ws, cx| {
                            ws.close_tab(i, cx);
                        })
                        .ok();
                }),
            );
        }

        // Add workspace and repo actions to tab bar
        let ws_home = workspace_handle.clone();
        let ws_open = workspace_handle.clone();
        tab_bar = tab_bar.end_slot(
            div()
                .h_flex()
                .gap_1()
                .child(
                    IconButton::new("tab-bar-home", IconName::Folder)
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .on_click(move |_: &gpui::ClickEvent, _, cx| {
                            ws_home
                                .update(cx, |ws, cx| {
                                    ws.go_home(cx);
                                })
                                .ok();
                        }),
                )
                .child(
                    IconButton::new("tab-bar-add", IconName::Plus)
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .on_click(move |_: &gpui::ClickEvent, _, cx| {
                            ws_open
                                .update(cx, |ws, cx| {
                                    ws.overlays.repo_opener.update(cx, |ro, cx| {
                                        ro.toggle_visible(cx);
                                    });
                                })
                                .ok();
                        }),
                ),
        );

        // Status bar with operation message and state indicators
        let mut status_bar = StatusBar::new()
            .branch(branch_name.clone())
            .ahead_behind(ahead, behind)
            .changes(staged_count, unstaged_count)
            .stash_count(stash_count)
            .repo_path(repo_path_display)
            .loading(!self.operations.active_operations.is_empty())
            .error(self.operations.last_failed_git_operation.is_some())
            .head_detached(head_detached);
        if !repo_state.is_clean() {
            status_bar = status_bar.repo_state_label(repo_state.label());
        }

        if let Some(msg) = &self.status_message {
            status_bar = status_bar.operation_message(msg.clone());
        }

        let operation_banner = if let Some(update) = self
            .operations
            .active_git_operation
            .clone()
            .or_else(|| self.operations.last_failed_git_operation.clone())
        {
            let is_failure = update.state == GitOperationState::Failed;
            let accent = if is_failure {
                cx.status().error
            } else {
                cx.status().info
            };
            let bg = if is_failure {
                cx.status().error_background
            } else {
                cx.status().info_background
            };
            let icon = if is_failure {
                IconName::FileConflict
            } else {
                IconName::Refresh
            };
            let details = update.details.clone();

            Some(
                div()
                    .h_flex()
                    .w_full()
                    .min_h(px(30.))
                    .px(px(10.))
                    .py(px(4.))
                    .gap(px(6.))
                    .items_center()
                    .bg(bg)
                    .border_b_1()
                    .border_color(accent)
                    .child(Icon::new(icon).size(IconSize::Small).color(if is_failure {
                        Color::Error
                    } else {
                        Color::Info
                    }))
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                Label::new(SharedString::from(update.summary.clone()))
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate(),
                            )
                            .when_some(details, |el, details| {
                                el.child(
                                    Label::new(SharedString::from(details))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                )
                            }),
                    )
                    .when(is_failure && update.retryable, |el| {
                        let retry_update = update.clone();
                        el.child(
                            Button::new("operation-retry", "Retry")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Filled)
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                    this.retry_git_operation(&retry_update, cx);
                                })),
                        )
                    })
                    .when(is_failure, |el| {
                        el.child(
                            IconButton::new("operation-dismiss", IconName::X)
                                .size(ButtonSize::Compact)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.operations.last_failed_git_operation = None;
                                    cx.notify();
                                })),
                        )
                    }),
            )
        } else {
            None
        };

        let operation_output_bar = self.render_operation_output_bar(cx);

        div()
            .id("workspace-root")
            .v_flex()
            .size_full()
            .font(ui_font)
            .bg(colors.background)
            .on_key_down(cx.listener(Self::handle_key_down))
            // Title bar
            .child({
                let sidebar = active_tab.sidebar.clone();
                let mut title = TitleBar::new(repo_name.clone(), branch_name.clone())
                    .has_changes(has_changes)
                    .head_detached(head_detached)
                    .on_branch_click(move |_, window, cx| {
                        sidebar.update(cx, |sb, cx| {
                            sb.ensure_branches_visible(window, cx);
                        });
                    });
                if !repo_state.is_clean() {
                    title = title.repo_state(repo_state.label());
                }
                title
            })
            // Toolbar
            .child(active_tab.toolbar.clone())
            .when_some(operation_banner, |el, banner| el.child(banner))
            .when_some(operation_output_bar, |el, bar| el.child(bar))
            // Conflict state banner (merge/rebase/cherry-pick/revert in progress)
            .when(!repo_state.is_clean(), |el| {
                let has_conflicts = active_tab.project.read(cx).has_conflicts();
                let conflict_count = active_tab.project.read(cx).conflicted_files().len();
                let state_label: SharedString = repo_state.label().into();
                let detail_msg: SharedString = if has_conflicts {
                    format!(
                        "{} file{} with conflicts — resolve before continuing",
                        conflict_count,
                        if conflict_count == 1 { "" } else { "s" }
                    )
                    .into()
                } else {
                    "All conflicts resolved — ready to continue".into()
                };

                el.child(
                    div()
                        .h_flex()
                        .w_full()
                        .min_h(px(32.))
                        .px(px(10.))
                        .py(px(4.))
                        .gap(px(6.))
                        .items_center()
                        .bg(if has_conflicts {
                            cx.status().warning_background
                        } else {
                            cx.status().success_background
                        })
                        .border_b_1()
                        .border_color(if has_conflicts {
                            cx.status().warning
                        } else {
                            cx.status().success
                        })
                        .child(
                            Icon::new(if has_conflicts {
                                IconName::FileConflict
                            } else {
                                IconName::Check
                            })
                            .size(IconSize::Small)
                            .color(if has_conflicts {
                                Color::Warning
                            } else {
                                Color::Success
                            }),
                        )
                        .child(
                            div()
                                .v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    Label::new(state_label)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::SEMIBOLD)
                                        .truncate(),
                                )
                                .child(
                                    Label::new(detail_msg)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                        .child(
                            Button::new("conflict-continue", "Continue")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Filled)
                                .color(Color::Success)
                                .disabled(has_conflicts)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.execute_command(CommandId::ContinueMerge, cx);
                                })),
                        )
                        .child(
                            Button::new("conflict-abort", "Abort")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Subtle)
                                .color(Color::Error)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.execute_command(CommandId::AbortOperation, cx);
                                })),
                        ),
                )
            })
            // Tab bar
            .child(tab_bar)
            // Main content area — drag_move listeners live here so they fire globally
            .child({
                let entity = cx.entity();
                div()
                    .id("main-content")
                    .h_flex()
                    .flex_1()
                    .min_h_0()
                    // Capture content area bounds each frame for use in resize calculations
                    .child(
                        canvas(
                            {
                                let entity = entity.clone();
                                move |bounds, _, cx| {
                                    entity
                                        .update(cx, |this, _| this.layout.content_bounds = bounds);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    // Global resize drag listeners
                    .on_drag_move::<SidebarResize>(cx.listener(
                        |this, e: &DragMoveEvent<SidebarResize>, _, cx| {
                            let new_w =
                                f32::from(e.event.position.x - this.layout.content_bounds.left())
                                    .clamp(120., 600.);
                            this.layout.sidebar_width = new_w;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DetailPanelResize>(cx.listener(
                        |this, e: &DragMoveEvent<DetailPanelResize>, _, cx| {
                            let new_w =
                                f32::from(this.layout.content_bounds.right() - e.event.position.x)
                                    .clamp(180., 600.);
                            this.layout.detail_panel_width = new_w;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<DiffViewerResize>(cx.listener(
                        |this, e: &DragMoveEvent<DiffViewerResize>, _, cx| {
                            let new_h =
                                f32::from(this.layout.content_bounds.bottom() - e.event.position.y)
                                    .clamp(60., 500.);
                            this.layout.diff_viewer_height = new_h;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    .on_drag_move::<CommitInputResize>(cx.listener(
                        |this, e: &DragMoveEvent<CommitInputResize>, _, cx| {
                            let new_h = f32::from(
                                this.layout.right_panel_bounds.bottom() - e.event.position.y,
                            )
                            .clamp(240., 500.);
                            this.layout.commit_input_height = new_h;
                            this.schedule_layout_save(cx);
                            cx.notify();
                        },
                    ))
                    // Left sidebar — branches
                    .child(
                        div()
                            .relative()
                            .w(px(self.layout.sidebar_width))
                            .h_full()
                            .flex_shrink_0()
                            .when(sidebar_focused, |el| {
                                el.border_t_2().border_color(focus_accent)
                            })
                            .child(active_tab.sidebar.clone())
                            // Resize handle straddles the right border
                            .child(
                                div()
                                    .id("sidebar-resize-handle")
                                    .absolute()
                                    .top_0()
                                    .right(px(-3.))
                                    .h_full()
                                    .w(px(5.))
                                    .when(!overlays_active, |el| {
                                        el.cursor_col_resize()
                                            .hover(|s| {
                                                s.bg(gpui::Hsla {
                                                    a: 0.6,
                                                    ..colors.border_focused
                                                })
                                            })
                                            .on_drag(SidebarResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            ),
                    )
                    // Center: loading indicator + graph (flex) + resize strip + diff viewer (fixed height)
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            // Loading indicator with spinner
                            .when(!self.operations.active_operations.is_empty(), |el| {
                                let label: SharedString = self
                                    .operations
                                    .active_operations
                                    .last()
                                    .map(|op| {
                                        let elapsed = op.started_at.elapsed().as_secs();
                                        if elapsed >= 2 {
                                            SharedString::from(format!(
                                                "{} ({}s)",
                                                op.label, elapsed
                                            ))
                                        } else {
                                            op.label.clone()
                                        }
                                    })
                                    .unwrap_or_else(|| "Loading...".into());

                                el.child(
                                    div()
                                        .h_flex()
                                        .w_full()
                                        .h(px(28.))
                                        .px_3()
                                        .items_center()
                                        .gap_2()
                                        .bg(colors.surface_background)
                                        .border_b_1()
                                        .border_color(colors.border_variant)
                                        .child(
                                            Spinner::new().size(SpinnerSize::Small).label(label),
                                        ),
                                )
                            })
                            // Graph view
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .when(graph_focused, |el| {
                                        el.border_t_2().border_color(focus_accent)
                                    })
                                    .child(active_tab.graph.clone()),
                            )
                            // Drag-to-resize strip between graph and diff viewer
                            .child(
                                div()
                                    .id("diff-resize-handle")
                                    .w_full()
                                    .h(px(3.))
                                    .flex_shrink_0()
                                    .border_t_1()
                                    .border_color(colors.border_variant)
                                    .when(!overlays_active, |el| {
                                        el.cursor_row_resize()
                                            .hover(|s| {
                                                s.bg(gpui::Hsla {
                                                    a: 0.6,
                                                    ..colors.border_focused
                                                })
                                            })
                                            .on_drag(DiffViewerResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                            // Diff viewer / Blame view / File History
                            .child(
                                div()
                                    .h(px(self.layout.diff_viewer_height))
                                    .flex_shrink_0()
                                    .when(diff_focused, |el| {
                                        el.border_t_2().border_color(focus_accent)
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::Diff, |el| {
                                        el.child(active_tab.diff_viewer.clone())
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::Blame, |el| {
                                        el.child(active_tab.blame_view.clone())
                                    })
                                    .when(
                                        bottom_panel_mode == BottomPanelMode::FileHistory,
                                        |el| el.child(active_tab.file_history_view.clone()),
                                    ),
                            ),
                    )
                    // Right panel: detail + resize handle + commit input
                    .child({
                        let commit_input_height = self.layout.commit_input_height;
                        div()
                            .relative()
                            .w(px(self.layout.detail_panel_width))
                            .h_full()
                            .flex_shrink_0()
                            .v_flex()
                            .border_l_1()
                            .border_color(colors.border_variant)
                            // Bounds tracking canvas for commit input resize
                            .child(
                                canvas(
                                    {
                                        let entity = entity.clone();
                                        move |bounds, _, cx| {
                                            entity.update(cx, |this, _| {
                                                this.layout.right_panel_bounds = bounds
                                            });
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            // Right panel tab bar (Details / Issues)
                            .child({
                                let is_details =
                                    active_tab.right_panel_mode == RightPanelMode::Details;
                                let is_issues =
                                    active_tab.right_panel_mode == RightPanelMode::Issues;
                                let ws_details = cx.entity().downgrade();
                                let ws_issues = cx.entity().downgrade();
                                div()
                                    .h_flex()
                                    .w_full()
                                    .h(px(26.))
                                    .bg(colors.toolbar_background)
                                    .border_b_1()
                                    .border_color(colors.border_variant)
                                    .child(
                                        div()
                                            .id("right-tab-details")
                                            .h_flex()
                                            .h_full()
                                            .px(px(10.))
                                            .items_center()
                                            .cursor_pointer()
                                            .when(is_details, |el| {
                                                el.border_b_2().border_color(colors.text_accent)
                                            })
                                            .hover(|s| s.bg(colors.ghost_element_hover))
                                            .on_click(move |_: &ClickEvent, _, cx| {
                                                ws_details
                                                    .update(cx, |ws, cx| {
                                                        if let Some(tab) =
                                                            ws.tabs.get_mut(ws.active_tab)
                                                        {
                                                            tab.right_panel_mode =
                                                                RightPanelMode::Details;
                                                            cx.notify();
                                                        }
                                                    })
                                                    .ok();
                                            })
                                            .child(
                                                Label::new("Details")
                                                    .size(LabelSize::XSmall)
                                                    .weight(gpui::FontWeight::SEMIBOLD)
                                                    .color(if is_details {
                                                        Color::Default
                                                    } else {
                                                        Color::Muted
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("right-tab-issues")
                                            .h_flex()
                                            .h_full()
                                            .px(px(10.))
                                            .items_center()
                                            .cursor_pointer()
                                            .when(is_issues, |el| {
                                                el.border_b_2().border_color(colors.text_accent)
                                            })
                                            .hover(|s| s.bg(colors.ghost_element_hover))
                                            .on_click(move |_: &ClickEvent, _, cx| {
                                                ws_issues
                                                    .update(cx, |ws, cx| {
                                                        if let Some(tab) =
                                                            ws.tabs.get_mut(ws.active_tab)
                                                        {
                                                            tab.right_panel_mode =
                                                                RightPanelMode::Issues;
                                                            let ip = tab.issues_panel.clone();
                                                            ip.update(cx, |panel, cx| {
                                                                if !panel.has_issues_loaded()
                                                                    && !panel.is_loading()
                                                                {
                                                                    panel.fetch_issues(cx);
                                                                }
                                                            });
                                                            cx.notify();
                                                        }
                                                    })
                                                    .ok();
                                            })
                                            .child(
                                                Label::new("Issues")
                                                    .size(LabelSize::XSmall)
                                                    .weight(gpui::FontWeight::SEMIBOLD)
                                                    .color(if is_issues {
                                                        Color::Default
                                                    } else {
                                                        Color::Muted
                                                    }),
                                            ),
                                    )
                            })
                            // Panel content area — shows either details or issues
                            .child(
                                div()
                                    .id("right-panel-content")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_hidden()
                                    .when(detail_focused, |el| {
                                        el.border_t_2().border_color(focus_accent)
                                    })
                                    .when(
                                        active_tab.right_panel_mode == RightPanelMode::Details,
                                        |el| el.child(active_tab.detail_panel.clone()),
                                    )
                                    .when(
                                        active_tab.right_panel_mode == RightPanelMode::Issues,
                                        |el| el.child(active_tab.issues_panel.clone()),
                                    ),
                            )
                            // Resize handle between detail and commit input
                            .child(
                                div()
                                    .id("commit-input-resize-handle")
                                    .w_full()
                                    .h(px(3.))
                                    .flex_shrink_0()
                                    .border_t_1()
                                    .border_color(colors.border_variant)
                                    .when(!overlays_active, |el| {
                                        el.cursor_row_resize()
                                            .hover(|s| {
                                                s.bg(gpui::Hsla {
                                                    a: 0.6,
                                                    ..colors.border_focused
                                                })
                                            })
                                            .on_drag(CommitInputResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                            // Commit panel at bottom
                            .child(
                                div()
                                    .h(px(commit_input_height))
                                    .flex_shrink_0()
                                    .child(active_tab.commit_panel.clone()),
                            )
                            // Width resize handle on left edge
                            .child(
                                div()
                                    .id("detail-panel-resize-handle")
                                    .absolute()
                                    .top_0()
                                    .left(px(-3.))
                                    .h_full()
                                    .w(px(5.))
                                    .when(!overlays_active, |el| {
                                        el.cursor_col_resize()
                                            .hover(|s| {
                                                s.bg(gpui::Hsla {
                                                    a: 0.6,
                                                    ..colors.border_focused
                                                })
                                            })
                                            .on_drag(DetailPanelResize, |val, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| val.clone())
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                |_: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                    }),
                            )
                    })
            })
            // Status bar
            .child(status_bar)
            .child(self.toast_layer.clone())
            // Command palette overlay (rendered last to be on top)
            .child(self.overlays.command_palette.clone())
            // Interactive rebase dialog overlay
            .child(self.overlays.interactive_rebase.clone())
            // Settings modal overlay
            .child(self.overlays.settings_modal.clone())
            // Branch dialog overlay
            .child(self.dialogs.branch_dialog.clone())
            // Tag dialog overlay
            .child(self.dialogs.tag_dialog.clone())
            // Rename dialog overlay
            .child(self.dialogs.rename_dialog.clone())
            // Repo opener overlay
            .child(self.overlays.repo_opener.clone())
            // Confirm dialog overlay
            .child(self.dialogs.confirm_dialog.clone())
            // Shortcuts help overlay
            .child(self.overlays.shortcuts_help.clone())
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn render_welcome_interactive(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let recent_workspaces = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|settings| settings.recent_workspaces(6))
            .unwrap_or_default();
        let recent_repos = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|settings| settings.settings().recent_repos.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|path| path.exists())
            .take(6)
            .collect::<Vec<_>>();

        let mut content = div()
            .v_flex()
            .gap(px(12.))
            .items_center()
            .max_w(px(520.))
            .w_full()
            // Logo area
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(48.))
                    .h(px(48.))
                    .rounded(px(12.))
                    .bg(colors.element_background)
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::Medium)
                            .color(Color::Accent),
                    ),
            )
            .child(
                Label::new("rgitui")
                    .size(LabelSize::Large)
                    .weight(gpui::FontWeight::BOLD),
            )
            .child(
                Label::new("A workspace-oriented desktop Git client")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .mt(px(4.))
                    .child(
                        Button::new("workspace-home-open-repo", "Open Repository")
                            .style(ButtonStyle::Filled)
                            .icon(IconName::Folder)
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.overlays.repo_opener.update(cx, |opener, cx| {
                                    opener.toggle_visible(cx);
                                });
                            })),
                    )
                    .child(
                        Button::new("workspace-home-new", "New Workspace")
                            .style(ButtonStyle::Outlined)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.go_home(cx);
                                this.overlays.repo_opener.update(cx, |opener, cx| {
                                    opener.toggle_visible(cx);
                                });
                            })),
                    )
                    .when(!recent_workspaces.is_empty(), |buttons| {
                        buttons.child(
                            Button::new("workspace-home-restore", "Restore Last")
                                .style(ButtonStyle::Subtle)
                                .icon(IconName::Clock)
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.restore_last_workspace(cx);
                                })),
                        )
                    }),
            );

        if !recent_workspaces.is_empty() {
            let mut workspaces_list = div().v_flex().w_full().mt(px(4.)).gap(px(4.)).child(
                Label::new("Recent Workspaces")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

            for (i, workspace) in recent_workspaces.iter().enumerate() {
                let workspace_id = workspace.id.clone();
                let workspace_name: SharedString = workspace.name.clone().into();
                let summary: SharedString = format!(
                    "{} repositories · updated {}",
                    workspace.repos.len(),
                    workspace
                        .last_opened_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                )
                .into();
                let repo_preview: SharedString = workspace
                    .repos
                    .iter()
                    .take(2)
                    .map(|repo| {
                        repo.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| repo.display().to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
                    .into();

                workspaces_list = workspaces_list.child(
                    div()
                        .id(ElementId::NamedInteger("recent-workspace".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .min_h(px(48.))
                        .px_3()
                        .py(px(6.))
                        .gap_2()
                        .items_start()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .bg(colors.ghost_element_background)
                        .border_1()
                        .border_color(colors.border_variant)
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                            let snapshot = cx
                                .try_global::<rgitui_settings::SettingsState>()
                                .and_then(|settings| settings.workspace(&workspace_id).cloned());
                            if let Some(snapshot) = snapshot {
                                if let Err(error) = this.restore_workspace_snapshot(snapshot, cx) {
                                    this.show_toast(error.to_string(), ToastKind::Error, cx);
                                }
                            }
                        }))
                        .child(
                            Icon::new(IconName::Stash)
                                .size(IconSize::Medium)
                                .color(Color::Accent),
                        )
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Label::new(workspace_name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM),
                                )
                                .child(
                                    Label::new(summary)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(repo_preview)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        ),
                );
            }

            content = content.child(workspaces_list);
        }

        if !recent_repos.is_empty() {
            let mut repos_list = div().v_flex().w_full().gap(px(2.)).child(
                Label::new("Recent Repositories")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            );

            for (i, repo_path) in recent_repos.iter().enumerate() {
                let repo_name: SharedString = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| repo_path.display().to_string())
                    .into();
                let repo_dir: SharedString = repo_path.display().to_string().into();
                let path = repo_path.clone();

                repos_list = repos_list.child(
                    div()
                        .id(ElementId::NamedInteger("recent-repo".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(32.))
                        .px_3()
                        .gap_2()
                        .items_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                            if let Err(error) = this.open_repo(path.clone(), cx) {
                                this.show_toast(error.to_string(), ToastKind::Error, cx);
                            }
                        }))
                        .child(
                            Icon::new(IconName::Folder)
                                .size(IconSize::Small)
                                .color(Color::Accent),
                        )
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Label::new(repo_name)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM),
                                )
                                .child(
                                    Label::new(repo_dir)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        ),
                );
            }

            content = content.child(repos_list);
        }

        // Keyboard shortcut hints
        content = content.child(
            div()
                .v_flex()
                .gap(px(4.))
                .mt(px(8.))
                .w_full()
                .items_center()
                .child(self.shortcut_hint("Open Repository", "Ctrl+O", colors))
                .child(self.shortcut_hint("Go Home", "Ctrl+H", colors))
                .child(self.shortcut_hint("Command Palette", "Ctrl+Shift+P", colors))
                .child(self.shortcut_hint("Settings", "Ctrl+,", colors)),
        );

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.background)
            .child(content)
    }

    fn shortcut_hint(
        &self,
        action: &str,
        shortcut: &str,
        colors: &rgitui_theme::ThemeColors,
    ) -> impl IntoElement {
        div()
            .h_flex()
            .w(px(260.))
            .justify_between()
            .items_center()
            .child(
                Label::new(SharedString::from(action.to_string()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .h_flex()
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .bg(colors.element_background)
                    .items_center()
                    .child(
                        Label::new(SharedString::from(shortcut.to_string()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
    }

    /// Schedule a debounced layout save (avoids writing to disk on every resize pixel).
    /// Cancels any previously scheduled save task to prevent task queue buildup.
    pub(super) fn schedule_layout_save(&mut self, cx: &mut Context<Self>) {
        // Drop the previous task (cancels it) before spawning a new one
        self.layout_save_task = None;

        let task = cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                this.update(cx, |this, cx| {
                    this.layout_save_task = None;
                    this.save_layout(cx);
                })
                .ok();
            },
        );
        self.layout_save_task = Some(task);
    }

    /// Persist current layout dimensions to settings.
    pub(super) fn save_layout(&self, cx: &mut Context<Self>) {
        if cx.try_global::<rgitui_settings::SettingsState>().is_some() {
            let settings = cx.global_mut::<rgitui_settings::SettingsState>();
            settings.settings_mut().layout.sidebar_width = self.layout.sidebar_width;
            settings.settings_mut().layout.detail_panel_width = self.layout.detail_panel_width;
            settings.settings_mut().layout.diff_viewer_height = self.layout.diff_viewer_height;
            settings.settings_mut().layout.commit_input_height = self.layout.commit_input_height;
            if let Err(e) = settings.save() {
                log::error!("Failed to save layout: {}", e);
            }
        }
    }
}

pub(crate) fn open_file_explorer(path: &std::path::Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            let canonical = path.canonicalize().unwrap_or(path.to_path_buf());
            let _ = std::process::Command::new("explorer.exe")
                .arg(canonical)
                .spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(&path).spawn();
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        }
    });
}

pub(crate) fn open_terminal(path: &std::path::Path, custom_command: &str) {
    let path = path.to_path_buf();
    let custom_command = custom_command.to_string();
    std::thread::spawn(move || {
        if !custom_command.is_empty() {
            let parts: Vec<&str> = custom_command.split_whitespace().collect();
            if let Some((program, args)) = parts.split_first() {
                let _ = std::process::Command::new(program)
                    .args(args)
                    .current_dir(&path)
                    .spawn();
            }
        } else {
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("wt.exe")
                    .arg("-d")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("cmd.exe")
                            .arg("/K")
                            .arg("cd")
                            .arg("/d")
                            .arg(&path)
                            .spawn()
                    });
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg("-a")
                    .arg("Terminal")
                    .arg(&path)
                    .spawn();
            }
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("x-terminal-emulator")
                    .current_dir(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("xterm")
                            .current_dir(&path)
                            .spawn()
                    });
            }
        }
    });
}

pub(crate) fn open_editor(path: &std::path::Path, custom_command: &str) {
    let path = path.to_path_buf();
    let custom_command = custom_command.to_string();
    std::thread::spawn(move || {
        if !custom_command.is_empty() {
            let parts: Vec<&str> = custom_command.split_whitespace().collect();
            if let Some((program, args)) = parts.split_first() {
                let _ = std::process::Command::new(program)
                    .args(args)
                    .arg(&path)
                    .spawn();
            }
        } else {
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("code").arg(&path).spawn();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("code")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("open")
                            .arg("-a")
                            .arg("TextEdit")
                            .arg(&path)
                            .spawn()
                    });
            }
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("code")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| std::process::Command::new("xdg-open").arg(&path).spawn());
            }
        }
    });
}
