//! Global content search panel — Ctrl+Shift+F to open.

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Hsla, KeyDownEvent, Render, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
};
use rgitui_git::SearchResult;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

/// Events emitted by the global search view.
#[derive(Debug, Clone)]
pub enum GlobalSearchViewEvent {
    /// User pressed Enter in the search input — contains the query to search.
    SearchSubmit(String),
    /// User clicked a search result.
    ResultSelected {
        path: String,
        line_number: usize,
    },
    Dismissed,
}

/// Global content search panel.
pub struct GlobalSearchView {
    /// Search query.
    query: String,
    /// All results from the last search.
    results: Arc<Vec<SearchResult>>,
    /// Currently highlighted row index.
    highlighted_row: Option<usize>,
    /// Whether a search is currently in-flight.
    loading: bool,
    /// Error message if the last search failed.
    error: Option<String>,
    /// Scroll handle for the results list.
    scroll_handle: UniformListScrollHandle,
    /// Focus handle for keyboard navigation.
    focus_handle: FocusHandle,
    /// Text input entity for the query field.
    query_input: Entity<TextInput>,
    /// Whether the panel is visible.
    visible: bool,
}

impl EventEmitter<GlobalSearchViewEvent> for GlobalSearchView {}

impl GlobalSearchView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let query_input = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Search across all files... (press Enter)");
            ti
        });

        let focus_handle = cx.focus_handle();

        cx.subscribe(
            &query_input,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    this.query = text.clone();
                    cx.notify();
                }
                TextInputEvent::Submit => {
                    if !this.query.trim().is_empty() {
                        let q = this.query.clone();
                        cx.emit(GlobalSearchViewEvent::SearchSubmit(q));
                    }
                    cx.stop_propagation();
                }
            },
        )
        .detach();

        Self {
            query: String::new(),
            results: Arc::new(Vec::new()),
            highlighted_row: None,
            loading: false,
            error: None,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle,
            query_input,
            visible: false,
        }
    }

    /// Show the search panel and focus the input.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.clear(cx);
        self.focus_handle.focus(window, cx);
        self.query_input.update(cx, |ti, cx| ti.focus(window, cx));
        cx.notify();
    }

    /// Hide the search panel.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.notify();
    }

    /// Whether the search panel is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Toggle the search panel visibility.
    pub fn toggle_visible(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.hide(cx);
        } else {
            self.visible = true;
            self.clear(cx);
            cx.notify();
        }
    }

    /// Focus the search input when the panel opens.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.query_input.update(cx, |ti, cx| ti.focus(window, cx));
        cx.notify();
    }

    /// Set the results from a completed search.
    pub fn set_results(&mut self, results: Vec<SearchResult>, cx: &mut Context<Self>) {
        self.results = Arc::new(results);
        self.loading = false;
        self.error = None;
        self.highlighted_row = None;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Mark search as loading.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }

    /// Set an error from a failed search.
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.loading = false;
        cx.notify();
    }

    /// Clear results and reset state.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.results = Arc::new(Vec::new());
        self.query.clear();
        self.loading = false;
        self.error = None;
        self.highlighted_row = None;
        cx.notify();
    }

    /// Returns true if this view has keyboard focus.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Handle a result row click — select it and emit ResultSelected.
    pub fn select_result(&mut self, result: SearchResult, cx: &mut Context<Self>) {
        if let Some(idx) = self
            .results
            .iter()
            .position(|r| r.line_number == result.line_number && r.path == result.path)
        {
            self.highlighted_row = Some(idx);
            cx.emit(GlobalSearchViewEvent::ResultSelected {
                path: result.path.to_string_lossy().to_string(),
                line_number: result.line_number,
            });
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let count = self.results.len();

        match key {
            "j" | "down" => {
                let next = self
                    .highlighted_row
                    .map(|r| (r + 1).min(count.saturating_sub(1)))
                    .unwrap_or(0);
                if count > 0 {
                    self.highlighted_row = Some(next);
                    self.scroll_handle
                        .scroll_to_item(next, ScrollStrategy::Nearest);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "k" | "up" => {
                let prev = self
                    .highlighted_row
                    .map(|r| r.saturating_sub(1))
                    .unwrap_or(0);
                if count > 0 {
                    self.highlighted_row = Some(prev);
                    self.scroll_handle
                        .scroll_to_item(prev, ScrollStrategy::Nearest);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "enter" => {
                if let Some(row) = self.highlighted_row {
                    if let Some(result) = self.results.get(row) {
                        cx.emit(GlobalSearchViewEvent::ResultSelected {
                            path: result.path.to_string_lossy().to_string(),
                            line_number: result.line_number,
                        });
                    }
                }
                cx.stop_propagation();
            }
            "escape" => {
                cx.emit(GlobalSearchViewEvent::Dismissed);
                cx.stop_propagation();
            }
            "g" => {
                if event.keystroke.modifiers.shift {
                    // Shift+G — jump to last
                    if count > 0 {
                        self.highlighted_row = Some(count - 1);
                        self.scroll_handle
                            .scroll_to_item(count - 1, ScrollStrategy::Bottom);
                        cx.notify();
                    }
                } else if count > 0 {
                    // g — jump to first
                    self.highlighted_row = Some(0);
                    self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for GlobalSearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("global-search-view").into_any_element();
        }

        let colors = cx.colors();
        let query = self.query.clone();
        let loading = self.loading;
        let error = self.error.clone();
        let result_count = self.results.len();

        div()
            .id("global-search-view")
            .v_flex()
            .size_full()
            .bg(colors.editor_background)
            .on_key_down(cx.listener(Self::handle_key_down))
            .child(self.render_toolbar(result_count, loading, cx))
            .child(self.render_content(&query, loading, error, result_count, cx))
            .into_any_element()
    }
}

impl GlobalSearchView {
    fn render_toolbar(
        &self,
        result_count: usize,
        loading: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();

        div()
            .id("global-search-toolbar")
            .h(px(42.))
            .px(px(10.))
            .gap(px(8.))
            .flex()
            .items_center()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .child({
                if loading {
                    Icon::new(IconName::Clock)
                        .size(IconSize::XSmall)
                        .color(Color::Accent)
                } else {
                    Icon::new(IconName::Search)
                        .size(IconSize::XSmall)
                        .color(Color::Muted)
                }
            })
            .child({
                div()
                    .flex_grow()
                    .h_flex()
                    .items_center()
                    .child(self.query_input.clone())
            })
            .child(
                Label::new(format!("{} results", result_count))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }

    fn render_content(
        &self,
        query: &str,
        loading: bool,
        error: Option<String>,
        result_count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();

        if let Some(err) = error {
            return div().flex_1().flex().items_center().justify_center().child(
                div()
                    .v_flex()
                    .gap(px(8.))
                    .items_center()
                    .px(px(24.))
                    .py(px(16.))
                    .rounded(px(8.))
                    .bg(colors.ghost_element_background)
                    .child(
                        Icon::new(IconName::XCircle)
                            .size(IconSize::Large)
                            .color(Color::Error),
                    )
                    .child(Label::new(err).size(LabelSize::Small).color(Color::Error)),
            );
        }

        if query.is_empty() && result_count == 0 && !loading {
            return div().flex_1().flex().items_center().justify_center().child(
                div()
                    .v_flex()
                    .gap(px(8.))
                    .items_center()
                    .px(px(24.))
                    .py(px(16.))
                    .rounded(px(8.))
                    .bg(colors.ghost_element_background)
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new("Type to search across all files")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Press Enter to search · j/k to navigate")
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            );
        }

        if result_count == 0 && !loading {
            return div().flex_1().flex().items_center().justify_center().child(
                div()
                    .v_flex()
                    .gap(px(8.))
                    .items_center()
                    .px(px(24.))
                    .py(px(16.))
                    .rounded(px(8.))
                    .bg(colors.ghost_element_background)
                    .child(
                        Icon::new(IconName::File)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new(format!("No results for \"{}\"", query))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        }

        // Render results list
        let results = self.results.clone();
        let highlighted_row = self.highlighted_row;
        let text_muted = colors.text_muted;
        let text_accent = colors.text_accent;
        let ghost_element_hover = colors.ghost_element_hover;
        let ghost_element_selected = colors.ghost_element_selected;
        let _editor_bg = colors.editor_background;

        let view = cx.entity().clone();

        div().flex_1().overflow_hidden().child(
            uniform_list(
                "global-search-results",
                results.len(),
                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                    range
                        .map(|i| {
                            let result = results[i].clone();
                            let is_selected = highlighted_row == Some(i);
                            let path_str: SharedString =
                                result.path.to_string_lossy().into_owned().into();
                            let content_str: SharedString = result.content.clone().into();
                            let line_str: SharedString = result.line_number.to_string().into();
                            let result_for_click = result.clone();

                            let bg = if is_selected {
                                ghost_element_selected
                            } else {
                                Hsla::default()
                            };

                            div()
                                .id(ElementId::Integer(i as u64))
                                .w_full()
                                .min_h(px(28.))
                                .px(px(10.))
                                .py(px(4.))
                                .bg(bg)
                                .hover(|s| s.bg(ghost_element_hover))
                                .cursor_pointer()
                                .on_click({
                                    let view = view.clone();
                                    let result_for_click = result_for_click.clone();
                                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| {
                                            this.select_result(result_for_click.clone(), cx);
                                        });
                                    }
                                })
                                .children([
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(text_accent)
                                        .child(Label::new(path_str).size(LabelSize::XSmall)),
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(text_muted)
                                        .mx(px(4.))
                                        .child(Label::new(":").size(LabelSize::XSmall)),
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(text_accent)
                                        .child(Label::new(line_str).size(LabelSize::XSmall)),
                                    div()
                                        .flex_shrink_0()
                                        .mx(px(6.))
                                        .text_xs()
                                        .text_color(text_muted)
                                        .child(Label::new("│").size(LabelSize::XSmall)),
                                    div()
                                        .flex_grow()
                                        .overflow_hidden()
                                        .text_xs()
                                        .text_color(text_muted)
                                        .child(Label::new(content_str).size(LabelSize::XSmall)),
                                ])
                        })
                        .collect::<Vec<_>>()
                },
            )
            .size_full()
            .track_scroll(&self.scroll_handle),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn search_result(path: &str, line: usize, content: &str) -> SearchResult {
        SearchResult {
            path: PathBuf::from(path),
            line_number: line,
            content: content.to_string(),
        }
    }

    #[test]
    fn search_result_struct() {
        let r = search_result("src/main.rs", 42, "fn main() {");
        assert_eq!(r.path, PathBuf::from("src/main.rs"));
        assert_eq!(r.line_number, 42);
        assert_eq!(r.content, "fn main() {");
    }

    #[test]
    fn results_clone() {
        let r = search_result("foo.txt", 1, "hello");
        let r2 = r.clone();
        assert_eq!(r.path, r2.path);
        assert_eq!(r.line_number, r2.line_number);
        assert_eq!(r.content, r2.content);
    }
}
