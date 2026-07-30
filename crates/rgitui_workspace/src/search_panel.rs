//! Global content search panel — Ctrl+Shift+F to open.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Render,
    ScrollHandle, SharedString, Window,
};
use rgitui_git::SearchResult;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent};

use crate::keymap;
use crate::CommandId;

const SEARCH_RESULT_ROW_HEIGHT: f32 = 28.0;
const SEARCH_RESULTS_PAGE_SIZE: usize = 250;

/// Events emitted by the global search view.
#[derive(Debug, Clone)]
pub enum GlobalSearchViewEvent {
    /// User pressed Enter in the search input — contains the query to search.
    SearchSubmit {
        query: String,
        generation: u64,
    },
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
    /// Number of result rows currently materialized in the view. Search can
    /// return many thousands of matches, so expand concrete rows in bounded
    /// pages instead of rebuilding the entire result set every frame.
    visible_result_count: usize,
    /// Whether a search is currently in-flight.
    loading: bool,
    /// Error message if the last search failed.
    error: Option<String>,
    /// Scroll handle for the results list.
    scroll_handle: ScrollHandle,
    /// Focus handle for keyboard navigation.
    focus_handle: FocusHandle,
    /// Text input entity for the query field.
    query_input: Entity<TextInput>,
    /// Whether the panel is visible.
    visible: bool,
    /// Invalidates stale async completions after a new query, clear, or hide.
    search_generation: u64,
}

fn request_is_current(current: u64, completed: u64) -> bool {
    current == completed
}

fn initial_visible_result_count(total: usize) -> usize {
    total.min(SEARCH_RESULTS_PAGE_SIZE)
}

