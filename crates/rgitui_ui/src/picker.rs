//! A searchable, virtualized picker overlay.
//!
//! Used for the model list, where a closed pill row cannot work: OpenRouter
//! alone offers hundreds of models, and the two questions that actually decide
//! the choice — "is it cheap" and "does it take tools" — are facets, not
//! substrings.
//!
//! Filtering and ranking are pure functions over `[PickerRow]` so they are
//! testable without a display, per the convention in `CLAUDE.md`.

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    Focusable, FontWeight, KeyDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};

use crate::{fuzzy_score, Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub id: SharedString,
    pub primary: SharedString,
    pub secondary: Option<SharedString>,
    /// Right-hand column, e.g. `1M   $0.30/$2.50`.
    pub trailing: Option<SharedString>,
    pub badges: Vec<SharedString>,
    /// Which filter chips this row belongs to.
    pub facets: Vec<SharedString>,
    /// Rendered above the list and always selectable, even when filtered out.
    /// A pinned model that has left the catalogue must stay reachable.
    pub pinned_note: Option<SharedString>,
}

impl PickerRow {
    pub fn new(id: impl Into<SharedString>, primary: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            primary: primary.into(),
            secondary: None,
            trailing: None,
            badges: Vec::new(),
            facets: Vec::new(),
            pinned_note: None,
        }
    }

    pub fn secondary(mut self, secondary: impl Into<SharedString>) -> Self {
        self.secondary = Some(secondary.into());
        self
    }

    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badges.push(badge.into());
        self
    }

    pub fn facet(mut self, facet: impl Into<SharedString>) -> Self {
        self.facets.push(facet.into());
        self
    }

    pub fn pinned_note(mut self, note: impl Into<SharedString>) -> Self {
        self.pinned_note = Some(note.into());
        self
    }
}

/// A filter chip above the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerChip {
    pub id: SharedString,
    pub label: SharedString,
}

impl PickerChip {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PickerEvent {
    Selected(SharedString),
    Dismissed,
    RefreshRequested,
    /// The active filter chip changed. The owner re-supplies rows.
    ChipChanged(SharedString),
}

/// Row height, in pixels. Two lines of text plus padding.
const ROW_HEIGHT: f32 = 44.0;

/// Rank rows against a query. Pure.
///
/// An empty query preserves the caller's order, which for a server-sorted
/// catalogue is already the most useful ranking there is.
pub fn rank_rows<'a>(rows: &'a [PickerRow], query: &str) -> Vec<&'a PickerRow> {
    let query = query.trim();
    if query.is_empty() {
        return rows.iter().collect();
    }
    let mut scored: Vec<(usize, &PickerRow)> = rows
        .iter()
        .filter_map(|row| {
            let score = fuzzy_score(query, &row.id)
                .into_iter()
                .chain(fuzzy_score(query, &row.primary))
                .max()?;
            Some((score, row))
        })
        .collect();
    // Descending: the best score first.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, row)| row).collect()
}

