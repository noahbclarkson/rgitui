use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    InteractiveElement, KeyDownEvent, ListSizingBehavior, MouseButton, MouseDownEvent,
    ParentElement, Render, ScrollStrategy, SharedString, Styled, UniformListScrollHandle,
    WeakEntity, Window,
};
use rgitui_git::{BisectDecision, BisectLogEntry};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, Tooltip};

const BISECT_ICON: IconName = IconName::GitMerge;

/// Events emitted by the bisect view.
#[derive(Debug, Clone)]
pub enum BisectViewEvent {
    /// User clicked a commit — scroll graph to it.
    CommitSelected(String),
    /// User pressed Escape — dismiss the panel.
    Dismissed,
    /// Copy the OID to clipboard.
    CopyOID(String),
    /// Mark the commit as good.
    Good(String),
    /// Mark the commit as bad.
    Bad(String),
    /// Skip this commit.
    Skip(String),
}

impl BisectViewEvent {
    fn decision(&self) -> Option<&str> {
        match self {
            Self::Good(_) => Some("good"),
            Self::Bad(_) => Some("bad"),
            Self::Skip(_) => Some("skip"),
            _ => None,
        }
    }
}

/// A bisect viewer panel that shows the git bisect log.
pub struct BisectView {
    entries: Arc<Vec<BisectLogEntry>>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    highlighted_row: Option<usize>,
}

impl EventEmitter<BisectViewEvent> for BisectView {}