fn expanded_visible_result_count(current: usize, total: usize) -> usize {
    current.saturating_add(SEARCH_RESULTS_PAGE_SIZE).min(total)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchContentState {
    Error,
    Prompt,
    NoResults,
    Results,
}

fn search_content_state(
    query: &str,
    loading: bool,
    has_error: bool,
    result_count: usize,
) -> SearchContentState {
    if has_error {
        SearchContentState::Error
    } else if query.is_empty() && result_count == 0 && !loading {
        SearchContentState::Prompt
    } else if result_count == 0 && !loading {
        SearchContentState::NoResults
    } else {
        SearchContentState::Results
    }
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
                    this.search_generation = this.search_generation.wrapping_add(1);
                    this.results = Arc::new(Vec::new());
                    this.highlighted_row = None;
                    this.visible_result_count = 0;
                    this.loading = false;
                    this.error = None;
                    cx.notify();
                }
                TextInputEvent::Submit => {
                    if !this.query.trim().is_empty() {
                        this.search_generation = this.search_generation.wrapping_add(1);
                        this.loading = true;
                        this.results = Arc::new(Vec::new());
                        this.highlighted_row = None;
                        this.visible_result_count = 0;
                        this.error = None;
                        cx.emit(GlobalSearchViewEvent::SearchSubmit {
                            query: this.query.clone(),
                            generation: this.search_generation,
                        });
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
            visible_result_count: 0,
            loading: false,
            error: None,
            scroll_handle: ScrollHandle::new(),
            focus_handle,
            query_input,
            visible: false,
            search_generation: 0,
        }
    }

    /// Show the search panel and focus the input.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_without_focus(cx);
        self.focus_handle.focus(window, cx);
        self.query_input.update(cx, |ti, cx| ti.focus(window, cx));
        cx.notify();
    }

    /// Show and reset the panel when no `Window` is available (for example,
    /// when invoked through the command palette). The user can then focus the
    /// already-visible input normally.
    pub fn show_without_focus(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.clear(cx);
    }

    /// Hide the search panel.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.search_generation = self.search_generation.wrapping_add(1);
        self.loading = false;
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
    pub fn set_results(
        &mut self,
        generation: u64,
        results: Vec<SearchResult>,
        cx: &mut Context<Self>,
    ) {
        if !request_is_current(self.search_generation, generation) {
            return;
        }
        self.results = Arc::new(results);
        self.visible_result_count = initial_visible_result_count(self.results.len());
        self.loading = false;
        self.error = None;
        self.highlighted_row = None;
        self.scroll_handle.scroll_to_item(0);
        cx.notify();
    }

    /// Mark search as loading.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }

    /// Set an error from a failed search.
    pub fn set_error(&mut self, generation: u64, error: String, cx: &mut Context<Self>) {
        if !request_is_current(self.search_generation, generation) {
            return;
        }
        self.error = Some(error);
        self.loading = false;
        cx.notify();
    }

    /// Clear results and reset state.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.results = Arc::new(Vec::new());
        self.query.clear();
        self.query_input.update(cx, |input, cx| input.clear(cx));
        self.loading = false;
        self.error = None;
        self.highlighted_row = None;
        self.visible_result_count = 0;
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

    /// Runs a keyboard command scoped to the shared `List` group.
    ///
    /// Enter opens the highlighted match; with nothing highlighted it is
    /// propagated so the query field's own submission runs the search.
    fn dispatch_command(&mut self, cmd: CommandId, _window: &mut Window, cx: &mut Context<Self>) {
        let count = self.results.len();

        match cmd {
            CommandId::Cancel => cx.emit(GlobalSearchViewEvent::Dismissed),
            CommandId::Confirm => match self.highlighted_row.and_then(|r| self.results.get(r)) {
                Some(result) => cx.emit(GlobalSearchViewEvent::ResultSelected {
                    path: result.path.to_string_lossy().to_string(),
                    line_number: result.line_number,
                }),
                None => cx.propagate(),
            },
            _ if count == 0 => {}
            CommandId::SelectNext => {
                let next = self
                    .highlighted_row
                    .map_or(0, |row| (row + 1).min(count - 1));
                // Walking past the rendered page is an explicit request for more.
                if next >= self.visible_result_count {
                    self.visible_result_count =
                        expanded_visible_result_count(self.visible_result_count, count);
                }
                self.highlight_row(next, cx);
            }
            CommandId::SelectPrev => {
                let prev = self.highlighted_row.map_or(0, |row| row.saturating_sub(1));
                self.highlight_row(prev, cx);
            }
            CommandId::SelectFirst => self.highlight_row(0, cx),
            CommandId::SelectLast => {
                // Jumping to the final match is an explicit request to
                // materialize the remaining pages.
                self.visible_result_count = count;
                self.highlight_row(count - 1, cx);
            }
            // A command this view does not own falls through to the next handler
            // out, and finally to the focused text field.
            _ => cx.propagate(),
        }
    }

    /// Moves the keyboard highlight and scrolls it into view.
    fn highlight_row(&mut self, row: usize, cx: &mut Context<Self>) {
        self.highlighted_row = Some(row);
        self.scroll_handle.scroll_to_item(row);
        cx.notify();
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
        let content_state = search_content_state(&query, loading, error.is_some(), result_count);

        let root = div()
            .id("global-search-view")
            .v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(colors.editor_background)
            .track_focus(&self.focus_handle)
            .map(|el| {
                keymap::bind_actions(
                    el,
                    "SearchPanel List",
                    &["Menu"],
                    cx,
                    Self::dispatch_command,
                )
            })
            .child(self.render_toolbar(result_count, loading, cx));

        if content_state == SearchContentState::Results {
            root.child(self.render_results(cx)).into_any_element()
        } else {
            root.child(self.render_status_content(&query, error, content_state, cx))
                .into_any_element()
        }
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

    fn render_status_content(
        &self,
        query: &str,
        error: Option<String>,
        content_state: SearchContentState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();

        if content_state == SearchContentState::Error {
            let err = error.expect("error content state requires an error message");
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
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
                )
                .into_any_element();
        }

        if content_state == SearchContentState::Prompt {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
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
                            Label::new("Press Enter to search | j/k to navigate")
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                        ),
                )
                .into_any_element();
        }

        if content_state == SearchContentState::NoResults {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
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
                )
                .into_any_element();
        }

        unreachable!("results are attached directly by GlobalSearchView::render")
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let results = self.results.clone();
        let highlighted_row = self.highlighted_row;
        let text_muted = colors.text_muted;
        let text_accent = colors.text_accent;
        let ghost_element_hover = colors.ghost_element_hover;
        let ghost_element_selected = colors.ghost_element_selected;
        let editor_bg = colors.editor_background;
        let visible_result_count = self.visible_result_count.min(results.len());

        let view = cx.entity().clone();
        let view_for_rows = view.clone();

        let rows = results
            .iter()
            .take(visible_result_count)
            .cloned()
            .enumerate()
            .map(move |(i, result)| {
                let is_selected = highlighted_row == Some(i);
                let path_str: SharedString = result.path.to_string_lossy().into_owned().into();
                let content_str: SharedString = result.content.clone().into();
                let line_str: SharedString = result.line_number.to_string().into();
                let result_for_click = result.clone();

                let bg = if is_selected {
                    ghost_element_selected
                } else {
                    editor_bg
                };

                div()
                    .id(ElementId::Integer(i as u64))
                    .h_flex()
                    .w_full()
                    .h(px(SEARCH_RESULT_ROW_HEIGHT))
                    .px(px(10.))
                    .py(px(4.))
                    .bg(bg)
                    .hover(|s| s.bg(ghost_element_hover))
                    .cursor_pointer()
                    .on_click({
                        let view = view_for_rows.clone();
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
                            .child(Label::new("|").size(LabelSize::XSmall)),
                        div()
                            .flex_grow()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(text_muted)
                            .child(Label::new(content_str).size(LabelSize::XSmall)),
                    ])
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let has_more = visible_result_count < results.len();
        let remaining = results.len().saturating_sub(visible_result_count);
        let view_for_more = view.clone();

        div()
            .id("global-search-results")
            .v_flex()
            .flex_grow()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .children(rows)
            .when(has_more, |list| {
                list.child(
                    div()
                        .id("global-search-load-more")
                        .h_flex()
                        .w_full()
                        .h(px(SEARCH_RESULT_ROW_HEIGHT))
                        .items_center()
                        .justify_center()
                        .bg(editor_bg)
                        .hover(|style| style.bg(ghost_element_hover))
                        .cursor_pointer()
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view_for_more.update(cx, |this, cx| {
                                this.visible_result_count = expanded_visible_result_count(
                                    this.visible_result_count,
                                    this.results.len(),
                                );
                                cx.notify();
                            });
                        })
                        .child(
                            Label::new(format!("Load more results ({remaining} remaining)"))
                                .size(LabelSize::XSmall)
                                .color(Color::Accent),
                        ),
                )
            })
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

    #[test]
    fn stale_search_generation_is_rejected() {
        assert!(request_is_current(7, 7));
        assert!(!request_is_current(8, 7));
    }

    #[test]
    fn non_empty_results_select_the_list_content_state() {
        assert_eq!(
            search_content_state("rgitui", false, false, 28),
            SearchContentState::Results
        );
    }

    #[test]
    fn loading_without_results_keeps_the_list_content_state() {
        assert_eq!(
            search_content_state("rgitui", true, false, 0),
            SearchContentState::Results
        );
    }

    #[test]
    fn result_materialization_is_bounded_and_expands_by_page() {
        assert_eq!(initial_visible_result_count(0), 0);
        assert_eq!(initial_visible_result_count(42), 42);
        assert_eq!(
            initial_visible_result_count(10_000),
            SEARCH_RESULTS_PAGE_SIZE
        );
        assert_eq!(expanded_visible_result_count(250, 10_000), 500);
        assert_eq!(expanded_visible_result_count(9_900, 10_000), 10_000);
        assert_eq!(
            expanded_visible_result_count(usize::MAX, usize::MAX),
            usize::MAX
        );
    }
}
