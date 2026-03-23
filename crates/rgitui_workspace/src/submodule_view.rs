use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, Context, ElementId, EventEmitter, FocusHandle, KeyDownEvent,
    ListSizingBehavior, MouseButton, MouseDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use rgitui_git::SubmoduleInfo;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

const SUBMODULE_ICON: IconName = IconName::GitBranch;

/// Events emitted by the submodule view.
#[derive(Debug, Clone)]
pub enum SubmoduleViewEvent {
    InitSubmodule(String),
    UpdateSubmodule(String),
    InitAll,
    UpdateAll,
    Dismissed,
}

/// A submodule viewer panel that shows submodule status.
pub struct SubmoduleView {
    submodules: Arc<Vec<SubmoduleInfo>>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    highlighted_row: Option<usize>,
}

impl EventEmitter<SubmoduleViewEvent> for SubmoduleView {}

impl SubmoduleView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            submodules: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            highlighted_row: None,
        }
    }

    pub fn set_submodules(&mut self, submodules: Vec<SubmoduleInfo>, cx: &mut Context<Self>) {
        self.submodules = Arc::new(submodules);
        self.highlighted_row = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.submodules = Arc::new(Vec::new());
        self.highlighted_row = None;
        cx.notify();
    }

    pub fn has_data(&self) -> bool {
        !self.submodules.is_empty()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let count = self.submodules.len();
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
            "escape" => {
                cx.emit(SubmoduleViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                self.highlighted_row = Some(0);
                self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn format_status(sub: &SubmoduleInfo) -> String {
        if !sub.is_initialized {
            return "not initialized".to_string();
        }

        match (&sub.head_oid, &sub.workdir_oid) {
            (Some(head), Some(workdir)) if head == workdir => "up to date".to_string(),
            (Some(_), Some(_)) => "modified".to_string(),
            (Some(_), None) => "not checked out".to_string(),
            (None, _) => "no commit".to_string(),
        }
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();

        div()
            .id("submodule-view")
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
                        Icon::new(SUBMODULE_ICON)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Submodules")
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
                            Icon::new(SUBMODULE_ICON)
                                .size(IconSize::Large)
                                .color(Color::Placeholder),
                        )
                        .child(
                            Label::new("No submodules found")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Add submodules to your repository")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for SubmoduleView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.submodules.is_empty() {
            return self.render_empty_state(cx);
        }

        let submodules = self.submodules.clone();
        let count = submodules.len();
        let view: WeakEntity<SubmoduleView> = cx.weak_entity();

        let editor_bg = colors.editor_background;
        let border_variant = colors.border_variant;
        let text_color = colors.text;
        let text_muted = colors.text_muted;
        let text_accent = colors.text_accent;

        let row_height = 28.0_f32;
        let highlighted_row = self.highlighted_row;

        let highlight_bg = colors.ghost_element_active;

        let list = uniform_list(
            "submodule-entries",
            count,
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let sub = &submodules[i];

                        let is_highlighted = highlighted_row == Some(i);
                        let effective_bg = if is_highlighted {
                            highlight_bg
                        } else {
                            editor_bg
                        };

                        let name_display: SharedString = sub.name.clone().into();
                        let path_display: SharedString =
                            sub.path.to_string_lossy().to_string().into();
                        let status_str = Self::format_status(sub);
                        let status_display: SharedString = status_str.clone().into();

                        // Determine status color (pre-resolved Hsla values)
                        let status_color = match status_str.as_str() {
                            "up to date" => text_accent,
                            _ => text_muted,
                        };

                        let commit_display: SharedString = sub
                            .workdir_oid
                            .as_ref()
                            .map(|oid| format!("{:.7}", oid))
                            .unwrap_or_else(|| "—".to_string())
                            .into();

                        let branch_display: SharedString = sub
                            .branch
                            .clone()
                            .unwrap_or_else(|| "default".to_string())
                            .into();

                        let tooltip_text: SharedString = format!(
                            "Name: {}\nPath: {}\nURL: {}\nStatus: {}",
                            sub.name,
                            sub.path.to_string_lossy(),
                            sub.url,
                            status_str
                        )
                        .into();

                        let view_click = view.clone();

                        div()
                            .id(ElementId::NamedInteger("submodule-entry".into(), i as u64))
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
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )
                            // Name
                            .child(
                                div()
                                    .w(px(140.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .px(px(8.))
                                    .text_xs()
                                    .text_color(text_color)
                                    .overflow_x_hidden()
                                    .child(name_display),
                            )
                            // Path
                            .child(
                                div()
                                    .w(px(120.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .overflow_x_hidden()
                                    .child(path_display),
                            )
                            // Status
                            .child(
                                div()
                                    .w(px(100.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(status_color)
                                    .child(status_display),
                            )
                            // Commit
                            .child(
                                div()
                                    .w(px(70.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_r_1()
                                    .border_color(border_variant)
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .font_family("monospace")
                                    .child(commit_display),
                            )
                            // Branch
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(text_muted)
                                    .overflow_x_hidden()
                                    .child(branch_display),
                            )
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_grow()
        .track_scroll(&self.scroll_handle);

        div()
            .id("submodule-view")
            .track_focus(&self.focus_handle)
            .key_context("SubmoduleView")
            .on_key_down(cx.listener(Self::handle_key_down))
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
                        Icon::new(SUBMODULE_ICON)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("Submodules")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Default),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new(SharedString::from(format!("{} submodules", count)))
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