impl BisectView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: Arc::new(Vec::new()),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            highlighted_row: None,
        }
    }

    pub fn set_entries(&mut self, entries: Vec<BisectLogEntry>, cx: &mut Context<Self>) {
        self.entries = Arc::new(entries);
        self.highlighted_row = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.entries = Arc::new(Vec::new());
        self.highlighted_row = None;
        cx.notify();
    }

    pub fn has_data(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Compute good / bad / skip / start counts from the entries list.
    fn decision_counts(&self) -> (usize, usize, usize, usize) {
        let mut good = 0;
        let mut bad = 0;
        let mut skip = 0;
        let mut start = 0;
        for e in self.entries.iter() {
            match e.decision {
                BisectDecision::Good => good += 1,
                BisectDecision::Bad => bad += 1,
                BisectDecision::Skip => skip += 1,
                BisectDecision::Start => start += 1,
            }
        }
        (good, bad, skip, start)
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
                        cx.emit(BisectViewEvent::CommitSelected(entry.sha.clone()));
                    }
                }
                cx.stop_propagation();
            }
            "escape" => {
                cx.emit(BisectViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                if modifiers.shift {
                    let last = count - 1;
                    self.highlighted_row = Some(last);
                    self.scroll_handle
                        .scroll_to_item(last, ScrollStrategy::Bottom);
                } else {
                    self.highlighted_row = Some(0);
                    self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                }
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn render_header(&self, cx: &mut Context<Self>, count: usize) -> gpui::Div {
        let colors = cx.colors();
        let (good, bad, skip, start) = self.decision_counts();

        // Estimate remaining steps: binary search over (bad - good - skip) range.
        // After k steps, range is halved. log2(range) steps remain.
        let range = good.max(bad).saturating_sub(1);
        let remaining = if range > 0 {
            (range as f64).log2().ceil() as usize
        } else {
            0
        };

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
                Icon::new(BISECT_ICON)
                    .size(IconSize::XSmall)
                    .color(Color::Accent),
            )
            .child(
                Label::new("Bisect")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Default),
            )
            .when(start > 0, |el| {
                el.child(
                    Label::new("started")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            })
            .child(div().flex_1())
            // Bad count
            .child(
                Label::new(format!("{} bad", bad))
                    .size(LabelSize::XSmall)
                    .color(if bad > 0 { Color::Error } else { Color::Muted }),
            )
            // Good count
            .child(
                Label::new(format!("{} good", good))
                    .size(LabelSize::XSmall)
                    .color(if good > 0 {
                        Color::Success
                    } else {
                        Color::Muted
                    }),
            )
            // Skip count
            .when(skip > 0, |el| {
                el.child(
                    Label::new(format!("{} skip", skip))
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                )
            })
            // Remaining steps
            .when(remaining > 0, |el| {
                el.child(
                    Label::new(format!("~{} steps left", remaining))
                        .size(LabelSize::XSmall)
                        .color(Color::Accent),
                )
            })
            // Entry count
            .child(
                Label::new(format!("{} entries", count))
                    .size(LabelSize::XSmall)
                    .color(Color::Placeholder),
            )
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let editor_bg = cx.colors().editor_background;
        let ghost_bg = cx.colors().ghost_element_background;
        div()
            .id("bisect-view")
            .v_flex()
            .size_full()
            .bg(editor_bg)
            .child(self.render_header(cx, 0))
            .child(
                div().flex_1().flex().items_center().justify_center().child(
                    div()
                        .v_flex()
                        .gap(px(8.))
                        .items_center()
                        .px(px(24.))
                        .py(px(16.))
                        .rounded(px(8.))
                        .bg(ghost_bg)
                        .child(
                            Icon::new(BISECT_ICON)
                                .size(IconSize::Large)
                                .color(Color::Placeholder),
                        )
                        .child(
                            Label::new("No bisect in progress")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Use 'Git: Bisect Start' to begin")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for BisectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entries.is_empty() {
            return self.render_empty_state(cx);
        }

        let entries = self.entries.clone();
        let count = entries.len();
        let view: WeakEntity<BisectView> = cx.weak_entity();

        let compactness = cx.global::<SettingsState>().settings().compactness;
        let row_height = compactness.spacing(24.0);
        let highlighted_row = self.highlighted_row;

        let editor_bg = cx.colors().editor_background;
        let text_color = cx.colors().text;
        let text_muted = cx.colors().text_muted;
        let border_variant = cx.colors().border_variant;
        let text_accent = cx.colors().text_accent;
        let highlight_bg = cx.colors().ghost_element_active;
        let error_hsla = cx.status().error;
        let success_hsla = cx.status().success;
        let warning_hsla = cx.status().warning;

        let list = uniform_list(
            "bisect-entries",
            count,
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let entry = &entries[i];

                        let is_highlighted = highlighted_row == Some(i);
                        let effective_bg = if is_highlighted {
                            highlight_bg
                        } else {
                            editor_bg
                        };

                        let sha_display: SharedString = entry.sha.clone().into();

                        let decision_str = match entry.decision {
                            BisectDecision::Start => "start",
                            BisectDecision::Good => "good",
                            BisectDecision::Bad => "bad",
                            BisectDecision::Skip => "skip",
                        };
                        let decision_display: SharedString = decision_str.into();

                        let decision_color = match entry.decision {
                            BisectDecision::Start => text_muted,
                            BisectDecision::Good => success_hsla,
                            BisectDecision::Bad => error_hsla,
                            BisectDecision::Skip => warning_hsla,
                        };

                        let subject_str = entry
                            .subject
                            .clone()
                            .unwrap_or_else(|| "(no subject)".to_string());
                        let subject_display: SharedString = subject_str.clone().into();

                        let tooltip_text: SharedString = format!(
                            "{} — {}\n{}",
                            &entry.sha[..entry.sha.len().min(7)],
                            decision_str,
                            &subject_str,
                        )
                        .into();

                        let view_click = view.clone();
                        let view_entry = view.clone();
                        let entry_sha = entry.sha.clone();

                        div()
                            .id(ElementId::NamedInteger("bisect-entry".into(), i as u64))
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
                            // SHA column
                            .child(
                                div()
                                    .w(px(80.))
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
                                        let sha = entry_sha.clone();
                                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                            view_entry
                                                .update(cx, |_, cx| {
                                                    cx.emit(BisectViewEvent::CommitSelected(
                                                        sha.clone(),
                                                    ));
                                                })
                                                .ok();
                                        }
                                    })
                                    .child(sha_display),
                            )
                            // Decision column
                            .child(
                                div()
                                    .w(px(60.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px(px(6.))
                                    .text_xs()
                                    .text_color(decision_color)
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(decision_display),
                            )
                            // Subject column
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
                                    .child(subject_display),
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
            .id("bisect-view")
            .track_focus(&self.focus_handle)
            .key_context("BisectView")
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(editor_bg)
            .child(self.render_header(cx, count))
            .child(list)
            .into_any_element()
    }
}
