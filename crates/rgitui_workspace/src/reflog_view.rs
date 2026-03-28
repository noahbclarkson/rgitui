use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent, Pixels, Point, Render,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::ReflogEntryInfo;
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

// Use Clock icon for reflog since History icon doesn't exist
const REFLOG_ICON: IconName = IconName::Clock;

/// Events emitted by the reflog view.
#[derive(Debug, Clone)]
pub enum ReflogViewEvent {
    CommitSelected(String),
    Dismissed,
    CopyOID(String),
    ResetHard(String),
    ResetSoft(String),
    ResetMixed(String),
}

/// A reflog viewer panel that shows HEAD reflog entries.
pub struct ReflogView {
    entries: Arc<Vec<ReflogEntryInfo>>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    selected_row: Option<usize>,
    highlighted_row: Option<usize>,
    /// Right-click context menu state: (entry_index, screen_position).
    context_menu: Option<(usize, Point<Pixels>)>,
}

impl EventEmitter<ReflogViewEvent> for ReflogView {}

impl ReflogView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            selected_row: None,
            highlighted_row: None,
            context_menu: None,
        }
    }

    pub fn set_entries(&mut self, entries: Vec<ReflogEntryInfo>, cx: &mut Context<Self>) {
        self.entries = Arc::new(entries);
        self.highlighted_row = None;
        self.selected_row = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.entries = Arc::new(Vec::new());
        self.highlighted_row = None;
        self.selected_row = None;
        cx.notify();
    }

    pub fn has_data(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    fn dismiss_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.is_some() {
            self.context_menu = None;
            cx.notify();
        }
    }

    fn show_context_menu(&mut self, index: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some((index, position));
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let count = self.entries.len();
        if count == 0 {
            return;
        }

        match key {
            "j" | "down" => {
                let next = self
                    .highlighted_row
                    .map(|r| (r + 1).min(count - 1))
                    .unwrap_or(0);
                self.highlighted_row = Some(next);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "k" | "up" => {
                let prev = self
                    .highlighted_row
                    .map(|r| r.saturating_sub(1))
                    .unwrap_or(0);
                self.highlighted_row = Some(prev);
                self.scroll_handle
                    .scroll_to_item(prev, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "enter" => {
                if let Some(row) = self.highlighted_row {
                    if let Some(entry) = self.entries.get(row) {
                        let oid = entry.new_oid.to_string();
                        cx.emit(ReflogViewEvent::CommitSelected(oid));
                    }
                }
                cx.stop_propagation();
            }
            "escape" => {
                cx.emit(ReflogViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                if modifiers.shift {
                    // G (Shift+G) — jump to last entry
                    let last = count - 1;
                    self.highlighted_row = Some(last);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Bottom);
                } else {
                    // g — jump to first entry
                    self.highlighted_row = Some(0);
                    self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                }
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();

        div()
            .id("reflog-view")
            .v_flex()
            .size_full()
            .bg(colors.editor_background)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(REFLOG_ICON)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Reflog")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div().flex_1().flex().items_center().justify_center().child(
                    div()
                        .v_flex()
                        .gap(px(8.))
                        .items_center()
                        .px(px(24.))
                        .py(px(16.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_background)
                        .child(
                            Icon::new(REFLOG_ICON)
                                .size(IconSize::Large)
                                .color(Color::Placeholder),
                        )
                        .child(
                            Label::new("No reflog entries")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Reflog tracks HEAD changes")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for ReflogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.entries.is_empty() {
            return self.render_empty_state(cx);
        }

        let entries = self.entries.clone();
        let count = entries.len();
        let view: WeakEntity<ReflogView> = cx.weak_entity();
        let view_for_dismiss = view.clone();

        let editor_bg = colors.editor_background;
        let text_color = colors.text;
        let text_muted = colors.text_muted;
        let border_variant = colors.border_variant;
        let text_accent = colors.text_accent;

        let compactness = cx.global::<SettingsState>().settings().compactness;
        let row_height = compactness.spacing(24.0);
        let highlighted_row = self.highlighted_row;
        let selected_row = self.selected_row;

        let selected_bg = colors.ghost_element_selected;
        let highlight_bg = colors.ghost_element_active;

        let list = uniform_list(
            "reflog-entries",
            count,
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let entry = &entries[i];

                        let is_highlighted = highlighted_row == Some(i);
                        let is_selected = selected_row == Some(i);
                        let effective_bg = if is_selected {
                            selected_bg
                        } else if is_highlighted {
                            highlight_bg
                        } else {
                            editor_bg
                        };

                        // Clone data to avoid lifetime issues
                        let new_sha_display: SharedString = entry.new_short_id.clone().into();
                        let old_sha_display: SharedString = if entry.old_short_id.is_empty() {
                            "0000000".into()
                        } else {
                            entry.old_short_id.clone().into()
                        };
                        let message_str = entry
                            .message
                            .clone()
                            .unwrap_or_else(|| "(no message)".to_string());
                        let message_display: SharedString = message_str.clone().into();
                        let committer_display: SharedString = entry.committer.clone().into();
                        let time_display: SharedString =
                            super::time::format_relative_time_abbreviated(entry.time.timestamp())
                                .into();

                        let tooltip_text: SharedString = format!(
                            "{} -> {}\n{}\n{}",
                            entry.old_short_id, entry.new_short_id, &message_str, entry.committer,
                        )
                        .into();

                        let view_click = view.clone();
                        let view_entry = view.clone();
                        let view_ctx_menu = view.clone();
                        let entry_oid = entry.new_oid.to_string();

                        div()
                            .id(ElementId::NamedInteger("reflog-entry".into(), i as u64))
                            .h_flex()
                            .h(px(row_height))
                            .w_full()
                            .bg(effective_bg)
                            .border_b_1()
                            .border_color(border_variant)
                            .tooltip(Tooltip::text(tooltip_text))
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
                                    view_click
                                        .update(cx, |this, cx| {
                                            this.highlighted_row = Some(i);
                                            this.selected_row = Some(i);
                                            this.dismiss_context_menu(cx);
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                move |event: &MouseDownEvent,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    view_ctx_menu
                                        .update(cx, |this, cx| {
                                            this.dismiss_context_menu(cx);
                                            this.highlighted_row = Some(i);
                                            this.selected_row = Some(i);
                                            this.show_context_menu(i, event.position, cx);
                                        })
                                        .ok();
                                },
                            )
                            .child(
                                div()
                                    .id(ElementId::NamedInteger("reflog-new-sha".into(), i as u64))
                                    .w(px(70.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .pl(px(8.))
                                    .text_xs()
                                    .text_color(text_accent)
                                    .cursor_pointer()
                                    .on_click({
                                        let entry_oid = entry_oid.clone();
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            let oid = entry_oid.clone();
                                            view_entry
                                                .update(cx, |_this, cx| {
                                                    cx.emit(ReflogViewEvent::CommitSelected(oid));
                                                })
                                                .ok();
                                        }
                                    })
                                    .child(new_sha_display),
                            )
                            .child(
                                div()
                                    .w(px(60.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(old_sha_display),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_color)
                                    .overflow_x_hidden()
                                    .child(message_display),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .overflow_x_hidden()
                                    .child(committer_display),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .child(time_display),
                            )
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        let view_dismiss = view_for_dismiss;
        div()
            .id("reflog-view")
            .track_focus(&self.focus_handle)
            .key_context("ReflogView")
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(
                MouseButton::Left,
                move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    view_dismiss
                        .update(cx, |this, cx| {
                            this.dismiss_context_menu(cx);
                        })
                        .ok();
                },
            )
            .v_flex()
            .size_full()
            .bg(editor_bg)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
                    .items_center()
                    .bg(colors.toolbar_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(REFLOG_ICON)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("Reflog")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(SharedString::from("HEAD"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new(SharedString::from(format!("{} entries", count)))
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .child(list)
            .child(self.render_context_menu(window, cx))
            .into_any_element()
    }
}

impl ReflogView {
    fn render_context_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (entry_idx, pos) = match self.context_menu {
            Some((idx, p)) => (idx, p),
            None => return div().into_any_element(),
        };

        let entry = match self.entries.get(entry_idx) {
            Some(e) => e,
            None => return div().into_any_element(),
        };

        let oid = entry.new_oid.to_string();
        let colors = cx.colors();
        let weak = cx.weak_entity();

        // Clamp menu position to stay within window bounds.
        let menu_w = px(200.);
        let menu_h = px(120.);
        let window_bounds = window.bounds();
        let max_x = window_bounds.size.width - menu_w;
        let max_y = window_bounds.size.height - menu_h;
        let clamped_x = if pos.x > max_x { max_x } else { pos.x };
        let clamped_y = if pos.y > max_y { max_y } else { pos.y };

        let menu_bg = colors.elevated_surface_background;
        let menu_border = colors.border;
        let menu_hover = colors.ghost_element_hover;
        let menu_active = colors.ghost_element_active;

        let menu_items: Vec<(&str, IconName)> = vec![
            ("Reset (hard)", IconName::Trash),
            ("Reset (soft)", IconName::Undo),
            ("Reset (mixed)", IconName::Refresh),
            ("Copy OID", IconName::Copy),
        ];

        let mut menu = div()
            .id("reflog-context-menu")
            .absolute()
            .left(clamped_x)
            .top(clamped_y)
            .v_flex()
            .min_w(px(200.))
            .py(px(3.))
            .bg(menu_bg)
            .border_1()
            .border_color(menu_border)
            .rounded(px(6.))
            .elevation_3(cx)
            .on_mouse_down(
                MouseButton::Left,
                |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                },
            );

        for (idx, (label_text, icon_name)) in menu_items.into_iter().enumerate() {
            let label: SharedString = label_text.into();
            let icon = icon_name;

            if idx > 0 {
                menu = menu.child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .my(px(2.))
                        .mx(px(8.))
                        .bg(colors.border_variant),
                );
            }

            let mut item = div()
                .id(ElementId::NamedInteger(
                    "reflog-ctx-action".into(),
                    idx as u64,
                ))
                .h_flex()
                .w_full()
                .h(px(26.))
                .px(px(8.))
                .mx(px(4.))
                .gap(px(6.))
                .items_center()
                .cursor_pointer()
                .rounded(px(3.))
                .hover(move |s| s.bg(menu_hover))
                .active(move |s| s.bg(menu_active));

            match idx {
                0 => {
                    let w = weak.clone();
                    let oid = oid.clone();
                    item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        w.update(cx, |this, cx| {
                            this.context_menu = None;
                            cx.notify();
                            cx.emit(ReflogViewEvent::ResetHard(oid.clone()));
                        })
                        .ok();
                    });
                }
                1 => {
                    let w = weak.clone();
                    let oid = oid.clone();
                    item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        w.update(cx, |this, cx| {
                            this.context_menu = None;
                            cx.notify();
                            cx.emit(ReflogViewEvent::ResetSoft(oid.clone()));
                        })
                        .ok();
                    });
                }
                2 => {
                    let w = weak.clone();
                    let oid = oid.clone();
                    item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        w.update(cx, |this, cx| {
                            this.context_menu = None;
                            cx.notify();
                            cx.emit(ReflogViewEvent::ResetMixed(oid.clone()));
                        })
                        .ok();
                    });
                }
                3 => {
                    let w = weak.clone();
                    let oid = oid.clone();
                    item = item.on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        w.update(cx, |this, cx| {
                            this.context_menu = None;
                            cx.notify();
                            cx.emit(ReflogViewEvent::CopyOID(oid.clone()));
                        })
                        .ok();
                    });
                }
                _ => {}
            }

            menu = menu.child(
                item.child(Icon::new(icon).size(IconSize::XSmall).color(Color::Muted))
                    .child(
                        Label::new(label)
                            .size(LabelSize::XSmall)
                            .color(Color::Default),
                    ),
            );
        }

        menu.into_any_element()
    }
}
