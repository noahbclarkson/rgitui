use gpui::prelude::*;
use gpui::{
    canvas, div, px, App, ClickEvent, Context, CursorStyle, DragMoveEvent, ElementId,
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
        log::trace!("Workspace::render: tabs={} active={}", self.tabs.len(), self.active_tab);
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

            if let Some((cached_name, cached_font)) = &self.cached_ui_font {
                if cached_name == &primary {
                    cached_font.clone()
                } else {
                    let f = Self::build_ui_font(primary.clone());
                    self.cached_ui_font = Some((primary, f.clone()));
                    f
                }
            } else {
                let f = Self::build_ui_font(primary.clone());
                self.cached_ui_font = Some((primary, f.clone()));
                f
            }
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
                .child(self.dialogs.worktree_dialog.clone())
                .child(self.dialogs.rename_dialog.clone())
                .child(self.dialogs.stash_branch_dialog.clone())
                .child(self.dialogs.create_pr_dialog.clone())
                .child(self.overlays.repo_opener.clone())
                .child(self.overlays.shortcuts_help.clone())
                .child(self.overlays.global_search.clone())
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
            || self.dialogs.worktree_dialog.read(cx).is_visible()
            || self.dialogs.rename_dialog.read(cx).is_visible()
            || self.overlays.repo_opener.read(cx).is_visible()
            || self.dialogs.confirm_dialog.read(cx).is_visible()
            || self.dialogs.create_pr_dialog.read(cx).is_visible()
            || self.overlays.shortcuts_help.read(cx).is_visible()
            || self.overlays.global_search.read(cx).is_visible();

        // Detect which panel has keyboard focus for visual indicators
        let sidebar_focused = active_tab.sidebar.read(cx).is_focused(window);
        let graph_focused = active_tab.graph.read(cx).is_focused(window);
        let detail_focused = active_tab.detail_panel.read(cx).is_focused(window);
        let diff_focused = active_tab.diff_viewer.read(cx).is_focused(window)
            || active_tab.blame_view.read(cx).is_focused(window)
            || active_tab.file_history_view.read(cx).is_focused(window)
            || active_tab.reflog_view.read(cx).is_focused(window)
            || active_tab.submodule_view.read(cx).is_focused(window);
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
                        "{} file{} with conflicts -- resolve before continuing",
                        conflict_count,
                        if conflict_count == 1 { "" } else { "s" }
                    )
                    .into()
                } else {
                    "All conflicts resolved -- ready to continue".into()
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
                                    .clamp(180., 720.);
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
                            .clamp(300., 500.);
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
                            // Bottom panel tabs + content
                            .child({
                                let tab_bar_bg = colors.toolbar_background;
                                let tab_border = colors.border_variant;
                                let tab_active_bg = colors.element_selected;
                                let tab_hover = colors.ghost_element_hover;

                                let make_tab =
                                    |id: &'static str,
                                     label: &'static str,
                                     mode: BottomPanelMode,
                                     current: BottomPanelMode| {
                                        let active = mode == current;
                                        let label: SharedString = label.into();
                                        div()
                                            .id(SharedString::from(id))
                                            .h(px(24.))
                                            .px(px(10.))
                                            .flex()
                                            .items_center()
                                            .cursor(CursorStyle::PointingHand)
                                            .rounded_t(px(4.))
                                            .text_xs()
                                            .when(active, |el| el.bg(tab_active_bg))
                                            .when(!active, |el| el.hover(move |s| s.bg(tab_hover)))
                                            .child(Label::new(label).size(LabelSize::XSmall).color(
                                                if active { Color::Default } else { Color::Muted },
                                            ))
                                    };

                                let ws = cx.entity().downgrade();
                                let ws2 = cx.entity().downgrade();
                                let ws3 = cx.entity().downgrade();

                                let tab_bar = div()
                                    .h_flex()
                                    .w_full()
                                    .h(px(26.))
                                    .bg(tab_bar_bg)
                                    .border_b_1()
                                    .border_color(tab_border)
                                    .gap(px(2.))
                                    .px(px(6.))
                                    .items_end()
                                    .child(
                                        make_tab(
                                            "bottom-tab-diff",
                                            "Diff",
                                            BottomPanelMode::Diff,
                                            bottom_panel_mode,
                                        )
                                        .on_click(
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                ws.update(cx, |this, cx| {
                                                    if let Some(tab) =
                                                        this.tabs.get_mut(this.active_tab)
                                                    {
                                                        tab.bottom_panel_mode =
                                                            BottomPanelMode::Diff;
                                                    }
                                                    cx.notify();
                                                })
                                                .ok();
                                            },
                                        ),
                                    )
                                    .child(
                                        make_tab(
                                            "bottom-tab-history",
                                            "History",
                                            BottomPanelMode::FileHistory,
                                            bottom_panel_mode,
                                        )
                                        .on_click(
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                ws2.update(cx, |this, cx| {
                                                    if let Some(tab) =
                                                        this.tabs.get_mut(this.active_tab)
                                                    {
                                                        if tab.bottom_panel_mode
                                                            == BottomPanelMode::FileHistory
                                                        {
                                                            tab.bottom_panel_mode =
                                                                BottomPanelMode::Diff;
                                                        } else {
                                                            this.execute_command(
                                                                crate::CommandId::FileHistory,
                                                                cx,
                                                            );
                                                        }
                                                    }
                                                    cx.notify();
                                                })
                                                .ok();
                                            },
                                        ),
                                    )
                                    .child(
                                        make_tab(
                                            "bottom-tab-blame",
                                            "Blame",
                                            BottomPanelMode::Blame,
                                            bottom_panel_mode,
                                        )
                                        .on_click(
                                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                                ws3.update(cx, |this, cx| {
                                                    if let Some(tab) =
                                                        this.tabs.get_mut(this.active_tab)
                                                    {
                                                        if tab.bottom_panel_mode
                                                            == BottomPanelMode::Blame
                                                        {
                                                            tab.bottom_panel_mode =
                                                                BottomPanelMode::Diff;
                                                        } else {
                                                            this.execute_command(
                                                                crate::CommandId::Blame,
                                                                cx,
                                                            );
                                                        }
                                                    }
                                                    cx.notify();
                                                })
                                                .ok();
                                            },
                                        ),
                                    );

                                div()
                                    .v_flex()
                                    .h(px(self.layout.diff_viewer_height))
                                    .flex_shrink_0()
                                    .when(diff_focused, |el| {
                                        el.border_t_2().border_color(focus_accent)
                                    })
                                    .child(tab_bar)
                                    .when(bottom_panel_mode == BottomPanelMode::Diff, |el| {
                                        el.child(active_tab.diff_viewer.clone())
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::Blame, |el| {
                                        el.child(active_tab.blame_view.clone())
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::FileHistory, |el| {
                                        el.child(active_tab.file_history_view.clone())
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::Reflog, |el| {
                                        el.child(active_tab.reflog_view.clone())
                                    })
                                    .when(bottom_panel_mode == BottomPanelMode::Submodules, |el| {
                                        el.child(active_tab.submodule_view.clone())
                                    })
                            }),
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
                            // Right panel tab bar (Details / Issues / PRs)
                            .child({
                                let is_details =
                                    active_tab.right_panel_mode == RightPanelMode::Details;
                                let is_issues =
                                    active_tab.right_panel_mode == RightPanelMode::Issues;
                                let is_prs =
                                    active_tab.right_panel_mode == RightPanelMode::PullRequests;
                                let ws_details = cx.entity().downgrade();
                                let ws_issues = cx.entity().downgrade();
                                let ws_prs = cx.entity().downgrade();
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
                                    .child(
                                        div()
                                            .id("right-tab-prs")
                                            .h_flex()
                                            .h_full()
                                            .px(px(10.))
                                            .items_center()
                                            .cursor_pointer()
                                            .when(is_prs, |el| {
                                                el.border_b_2().border_color(colors.text_accent)
                                            })
                                            .hover(|s| s.bg(colors.ghost_element_hover))
                                            .on_click(move |_: &ClickEvent, _, cx| {
                                                ws_prs
                                                    .update(cx, |ws, cx| {
                                                        if let Some(tab) =
                                                            ws.tabs.get_mut(ws.active_tab)
                                                        {
                                                            tab.right_panel_mode =
                                                                RightPanelMode::PullRequests;
                                                            let pp = tab.prs_panel.clone();
                                                            pp.update(cx, |panel, cx| {
                                                                if !panel.has_prs_loaded()
                                                                    && !panel.is_loading()
                                                                {
                                                                    panel.fetch_prs(cx);
                                                                }
                                                            });
                                                            cx.notify();
                                                        }
                                                    })
                                                    .ok();
                                            })
                                            .child(
                                                Label::new("PRs")
                                                    .size(LabelSize::XSmall)
                                                    .weight(gpui::FontWeight::SEMIBOLD)
                                                    .color(if is_prs {
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
                                    )
                                    .when(
                                        active_tab.right_panel_mode == RightPanelMode::PullRequests,
                                        |el| el.child(active_tab.prs_panel.clone()),
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
            // Worktree dialog overlay
            .child(self.dialogs.worktree_dialog.clone())
            // Rename dialog overlay
            .child(self.dialogs.rename_dialog.clone())
            // Repo opener overlay
            .child(self.overlays.repo_opener.clone())
            // Confirm dialog overlay
            .child(self.dialogs.confirm_dialog.clone())
            // Create PR dialog overlay
            .child(self.dialogs.create_pr_dialog.clone())
            // Shortcuts help overlay
            .child(self.overlays.shortcuts_help.clone())
            // Global search overlay
            .child(self.overlays.global_search.clone())
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
                    "{} repositories | updated {}",
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

/// Console-based terminals need `CREATE_NEW_CONSOLE` so Windows allocates a
/// visible console window.  GUI terminal emulators (wt, alacritty, …) create
/// their own windows and should be spawned with `DETACHED_PROCESS` instead.
#[cfg(target_os = "windows")]
fn is_console_terminal(program: &str) -> bool {
    matches!(
        program.to_ascii_lowercase().as_str(),
        "cmd" | "cmd.exe" | "command.com" | "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    )
}

/// Builds the terminal command arguments from a custom command string and repo path.
/// Returns `(program, args)` where `args` does NOT include the path — callers
/// that need `current_dir` use it directly, callers that pass the path as an
/// argument construct the final args list accordingly.
///
/// This is a pure function to enable unit testing.
#[allow(dead_code)]
pub(crate) fn build_terminal_args(
    custom_command: &str,
    path: &std::path::Path,
) -> (String, Vec<String>) {
    let path_str = path.to_string_lossy().to_string();
    let parts: Vec<&str> = custom_command.split_whitespace().collect();

    if let Some((program, rest)) = parts.split_first() {
        // For known single-word commands, apply terminal-specific argument conventions.
        // For multi-word commands (e.g. "wt -d"), treat all parts after the program
        // as the initial args to pass through.
        match program.to_ascii_lowercase().as_str() {
            #[cfg(target_os = "windows")]
            "wt" | "wt.exe" => {
                // Windows Terminal: `-d <path>` — the -d flag is mandatory for wt
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.extend(["-d".to_string(), path_str]);
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "powershell" | "pwsh" | "pwsh.exe" | "powershell.exe" => {
                // PowerShell: `-NoExit -Command "cd '<path>'"`
                let mut args = vec!["-NoExit".to_string(), "-Command".to_string()];
                args.push(format!("cd '{}'", path_str.replace('\'', "''")));
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "cmd" | "cmd.exe" | "command.com" => {
                // Command Prompt: `/K cd /d <path>`
                // This explicitly changes directory, unlike current_dir() which may not
                // be respected when cmd.exe is spawned as a detached process from a GUI app.
                let args = vec![
                    "/K".to_string(),
                    "cd".to_string(),
                    "/d".to_string(),
                    path_str,
                ];
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "alacritty" | "alacritty.exe" => {
                // Alacritty: `--working-directory <path>`
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.push("--working-directory".to_string());
                args.push(path_str);
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "wezterm" | "wezterm.exe" | "wezterm-mux-server" | "wezterm-cli" => {
                // WezTerm: `--cwd <path>`
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.push("--cwd".to_string());
                args.push(path_str);
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "kitty" | "kitty.exe" => {
                // Kitty: `--directory <path>`
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.push("--directory".to_string());
                args.push(path_str);
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            "macos" | "terminal" | "terminal.app" | "iterm" | "iterm2" => {
                // These are handled by macOS-specific code paths; pass path as bare arg.
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.push(path_str);
                (program.to_string(), args)
            }
            #[cfg(target_os = "windows")]
            _ => {
                // Unknown command on Windows: for console programs, use cmd-style /K cd /d
                // since current_dir() may not propagate correctly to console processes spawned
                // from a GUI app. For GUI programs, current_dir() alone is sufficient.
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                if is_console_terminal(program) && args.is_empty() {
                    // Use /K (keep running) with cd /d (change drive + dir)
                    args.extend([
                        "/K".to_string(),
                        "cd".to_string(),
                        "/d".to_string(),
                        path_str,
                    ]);
                } else {
                    args.push(path_str);
                }
                (program.to_string(), args)
            }
            #[cfg(not(target_os = "windows"))]
            _ => {
                // Non-Windows: path is set via current_dir(), pass as bare arg.
                let mut args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                args.push(path_str);
                (program.to_string(), args)
            }
        }
    } else {
        // Empty command — fall back to default terminal detection.
        ("cmd.exe".to_string(), vec![])
    }
}

/// Builds the editor command arguments from a custom command string and a file path.
/// Unlike terminal commands which can use `current_dir()`, editors need the path
/// passed as a proper argument in a way the specific editor understands.
///
/// Returns `(program, args)` where the file path is NOT included — callers
/// construct the final command by appending the path appropriately per-editor.
pub(crate) fn build_editor_args(
    custom_command: &str,
    _path: &std::path::Path,
) -> (String, Vec<String>, bool) {
    // Returns (program, base_args, path_is_bare_arg)
    // path_is_bare_arg = true means append <path> as a bare final argument
    // path_is_bare_arg = false means the path is incorporated into args already
    let parts: Vec<&str> = custom_command.split_whitespace().collect();

    if let Some((program, rest)) = parts.split_first() {
        match program.to_ascii_lowercase().as_str() {
            // VS Code: path appended as bare arg by caller via path_is_bare_arg
            "code" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // JetBrains IDEs: path as bare arg
            "idea" | "idea64" | "idea.exe" | "pycharm" | "pycharm64" | "pycharm.exe"
            | "webstorm" | "webstorm.exe" | "rider" | "rider.exe" | "goland" | "goland.exe"
            | "datagrip" | "datagrip.exe" | "phpstorm" | "phpstorm.exe" | "rubymine"
            | "rubymine.exe" | "clion" | "clion.exe" | "fleet" | "fleet.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // Vim/Neovim: path as bare arg (vim file, nvim file)
            "vim" | "vim.exe" | "vi" | "nvim" | "nvim.exe" | "gvim" | "gvim.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // Emacs: path as bare arg (emacs file) or --directory for folder
            "emacs" | "emacs.exe" | "emacsclient" | "emacsclient.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // Sublime Text: path as bare arg
            "subl" | "sublime" | "sublime_text" | "sublime_text.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // Atom: path as bare arg
            "atom" | "atom.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            // Notepad++: path as bare arg
            "notepad++" | "notepad++.exe" => {
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
            _ => {
                // Unknown editor: treat as bare-arg (most editors accept path as final arg)
                let args = rest.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
                (program.to_string(), args, true)
            }
        }
    } else {
        ("code".to_string(), vec![], true)
    }
}

pub(crate) fn open_terminal(path: &std::path::Path, custom_command: &str) {
    let path = path.to_path_buf();
    let custom_command = custom_command.to_string();
    std::thread::spawn(move || {
        if !custom_command.is_empty() {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x00000008;
                const CREATE_NEW_CONSOLE: u32 = 0x00000010;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

                let (program, args) = build_terminal_args(&custom_command, &path);
                let mut cmd = std::process::Command::new(&program);
                cmd.args(&args).current_dir(&path);

                if is_console_terminal(&program) {
                    cmd.creation_flags(CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP);
                } else {
                    cmd.stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                }

                if let Err(e) = cmd.spawn() {
                    eprintln!(
                        "[rgitui] Failed to open terminal '{}' with args {:?} in '{}': {}",
                        program,
                        args,
                        path.display(),
                        e
                    );
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                if !path.is_dir() {
                    eprintln!(
                        "[rgitui] Cannot open terminal: directory '{}' does not exist",
                        path.display()
                    );
                    return;
                }
                let parts: Vec<&str> = custom_command.split_whitespace().collect();
                if let Some((program, args)) = parts.split_first() {
                    if let Err(e) = std::process::Command::new(program)
                        .args(args)
                        .current_dir(&path)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        eprintln!(
                            "[rgitui] Failed to open terminal '{}' in '{}': {}",
                            program,
                            path.display(),
                            e
                        );
                    }
                }
            }
        } else {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x00000008;
                const CREATE_NEW_CONSOLE: u32 = 0x00000010;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

                let result = std::process::Command::new("wt.exe")
                    .arg("-d")
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
                    .spawn();

                if let Err(wt_err) = result {
                    eprintln!(
                        "[rgitui] Failed to open Windows Terminal in '{}': {} — trying cmd.exe",
                        path.display(),
                        wt_err
                    );
                    let fallback = std::process::Command::new("cmd.exe")
                        .args(["/K", "cd", "/d", &path.to_string_lossy()])
                        .creation_flags(CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP)
                        .spawn();

                    if let Err(cmd_err) = fallback {
                        eprintln!(
                            "[rgitui] Failed to open cmd.exe in '{}': {}",
                            path.display(),
                            cmd_err
                        );
                    }
                }
            }
            #[cfg(target_os = "macos")]
            {
                if let Err(e) = std::process::Command::new("open")
                    .arg("-a")
                    .arg("Terminal")
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    eprintln!(
                        "[rgitui] Failed to open Terminal.app in '{}': {}",
                        path.display(),
                        e
                    );
                }
            }
            #[cfg(target_os = "linux")]
            {
                if !path.is_dir() {
                    eprintln!(
                        "[rgitui] Cannot open terminal: directory '{}' does not exist",
                        path.display()
                    );
                    return;
                }
                // Try terminals in preference order. $TERMINAL env, then common
                // emulators. x-terminal-emulator is Debian-specific and missing
                // on NixOS and other distros.
                let candidates: Vec<String> = std::env::var("TERMINAL")
                    .into_iter()
                    .chain(
                        [
                            "kitty",
                            "alacritty",
                            "wezterm",
                            "foot",
                            "gnome-terminal",
                            "konsole",
                            "xfce4-terminal",
                            "x-terminal-emulator",
                            "xterm",
                        ]
                        .into_iter()
                        .map(String::from),
                    )
                    .collect();
                let mut launched = false;
                for term in &candidates {
                    let result = std::process::Command::new(term)
                        .current_dir(&path)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                    if result.is_ok() {
                        launched = true;
                        break;
                    }
                }
                if !launched {
                    eprintln!(
                        "[rgitui] Failed to open terminal emulator in '{}': none of {:?} found",
                        path.display(),
                        candidates
                    );
                }
            }
        }
    });
}

pub(crate) fn open_editor(path: &std::path::Path, custom_command: &str) {
    let path = path.to_path_buf();
    let custom_command = custom_command.to_string();
    std::thread::spawn(move || {
        if !custom_command.is_empty() {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;

                let (program, base_args, path_is_bare_arg) =
                    build_editor_args(&custom_command, &path);
                let mut cmd = std::process::Command::new(&program);
                cmd.args(&base_args).creation_flags(CREATE_NO_WINDOW);
                if path_is_bare_arg {
                    cmd.arg(&path);
                }
                if let Err(e) = cmd.spawn() {
                    eprintln!(
                        "[rgitui] Failed to open editor '{}' for '{}': {}",
                        program,
                        path.display(),
                        e
                    );
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let (program, base_args, path_is_bare_arg) =
                    build_editor_args(&custom_command, &path);
                if !program.is_empty() {
                    let cwd = path.parent().unwrap_or(&path);
                    let mut cmd = std::process::Command::new(&program);
                    cmd.args(&base_args).current_dir(cwd);
                    if path_is_bare_arg {
                        cmd.arg(&path);
                    }
                    if let Err(e) = cmd.spawn() {
                        eprintln!(
                            "[rgitui] Failed to open editor '{}' in '{}': {}",
                            program,
                            cwd.display(),
                            e
                        );
                    }
                }
            }
        } else {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;

                if let Err(e) = std::process::Command::new("code")
                    .arg(&path)
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                {
                    eprintln!(
                        "[rgitui] Failed to open VS Code in '{}': {}",
                        path.display(),
                        e
                    );
                }
            }
            #[cfg(target_os = "macos")]
            {
                if let Err(e) = std::process::Command::new("code")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("open")
                            .arg("-a")
                            .arg("TextEdit")
                            .arg(&path)
                            .spawn()
                    })
                {
                    eprintln!(
                        "[rgitui] Failed to open editor in '{}': {}",
                        path.display(),
                        e
                    );
                }
            }
            #[cfg(target_os = "linux")]
            {
                if let Err(e) = std::process::Command::new("code")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| std::process::Command::new("xdg-open").arg(&path).spawn())
                {
                    eprintln!(
                        "[rgitui] Failed to open editor in '{}': {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    });
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod terminal_args_tests {
    use super::*;

    fn winpath(s: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(s)
    }

    #[test]
    fn cmd_custom_command_uses_cd_args() {
        let path = winpath("C:\\Projects\\myrepo");
        let (program, args) = build_terminal_args("cmd", &path);
        assert_eq!(program, "cmd");
        assert_eq!(args, &["/K", "cd", "/d", "C:\\Projects\\myrepo"]);
    }

    #[test]
    fn cmd_with_uppercase_variant() {
        let path = winpath("D:\\Work");
        let (program, args) = build_terminal_args("CMD", &path);
        assert_eq!(program, "CMD");
        assert_eq!(args, &["/K", "cd", "/d", "D:\\Work"]);
    }

    #[test]
    fn powershell_custom_command_uses_noexit_cd() {
        let path = winpath("C:\\Repos\\app");
        let (program, args) = build_terminal_args("powershell", &path);
        assert_eq!(program, "powershell");
        assert_eq!(args, &["-NoExit", "-Command", "cd 'C:\\Repos\\app'"]);
    }

    #[test]
    fn pwsh_short_name() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("pwsh", &path);
        assert_eq!(program, "pwsh");
        assert!(args.starts_with(&["-NoExit".to_string(), "-Command".to_string()]));
    }

    #[test]
    fn wt_custom_command_uses_dash_d() {
        let path = winpath("E:\\Code\\rgitui");
        let (program, args) = build_terminal_args("wt", &path);
        assert_eq!(program, "wt");
        assert_eq!(args, &["-d", "E:\\Code\\rgitui"]);
    }

    #[test]
    fn wt_with_extra_flags() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("wt --title MyTitle", &path);
        assert_eq!(program, "wt");
        assert_eq!(args, &["--title", "MyTitle", "-d", "C:\\test"]);
    }

    #[test]
    fn alacritty_uses_working_directory() {
        let path = winpath("C:\\AlacrittyTest");
        let (program, args) = build_terminal_args("alacritty", &path);
        assert_eq!(program, "alacritty");
        assert_eq!(args, &["--working-directory", "C:\\AlacrittyTest"]);
    }

    #[test]
    fn alacritty_with_profile_flag() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("alacritty -o AlwaysUsePipeFrontend=true", &path);
        assert_eq!(program, "alacritty");
        assert_eq!(
            args,
            &[
                "-o",
                "AlwaysUsePipeFrontend=true",
                "--working-directory",
                "C:\\test"
            ]
        );
    }

    #[test]
    fn wezterm_uses_cwd() {
        let path = winpath("C:\\WeztermTest");
        let (program, args) = build_terminal_args("wezterm", &path);
        assert_eq!(program, "wezterm");
        assert_eq!(args, &["--cwd", "C:\\WeztermTest"]);
    }

    #[test]
    fn kitty_uses_directory() {
        let path = winpath("C:\\KittyTest");
        let (program, args) = build_terminal_args("kitty", &path);
        assert_eq!(program, "kitty");
        assert_eq!(args, &["--directory", "C:\\KittyTest"]);
    }

    #[test]
    fn unknown_terminal_appends_path_as_bare_arg() {
        let path = winpath("C:\\UnknownTerminal");
        let (program, args) = build_terminal_args("mystic_term", &path);
        assert_eq!(program, "mystic_term");
        assert_eq!(args, &["C:\\UnknownTerminal"]);
    }

    #[test]
    fn unknown_terminal_with_flags() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("custom --flag value", &path);
        assert_eq!(program, "custom");
        assert_eq!(args, &["--flag", "value", "C:\\test"]);
    }

    #[test]
    fn path_with_single_quotes_escaped_in_powershell() {
        let path = winpath("C:\\O'Reilly\\Test");
        let (program, args) = build_terminal_args("powershell", &path);
        assert_eq!(program, "powershell");
        // Single quotes in path should be doubled: ' becomes ''
        assert!(args[2].contains("''"));
    }

    #[test]
    fn empty_command_returns_fallback() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("", &path);
        assert_eq!(program, "cmd.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn cmd_exe_explicit() {
        let path = winpath("C:\\test");
        let (program, args) = build_terminal_args("cmd.exe", &path);
        assert_eq!(program, "cmd.exe");
        assert_eq!(args, &["/K", "cd", "/d", "C:\\test"]);
    }

    // ─── build_editor_args tests ───────────────────────────────────────────────

    #[test]
    fn vscode_editor_appends_path_as_bare_arg() {
        let path = winpath("C:\\Projects\\myrepo");
        let (program, args, path_is_bare) = build_editor_args("code", &path);
        assert_eq!(program, "code");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn vscode_editor_with_wait_flag() {
        let path = winpath("C:\\Projects\\myrepo");
        let (program, args, path_is_bare) = build_editor_args("code --wait", &path);
        assert_eq!(program, "code");
        assert_eq!(args, &["--wait"]);
        assert!(path_is_bare);
    }

    #[test]
    fn vscode_editor_unknown_flags_passed_through() {
        let path = winpath("C:\\test");
        let (program, args, path_is_bare) =
            build_editor_args("code --disable-extensions --new-window", &path);
        assert_eq!(program, "code");
        assert_eq!(args, &["--disable-extensions", "--new-window"]);
        assert!(path_is_bare);
    }

    #[test]
    fn jetbrains_idea_appends_path() {
        let path = winpath("C:\\Projects\\myrepo");
        let (program, args, path_is_bare) = build_editor_args("idea", &path);
        assert_eq!(program, "idea");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn jetbrains_pycharm_with_project_flag() {
        let path = winpath("C:\\Projects\\myrepo");
        let (program, args, path_is_bare) = build_editor_args("pycharm --new-project", &path);
        assert_eq!(program, "pycharm");
        assert_eq!(args, &["--new-project"]);
        assert!(path_is_bare);
    }

    #[test]
    fn vim_editor_appends_path() {
        let path = winpath("C:\\test\\file.rs");
        let (program, args, path_is_bare) = build_editor_args("vim", &path);
        assert_eq!(program, "vim");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn neovim_editor_appends_path() {
        let path = winpath("C:\\test\\file.txt");
        let (program, args, path_is_bare) = build_editor_args("nvim", &path);
        assert_eq!(program, "nvim");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn sublime_editor_appends_path() {
        let path = winpath("C:\\test\\file.txt");
        let (program, args, path_is_bare) = build_editor_args("subl", &path);
        assert_eq!(program, "subl");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn unknown_editor_appends_path_as_bare_arg() {
        let path = winpath("C:\\test\\file.txt");
        let (program, args, path_is_bare) = build_editor_args("fancy_editor", &path);
        assert_eq!(program, "fancy_editor");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }

    #[test]
    fn unknown_editor_with_flags() {
        let path = winpath("C:\\test");
        let (program, args, path_is_bare) = build_editor_args("custom --flag value", &path);
        assert_eq!(program, "custom");
        assert_eq!(args, &["--flag", "value"]);
        assert!(path_is_bare);
    }

    #[test]
    fn empty_editor_returns_code_fallback() {
        let path = winpath("C:\\test");
        let (program, args, path_is_bare) = build_editor_args("", &path);
        assert_eq!(program, "code");
        assert!(args.is_empty());
        assert!(path_is_bare);
    }
}

// Cross-platform tests for build_terminal_args (runs on all platforms).
// On non-Windows, the known-terminal match arms are not available, so all
// custom commands fall through to the "append path as bare arg" behavior.
// This verifies that non-Windows platforms get a predictable, testable interface.
#[cfg(not(target_os = "windows"))]
#[cfg(test)]
mod terminal_args_cross_platform_tests {
    use super::*;

    fn path(s: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(s)
    }

    #[test]
    fn unknown_terminal_appends_path_as_bare_arg_on_linux() {
        // On Linux, all terminals go through the unknown-terminal path since
        // the Windows-specific match arms (wt, cmd, etc.) are cfg-gated out.
        let path = path("/home/user/repo");
        let (program, args) = build_terminal_args("gnome-terminal", &path);
        assert_eq!(program, "gnome-terminal");
        assert_eq!(args, &["/home/user/repo"]);
    }

    #[test]
    fn unknown_terminal_with_flags_appends_path() {
        let path = path("/home/user/repo");
        let (program, args) = build_terminal_args("konsole --workdir", &path);
        assert_eq!(program, "konsole");
        assert_eq!(args, &["--workdir", "/home/user/repo"]);
    }

    #[test]
    fn empty_command_returns_fallback_on_linux() {
        let path = path("/home/user/repo");
        let (program, args) = build_terminal_args("", &path);
        assert_eq!(program, "cmd.exe"); // fallback program (platform-neutral fallback)
        assert!(args.is_empty());
    }

    #[test]
    fn xterm_explicit_command() {
        let path = path("/tmp/test");
        let (program, args) = build_terminal_args("xterm", &path);
        assert_eq!(program, "xterm");
        assert_eq!(args, &["/tmp/test"]);
    }

    #[test]
    fn kitty_on_linux_appends_directory_flag() {
        // Kitty is only cfg-gated on Windows; on Linux it's treated as unknown.
        // This tests that Linux gets consistent unknown-terminal behavior.
        let path = path("/home/user/kitty-test");
        let (program, args) = build_terminal_args("kitty", &path);
        assert_eq!(program, "kitty");
        assert_eq!(args, &["/home/user/kitty-test"]);
    }
}