/// Keep only rows carrying `facet`. An empty facet keeps everything, which is
/// what the "All" chip means.
pub fn rows_in_facet<'a>(rows: &'a [&'a PickerRow], facet: &str) -> Vec<&'a PickerRow> {
    if facet.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .copied()
        .filter(|row| row.facets.iter().any(|value| value == facet))
        .collect()
}

pub struct Picker {
    rows: Vec<PickerRow>,
    chips: Vec<PickerChip>,
    active_chip: SharedString,
    selected_id: Option<SharedString>,
    highlighted: usize,
    query_editor: gpui::Entity<TextInput>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    footer_note: Option<SharedString>,
    status_note: Option<SharedString>,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Focusable for Picker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Picker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let query_editor = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Search models…");
            input
        });

        cx.subscribe(
            &query_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(_) => {
                    this.highlighted = 0;
                    this.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                TextInputEvent::Submit => this.commit_highlighted(cx),
                TextInputEvent::Blurred => {}
            },
        )
        .detach();

        Self {
            rows: Vec::new(),
            chips: Vec::new(),
            active_chip: SharedString::default(),
            selected_id: None,
            highlighted: 0,
            query_editor,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            footer_note: None,
            status_note: None,
        }
    }

    pub fn set_rows(&mut self, rows: Vec<PickerRow>, cx: &mut Context<Self>) {
        self.rows = rows;
        self.highlighted = 0;
        cx.notify();
    }

    pub fn set_chips(&mut self, chips: Vec<PickerChip>, cx: &mut Context<Self>) {
        self.chips = chips;
        cx.notify();
    }

    pub fn set_active_chip(&mut self, chip: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.active_chip = chip.into();
        self.highlighted = 0;
        cx.notify();
    }

    pub fn set_selected(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        self.selected_id = id;
        cx.notify();
    }

    /// A short line in the footer, e.g. `312 models · updated 3 h ago`.
    pub fn set_footer_note(&mut self, note: Option<SharedString>, cx: &mut Context<Self>) {
        self.footer_note = note;
        cx.notify();
    }

    /// A warning shown above the list — a stale catalogue, or a fetch that
    /// failed. Never blanks the list.
    pub fn set_status_note(&mut self, note: Option<SharedString>, cx: &mut Context<Self>) {
        self.status_note = note;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query_editor
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn clear_query(&mut self, cx: &mut Context<Self>) {
        self.query_editor.update(cx, |input, cx| input.clear(cx));
        self.highlighted = 0;
        cx.notify();
    }

    /// The rows currently visible, after chip and query filtering.
    fn visible_rows(&self, cx: &Context<Self>) -> Vec<PickerRow> {
        let query = self.query_editor.read(cx).text().to_string();
        let ranked = rank_rows(&self.rows, &query);
        rows_in_facet(&ranked, &self.active_chip)
            .into_iter()
            .cloned()
            .collect()
    }

    fn commit_highlighted(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible_rows(cx);
        let Some(row) = visible.get(self.highlighted) else {
            return;
        };
        let id = row.id.clone();
        self.selected_id = Some(id.clone());
        cx.emit(PickerEvent::Selected(id));
        cx.notify();
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.visible_rows(cx).len();
        if count == 0 {
            return;
        }
        let next = (self.highlighted as isize + delta).clamp(0, count as isize - 1) as usize;
        self.highlighted = next;
        self.scroll_handle
            .scroll_to_item(next, ScrollStrategy::Center);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(PickerEvent::Dismissed),
            "down" => self.move_highlight(1, cx),
            "up" => self.move_highlight(-1, cx),
            "pagedown" => self.move_highlight(8, cx),
            "pageup" => self.move_highlight(-8, cx),
            "enter" => self.commit_highlighted(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for Picker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let visible = self.visible_rows(cx);
        let total = self.rows.len();
        let shown = visible.len();
        let highlighted = self.highlighted;
        let selected_id = self.selected_id.clone();

        let mut chip_row = div()
            .flex()
            .flex_row()
            // Wrapping is not optional here: a fixed row of chips is exactly
            // what made the old model pills overflow the window.
            .flex_wrap()
            .items_start()
            .gap(px(4.))
            .px(px(10.))
            .py(px(6.));
        for chip in &self.chips {
            let is_active = chip.id == self.active_chip;
            let chip_id = chip.id.clone();
            chip_row = chip_row.child(
                div()
                    .id(ElementId::Name(format!("picker-chip-{}", chip.id).into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(24.))
                    .px(px(10.))
                    .rounded(px(12.))
                    .cursor_pointer()
                    .bg(if is_active {
                        colors.element_selected
                    } else {
                        colors.element_background
                    })
                    .hover(|style| style.bg(colors.ghost_element_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.set_active_chip(chip_id.clone(), cx);
                        cx.emit(PickerEvent::ChipChanged(chip_id.clone()));
                    }))
                    .child(
                        Label::new(chip.label.clone())
                            .size(LabelSize::XSmall)
                            .color(if is_active {
                                Color::Default
                            } else {
                                Color::Muted
                            })
                            .weight(if is_active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            }),
                    ),
            );
        }

        let rows_for_list = visible.clone();
        let list = uniform_list(
            "picker-rows",
            shown,
            cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
                let colors = cx.colors().clone();
                range
                    .map(|index| {
                        let Some(row) = rows_for_list.get(index) else {
                            return div().h(px(ROW_HEIGHT)).into_any_element();
                        };
                        let is_highlighted = index == highlighted;
                        let is_selected = selected_id.as_ref() == Some(&row.id);
                        let row_id = row.id.clone();

                        let mut badges = div().flex().flex_row().gap(px(4.));
                        for badge in &row.badges {
                            badges = badges.child(
                                div()
                                    .px(px(5.))
                                    .rounded(px(3.))
                                    .bg(colors.element_background)
                                    .child(
                                        Label::new(badge.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            );
                        }

                        div()
                            .id(ElementId::NamedInteger("picker-row".into(), index as u64))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .h(px(ROW_HEIGHT))
                            .px(px(10.))
                            .cursor_pointer()
                            .when(is_highlighted, |el| el.bg(colors.element_selected))
                            .hover(|style| style.bg(colors.ghost_element_hover))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.selected_id = Some(row_id.clone());
                                cx.emit(PickerEvent::Selected(row_id.clone()));
                                cx.notify();
                            }))
                            .child(div().w(px(14.)).flex_shrink_0().when(is_selected, |el| {
                                el.child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::XSmall)
                                        .color(Color::Accent),
                                )
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        Label::new(row.primary.clone())
                                            .size(LabelSize::Small)
                                            .color(Color::Default),
                                    )
                                    .when_some(row.secondary.clone(), |el, secondary| {
                                        el.child(
                                            Label::new(secondary)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                    }),
                            )
                            .child(badges)
                            .when_some(row.trailing.clone(), |el, trailing| {
                                el.child(
                                    div().flex_shrink_0().child(
                                        Label::new(trailing)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                                )
                            })
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll_handle)
        .flex_1()
        .min_h(px(0.));

        div()
            .id("model-picker")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w_full()
            .max_h(px(420.))
            .rounded(px(8.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .elevation_2(cx)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1().min_w_0().child(self.query_editor.clone()))
                    .child(
                        crate::IconButton::new("picker-refresh", IconName::Refresh)
                            .size(crate::ButtonSize::Compact)
                            .color(Color::Muted)
                            .tooltip("Refresh the model list")
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                cx.emit(PickerEvent::RefreshRequested);
                            })),
                    ),
            )
            .when(!self.chips.is_empty(), |el| el.child(chip_row))
            // A failed refresh shows a note; it never blanks the list, because
            // a stale list is far more use than an empty one.
            .when_some(self.status_note.clone(), |el, note| {
                el.child(
                    div().px(px(10.)).pb(px(4.)).child(
                        Label::new(note)
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    ),
                )
            })
            .child(if shown == 0 {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .h(px(80.))
                    .child(
                        Label::new("No models match this search")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .into_any_element()
            } else {
                list.into_any_element()
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new("↑↓ navigate   ⏎ select   esc cancel")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new(match &self.footer_note {
                            Some(note) => format!("{shown} of {total} · {note}"),
                            None => format!("{shown} of {total}"),
                        })
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<PickerRow> {
        vec![
            PickerRow::new("google/gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite")
                .facet("cheap")
                .facet("tools"),
            PickerRow::new("openai/gpt-5.6-luna", "GPT-5.6 Luna")
                .facet("cheap")
                .facet("tools"),
            PickerRow::new("anthropic/claude-opus-4-6", "Claude Opus 4.6").facet("tools"),
        ]
    }

    #[test]
    fn an_empty_query_preserves_the_supplied_order() {
        let rows = rows();
        let ranked = rank_rows(&rows, "");
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].id, rows[0].id);
        assert_eq!(ranked[2].id, rows[2].id);
    }

    #[test]
    fn a_query_ranks_matches_and_drops_non_matches() {
        let rows = rows();
        let ranked = rank_rows(&rows, "flash");
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].id.contains("flash"));
    }

    #[test]
    fn a_query_matches_the_display_name_as_well_as_the_id() {
        let rows = rows();
        let ranked = rank_rows(&rows, "Opus");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "anthropic/claude-opus-4-6");
    }

    #[test]
    fn a_query_that_matches_nothing_yields_an_empty_list_not_everything() {
        assert!(rank_rows(&rows(), "zzzzzz").is_empty());
    }

    #[test]
    fn ranking_is_case_insensitive() {
        assert_eq!(rank_rows(&rows(), "FLASH").len(), 1);
    }

    #[test]
    fn an_empty_chip_means_all_rows() {
        let rows = rows();
        let all: Vec<&PickerRow> = rows.iter().collect();
        assert_eq!(rows_in_facet(&all, "").len(), 3);
    }

    #[test]
    fn a_chip_keeps_only_rows_carrying_that_facet() {
        let rows = rows();
        let all: Vec<&PickerRow> = rows.iter().collect();
        assert_eq!(rows_in_facet(&all, "cheap").len(), 2);
        assert_eq!(rows_in_facet(&all, "tools").len(), 3);
        assert_eq!(rows_in_facet(&all, "free").len(), 0);
    }

    #[test]
    fn chip_and_query_filtering_compose() {
        let rows = rows();
        let ranked = rank_rows(&rows, "gpt");
        let filtered = rows_in_facet(&ranked, "cheap");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "openai/gpt-5.6-luna");
    }

    #[test]
    fn row_builders_compose() {
        let row = PickerRow::new("id", "Primary")
            .secondary("Google · fast")
            .trailing("1M   $0.25/$1.50")
            .badge("tools")
            .facet("cheap")
            .pinned_note("no longer offered");
        assert_eq!(row.secondary.as_deref(), Some("Google · fast"));
        assert_eq!(row.badges, vec![SharedString::from("tools")]);
        assert_eq!(row.facets, vec![SharedString::from("cheap")]);
        assert_eq!(row.pinned_note.as_deref(), Some("no longer offered"));
    }
}
