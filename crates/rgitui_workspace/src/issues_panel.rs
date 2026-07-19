use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    KeyDownEvent, Render, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
};

use crate::github_api::GithubCollectionError;
use crate::github_data_service::{
    GithubCollectionPayload, GithubDataKey, GithubDataService, GithubFetchPolicy,
};
use crate::markdown_view::render_markdown;
use gpui::Entity;
use rgitui_theme::{hex_to_hsla_strict, ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize,
    TextInput, TextInputEvent,
};

#[derive(Clone, Debug)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: IssueState,
    pub author: String,
    pub created_at: String,
    pub labels: Vec<IssueLabel>,
    pub comments_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Clone, Debug)]
pub struct IssueLabel {
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug)]
pub struct IssueComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub enum IssuesPanelEvent {
    IssueSelected(Issue, Vec<IssueComment>),
}

impl EventEmitter<IssuesPanelEvent> for IssuesPanel {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IssueFilter {
    Open,
    Closed,
    All,
}

impl IssueFilter {
    fn api_value(&self) -> &str {
        match self {
            IssueFilter::Open => "open",
            IssueFilter::Closed => "closed",
            IssueFilter::All => "all",
        }
    }

    fn label(&self) -> &str {
        match self {
            IssueFilter::Open => "Open",
            IssueFilter::Closed => "Closed",
            IssueFilter::All => "All",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum IssuesPanelView {
    List,
    Detail,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GithubRequestIdentity {
    owner: String,
    repo: String,
    auth_fingerprint: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum IssueListMode {
    Filter(IssueFilter),
    Search(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IssueListRequestKey {
    identity: GithubRequestIdentity,
    mode: IssueListMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IssueCommentsRequestKey {
    identity: GithubRequestIdentity,
    issue_number: u64,
}

fn issue_list_key_is_loaded(
    loaded: Option<&IssueListRequestKey>,
    current: Option<&IssueListRequestKey>,
) -> bool {
    loaded.is_some() && loaded == current
}

fn request_is_current<K: PartialEq>(active: Option<&(u64, K)>, generation: u64, key: &K) -> bool {
    active.is_some_and(|(active_generation, active_key)| {
        *active_generation == generation && active_key == key
    })
}

fn filter_change_requires_fetch(
    current_filter: &IssueFilter,
    next_filter: &IssueFilter,
    had_search: bool,
) -> bool {
    had_search || current_filter != next_filter
}

pub struct IssuesPanel {
    github_data: Entity<GithubDataService>,
    issues: Arc<[Issue]>,
    selected_index: Option<usize>,
    is_loading: bool,
    error_message: Option<String>,
    /// Set when a fetch failed with 401/403/404 and no token is configured —
    /// drives a "sign in" empty state instead of a raw error.
    auth_required: bool,
    filter: IssueFilter,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    github_token: Option<String>,
    github_owner: String,
    github_repo: String,
    view_mode: IssuesPanelView,
    selected_issue: Option<Issue>,
    selected_comments: Vec<IssueComment>,
    comments_loading: bool,
    /// Detail-scoped error surfaced when an issue's comments fail to load. Kept
    /// separate from `error_message` so a transient comment-fetch failure
    /// renders inside the detail view instead of leaking into the list view.
    comments_error: Option<String>,
    last_fetched: Option<std::time::Instant>,
    loaded_list_key: Option<IssueListRequestKey>,
    active_list_request: Option<(u64, IssueListRequestKey)>,
    list_request_generation: u64,
    active_comments_request: Option<(u64, IssueCommentsRequestKey)>,
    comments_request_generation: u64,
    search_query: Option<String>,
    is_searching: bool,
    query_input: Entity<TextInput>,
}

impl IssuesPanel {
    pub(crate) fn new(cx: &mut Context<Self>, github_data: Entity<GithubDataService>) -> Self {
        Self {
            github_data,
            issues: Vec::new().into(),
            selected_index: None,
            is_loading: false,
            error_message: None,
            auth_required: false,
            filter: IssueFilter::Open,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            github_token: None,
            github_owner: String::new(),
            github_repo: String::new(),
            view_mode: IssuesPanelView::List,
            selected_issue: None,
            selected_comments: Vec::new(),
            comments_loading: false,
            comments_error: None,
            last_fetched: None,
            loaded_list_key: None,
            active_list_request: None,
            list_request_generation: 0,
            active_comments_request: None,
            comments_request_generation: 0,
            search_query: None,
            is_searching: false,
            query_input: {
                let query_input = cx.new(|cx| {
                    let mut ti = TextInput::new(cx);
                    ti.set_placeholder("Search issues...");
                    ti
                });
                let input = query_input.clone();
                cx.subscribe(
                    &query_input,
                    move |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                        TextInputEvent::Submit => {
                            let query = input.read(cx).text().to_string();
                            this.submit_search(query, cx);
                        }
                        TextInputEvent::Changed(_) => {}
                    },
                )
                .detach();
                query_input
            },
        }
    }

    pub fn has_issues_loaded(&self) -> bool {
        let current = self.current_display_key();
        issue_list_key_is_loaded(self.loaded_list_key.as_ref(), current.as_ref())
    }

    pub fn is_loading(&self) -> bool {
        self.active_list_request.is_some()
    }

    fn request_identity(&self) -> Option<GithubRequestIdentity> {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            return None;
        }
        Some(GithubRequestIdentity {
            owner: self.github_owner.clone(),
            repo: self.github_repo.clone(),
            auth_fingerprint: token_fingerprint(self.github_token.as_deref()),
        })
    }

    fn current_display_key(&self) -> Option<IssueListRequestKey> {
        let identity = self.request_identity()?;
        let mode = self
            .search_query
            .as_ref()
            .map(|query| IssueListMode::Search(query.clone()))
            .unwrap_or_else(|| IssueListMode::Filter(self.filter.clone()));
        Some(IssueListRequestKey { identity, mode })
    }

    fn invalidate_requests_and_data(&mut self, cx: &mut Context<Self>) {
        self.list_request_generation = self.list_request_generation.wrapping_add(1);
        self.comments_request_generation = self.comments_request_generation.wrapping_add(1);
        self.active_list_request = None;
        self.active_comments_request = None;
        self.is_loading = false;
        self.comments_loading = false;
        self.issues = Vec::new().into();
        self.loaded_list_key = None;
        self.last_fetched = None;
        self.selected_index = None;
        self.selected_issue = None;
        self.selected_comments.clear();
        self.comments_error = None;
        self.view_mode = IssuesPanelView::List;
        self.search_query = None;
        self.is_searching = false;
        self.query_input.update(cx, |input, cx| input.clear(cx));
    }

    pub fn clear_configuration(&mut self, cx: &mut Context<Self>) {
        if self.github_token.is_none()
            && self.github_owner.is_empty()
            && self.github_repo.is_empty()
        {
            return;
        }
        self.invalidate_requests_and_data(cx);
        self.github_token = None;
        self.github_owner.clear();
        self.github_repo.clear();
        self.error_message = None;
        self.auth_required = false;
        cx.notify();
    }

    pub fn configure(
        &mut self,
        token: Option<String>,
        owner: String,
        repo: String,
        cx: &mut Context<Self>,
    ) {
        // Idempotent: a repository refresh fires on every working-tree change,
        // so skip the notify churn when nothing actually changed.
        if self.github_token == token && self.github_owner == owner && self.github_repo == repo {
            return;
        }
        let now_configured = !owner.is_empty() && !repo.is_empty();
        self.invalidate_requests_and_data(cx);
        self.github_token = token;
        self.github_owner = owner;
        self.github_repo = repo;
        // Clear the stale "No GitHub remote configured" error once a valid
        // remote is known (it loads asynchronously after the tab opens). The
        // panel fetches lazily when next viewed.
        if now_configured {
            self.error_message = None;
            self.auth_required = false;
        }
        cx.notify();
    }

    pub fn fetch_issues(&mut self, cx: &mut Context<Self>) {
        self.fetch_issues_with_policy(false, cx);
    }

    fn force_fetch_issues(&mut self, cx: &mut Context<Self>) {
        self.fetch_issues_with_policy(true, cx);
    }

    fn retry_current_request(&mut self, cx: &mut Context<Self>) {
        if let Some(query) = self.search_query.clone() {
            self.do_search(query, cx);
        } else {
            self.force_fetch_issues(cx);
        }
    }

    fn fetch_issues_with_policy(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message = Some("No GitHub remote configured".into());
            cx.notify();
            return;
        }

        // Public repositories are readable without a token, so attempt the
        // fetch unauthenticated when none is configured; auth failures are
        // reported through `auth_required` below.
        let token = self.github_token.clone().filter(|t| !t.is_empty());
        let key = IssueListRequestKey {
            identity: self.request_identity().expect("configured GitHub identity"),
            mode: IssueListMode::Filter(self.filter.clone()),
        };

        if self
            .active_list_request
            .as_ref()
            .is_some_and(|(_, active_key)| active_key == &key)
        {
            return;
        }

        // Skip refetch if data was loaded recently (within 60 seconds).
        if !force && self.loaded_list_key.as_ref() == Some(&key) {
            if let Some(last) = self.last_fetched {
                if last.elapsed() < std::time::Duration::from_secs(60) {
                    log::debug!(
                        "fetch_issues: cached, skipping (loaded {:.0}s ago)",
                        last.elapsed().as_secs_f32()
                    );
                    return;
                }
            }
        }

        self.is_loading = true;
        self.error_message = None;
        self.auth_required = false;
        cx.notify();

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let state = self.filter.api_value().to_string();
        log::info!("fetch_issues: {}/{} filter={}", owner, repo, state);
        let http = cx.http_client();
        let url = github_issues_url(&owner, &repo, &state);
        let data_key = GithubDataKey::new(&owner, &repo, &url, token.as_deref());
        let fetch_task = self.github_data.update(cx, |service, cx| {
            service.fetch_collection(
                data_key,
                url,
                token,
                http,
                if force {
                    GithubFetchPolicy::Revalidate
                } else {
                    GithubFetchPolicy::UseFresh
                },
                cx,
            )
        });
        self.list_request_generation = self.list_request_generation.wrapping_add(1);
        let request_generation = self.list_request_generation;
        self.active_list_request = Some((request_generation, key.clone()));

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match fetch_task.await {
                Ok(payload) => {
                    log_payload_metadata("issues", &payload);
                    let bodies = payload.bodies;
                    cx.background_executor()
                        .spawn(async move { parse_github_issues(&bodies) })
                        .await
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    if !request_is_current(
                        panel.active_list_request.as_ref(),
                        request_generation,
                        &key,
                    ) {
                        return;
                    }
                    panel.is_loading = false;
                    panel.active_list_request = None;
                    match result {
                        Ok(issues) => {
                            panel.issues = issues.into();
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            panel.loaded_list_key = Some(key);
                            log::info!("fetch_issues complete: {} issues", panel.issues.len());
                        }
                        Err(e) => {
                            let no_token = panel.github_token.as_deref().unwrap_or("").is_empty();
                            panel.auth_required = e.auth_required && no_token;
                            panel.error_message = Some(e.message);
                        }
                    }
                    cx.notify();
                })
                .ok();
            });
        })
        .detach();
    }

    fn select_issue(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(issue) = self.issues.get(index).cloned() else {
            return;
        };
        self.selected_index = Some(index);
        self.selected_issue = Some(issue.clone());
        self.selected_comments = Vec::new();
        self.comments_error = None;
        self.comments_loading = true;
        self.view_mode = IssuesPanelView::Detail;
        cx.notify();

        let token = self.github_token.clone().filter(|t| !t.is_empty());

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let issue_number = issue.number;
        let Some(identity) = self.request_identity() else {
            return;
        };
        let request_key = IssueCommentsRequestKey {
            identity,
            issue_number,
        };
        if self
            .active_comments_request
            .as_ref()
            .is_some_and(|(_, active_key)| active_key == &request_key)
        {
            return;
        }
        self.comments_request_generation = self.comments_request_generation.wrapping_add(1);
        let request_generation = self.comments_request_generation;
        self.active_comments_request = Some((request_generation, request_key.clone()));
        let http = cx.http_client();
        let url = github_comments_url(&owner, &repo, issue_number);
        let data_key = GithubDataKey::new(&owner, &repo, &url, token.as_deref());
        let fetch_task = self.github_data.update(cx, |service, cx| {
            service.fetch_collection(data_key, url, token, http, GithubFetchPolicy::UseFresh, cx)
        });
        let issue_for_event = issue;

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match fetch_task.await {
                Ok(payload) => {
                    log_payload_metadata("issue comments", &payload);
                    let bodies = payload.bodies;
                    cx.background_executor()
                        .spawn(async move { parse_github_comments(&bodies) })
                        .await
                }
                Err(error) => Err(error.message),
            };
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    let is_current = request_is_current(
                        panel.active_comments_request.as_ref(),
                        request_generation,
                        &request_key,
                    ) && panel.selected_issue.as_ref().map(|issue| issue.number)
                        == Some(issue_number);
                    if !is_current {
                        return;
                    }
                    panel.active_comments_request = None;
                    panel.comments_loading = false;
                    match result {
                        Ok(comments) => {
                            panel.selected_comments = comments.clone();
                            cx.emit(IssuesPanelEvent::IssueSelected(issue_for_event, comments));
                        }
                        Err(e) => {
                            panel.comments_error = Some(e);
                        }
                    }
                    cx.notify();
                })
                .ok();
            });
        })
        .detach();
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        self.comments_request_generation = self.comments_request_generation.wrapping_add(1);
        self.active_comments_request = None;
        self.comments_loading = false;
        self.view_mode = IssuesPanelView::List;
        self.selected_issue = None;
        self.selected_comments = Vec::new();
        self.comments_error = None;
        cx.notify();
    }

    fn set_filter(&mut self, filter: IssueFilter, cx: &mut Context<Self>) {
        self.is_searching = false;
        let had_search = self.search_query.is_some();
        self.search_query = None;
        if had_search {
            self.query_input.update(cx, |input, cx| input.clear(cx));
        }
        if filter_change_requires_fetch(&self.filter, &filter, had_search) {
            self.filter = filter;
            self.force_fetch_issues(cx);
        }
    }

    fn toggle_search(&mut self, cx: &mut Context<Self>) {
        let had_query = self.search_query.is_some();
        self.is_searching = !self.is_searching;
        if !self.is_searching {
            self.search_query = None;
            // Exiting search must restore the normal filtered list. Search
            // results may have replaced `issues`, so force a refetch by
            // clearing the cache timestamp rather than relying on the list
            // being empty.
            if had_query || self.issues.is_empty() {
                self.last_fetched = None;
                self.fetch_issues(cx);
            }
        }
        cx.notify();
    }

    fn submit_search(&mut self, query: String, cx: &mut Context<Self>) {
        let query = query.trim();
        if query.is_empty() {
            self.is_searching = false;
            self.search_query = None;
            self.force_fetch_issues(cx);
            return;
        }
        self.search_query = Some(query.to_string());
        self.is_searching = false;
        self.do_search(query.to_string(), cx);
    }

    fn do_search(&mut self, query: String, cx: &mut Context<Self>) {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message = Some("No GitHub remote configured".into());
            cx.notify();
            return;
        }
        let token = self.github_token.clone().filter(|t| !t.is_empty());
        let request_key = IssueListRequestKey {
            identity: self.request_identity().expect("configured GitHub identity"),
            mode: IssueListMode::Search(query.clone()),
        };
        if self
            .active_list_request
            .as_ref()
            .is_some_and(|(_, active_key)| active_key == &request_key)
        {
            return;
        }
        self.is_loading = true;
        self.error_message = None;
        self.auth_required = false;
        cx.notify();
        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let http = cx.http_client();
        let url = github_issue_search_url(&owner, &repo, &query);
        let data_key = GithubDataKey::new(&owner, &repo, &url, token.as_deref());
        let fetch_task = self.github_data.update(cx, |service, cx| {
            service.fetch_collection(
                data_key,
                url,
                token,
                http,
                GithubFetchPolicy::Revalidate,
                cx,
            )
        });
        self.list_request_generation = self.list_request_generation.wrapping_add(1);
        let request_generation = self.list_request_generation;
        self.active_list_request = Some((request_generation, request_key.clone()));
        log::info!("search_issues: {}/{} query='{}'", owner, repo, query);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match fetch_task.await {
                Ok(payload) => {
                    log_payload_metadata("issue search", &payload);
                    let bodies = payload.bodies;
                    cx.background_executor()
                        .spawn(async move { parse_github_issue_search(&bodies) })
                        .await
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                let _ = this.update(cx, |panel, cx| {
                    let is_current = request_is_current(
                        panel.active_list_request.as_ref(),
                        request_generation,
                        &request_key,
                    );
                    if !is_current {
                        return;
                    }
                    panel.active_list_request = None;
                    panel.is_loading = false;
                    match result {
                        Ok(issues) => {
                            panel.issues = issues.into();
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            panel.loaded_list_key = Some(request_key);
                            log::info!("search_issues complete: {} results", panel.issues.len());
                        }
                        Err(e) => {
                            let no_token = panel.github_token.as_deref().unwrap_or("").is_empty();
                            panel.auth_required = e.auth_required && no_token;
                            panel.error_message = Some(e.message);
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        if self.view_mode == IssuesPanelView::Detail {
            if key == "escape" {
                self.go_back(cx);
                cx.stop_propagation();
            }
            return;
        }

        // While searching the text input owns the keyboard; list navigation
        // must not steal j/k or hijack Enter from the search submission.
        if self.is_searching {
            return;
        }

        let count = self.issues.len();
        if count == 0 {
            return;
        }

        match key {
            "down" | "j" => {
                let next = self
                    .selected_index
                    .map(|i| (i + 1).min(count - 1))
                    .unwrap_or(0);
                self.selected_index = Some(next);
                self.scroll_handle
                    .scroll_to_item(next, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "up" | "k" => {
                let prev = self
                    .selected_index
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.selected_index = Some(prev);
                self.scroll_handle
                    .scroll_to_item(prev, ScrollStrategy::Nearest);
                cx.notify();
                cx.stop_propagation();
            }
            "enter" => {
                if let Some(index) = self.selected_index {
                    self.select_issue(index, cx);
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.view_mode == IssuesPanelView::Detail {
            return div()
                .h_flex()
                .w_full()
                .h(px(32.))
                .px(px(12.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    IconButton::new("issues-back", IconName::ChevronLeft)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go_back(cx);
                        })),
                )
                .child(
                    Label::new("Issue Detail")
                        .size(LabelSize::Small)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Default),
                )
                .into_any_element();
        }

        let is_open = self.filter == IssueFilter::Open;
        let is_closed = self.filter == IssueFilter::Closed;
        let is_all = self.filter == IssueFilter::All;

        let issue_count: SharedString = format!("{}", self.issues.len()).into();
        let search_query = self.search_query.clone();

        div()
            .h_flex()
            .w_full()
            .h(px(32.))
            .px(px(12.))
            .gap(px(6.))
            .items_center()
            .bg(colors.surface_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                Icon::new(IconName::DotOutline)
                    .size(IconSize::XSmall)
                    .color(Color::Accent),
            )
            .child(
                Label::new("Issues")
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Default),
            )
            .when(!self.issues.is_empty() && search_query.is_none(), |el| {
                el.child(Badge::new(issue_count).color(Color::Muted))
            })
            .when(search_query.is_some(), |el| {
                el.child(
                    Label::new(format!("Search: '{}'", search_query.as_ref().unwrap()))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            })
            .child(div().flex_1())
            .when(!self.is_searching, |el| {
                el.child(
                    div()
                        .h_flex()
                        .h(px(22.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(colors.border_variant)
                        .overflow_hidden()
                        .child(
                            Button::new("filter-open", "Open")
                                .size(ButtonSize::Compact)
                                .style(if is_open {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Transparent
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.set_filter(IssueFilter::Open, cx);
                                })),
                        )
                        .child(
                            Button::new("filter-closed", "Closed")
                                .size(ButtonSize::Compact)
                                .style(if is_closed {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Transparent
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.set_filter(IssueFilter::Closed, cx);
                                })),
                        )
                        .child(
                            Button::new("filter-all", "All")
                                .size(ButtonSize::Compact)
                                .style(if is_all {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Transparent
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.set_filter(IssueFilter::All, cx);
                                })),
                        ),
                )
            })
            .when(self.is_searching, |el| {
                el.child(
                    div()
                        .id("issues-search-input-container")
                        .h(px(24.))
                        .flex_1()
                        .max_w(px(200.))
                        .child(self.query_input.clone()),
                )
            })
            .child(
                IconButton::new("issues-search", IconName::Search)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .tooltip("Search issues")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_search(cx);
                    })),
            )
            .child(
                IconButton::new("issues-refresh", IconName::Refresh)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        // Force a fresh fetch even within the cache window, and
                        // discard any active search so Refresh always restores
                        // the normal filtered list.
                        this.is_searching = false;
                        this.search_query = None;
                        this.force_fetch_issues(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let Some(issue) = &self.selected_issue else {
            return div().into_any_element();
        };

        let title: SharedString = issue.title.clone().into();
        let number_text: SharedString = format!("#{}", issue.number).into();
        let author: SharedString = issue.author.clone().into();
        let created: SharedString = issue.created_at.clone().into();

        let (state_label, state_icon, state_color) = match issue.state {
            IssueState::Open => ("Open", IconName::DotOutline, Color::Success),
            IssueState::Closed => ("Closed", IconName::CheckCircle, Color::Accent),
        };

        let mut content = div()
            .id("issue-detail-scroll")
            .v_flex()
            .flex_1()
            .overflow_y_scroll()
            .p_3()
            .gap_3();

        let mut card = div()
            .v_flex()
            .w_full()
            .px(px(16.))
            .py(px(14.))
            .gap(px(10.))
            .bg(colors.elevated_surface_background)
            .rounded(px(8.))
            .border_1()
            .border_color(colors.border_variant);

        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .items_start()
                .child(
                    div().flex_1().min_w_0().child(
                        Label::new(title)
                            .size(LabelSize::Large)
                            .weight(gpui::FontWeight::BOLD),
                    ),
                )
                .child(
                    Label::new(number_text)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
        );

        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            Icon::new(state_icon)
                                .size(IconSize::XSmall)
                                .color(state_color),
                        )
                        .child(Badge::new(SharedString::from(state_label)).color(state_color)),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            Icon::new(IconName::User)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(author)
                                .size(LabelSize::XSmall)
                                .weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            Icon::new(IconName::Clock)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(created)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if !issue.labels.is_empty() {
            let mut labels_row = div().h_flex().gap(px(4.)).flex_wrap();
            for label in &issue.labels {
                let label_name: SharedString = label.name.clone().into();
                labels_row = labels_row.child(Badge::new(label_name).color(label_color(label)));
            }
            card = card.child(labels_row);
        }

        if let Some(body) = &issue.body {
            if !body.is_empty() {
                card = card.child(
                    div()
                        .w_full()
                        .pt_2()
                        .mt_1()
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(render_markdown(body, window, cx)),
                );
            }
        }

        content = content.child(card);

        if self.comments_loading {
            content = content.child(
                div()
                    .h_flex()
                    .w_full()
                    .py(px(16.))
                    .gap(px(8.))
                    .justify_center()
                    .items_center()
                    .child(
                        Icon::new(IconName::Refresh)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Loading comments...")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        } else if let Some(err) = &self.comments_error {
            let err_text: SharedString = format!("Failed to load comments: {err}").into();
            content = content.child(
                div()
                    .h_flex()
                    .w_full()
                    .py(px(16.))
                    .gap(px(8.))
                    .justify_center()
                    .items_center()
                    .child(
                        Icon::new(IconName::AlertTriangle)
                            .size(IconSize::Small)
                            .color(Color::Error),
                    )
                    .child(
                        Label::new(err_text)
                            .size(LabelSize::Small)
                            .color(Color::Error),
                    ),
            );
        } else if !self.selected_comments.is_empty() {
            let comment_count = self.selected_comments.len();
            let comments_header: SharedString = format!(
                "{} comment{}",
                comment_count,
                if comment_count == 1 { "" } else { "s" }
            )
            .into();
            content = content.child(
                div()
                    .h_flex()
                    .w_full()
                    .py(px(6.))
                    .gap(px(6.))
                    .items_center()
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(comments_header)
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    ),
            );

            for (i, comment) in self.selected_comments.iter().enumerate() {
                let comment_author: SharedString = comment.author.clone().into();
                let comment_date: SharedString = comment.created_at.clone().into();

                content = content.child(
                    div()
                        .id(ElementId::NamedInteger("issue-comment".into(), i as u64))
                        .v_flex()
                        .w_full()
                        .px(px(14.))
                        .py(px(12.))
                        .gap(px(8.))
                        .bg(colors.elevated_surface_background)
                        .rounded(px(8.))
                        .border_1()
                        .border_color(colors.border_variant)
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(20.))
                                        .rounded_full()
                                        .bg(colors.ghost_element_selected)
                                        .child(
                                            Icon::new(IconName::User)
                                                .size(IconSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                )
                                .child(
                                    Label::new(comment_author)
                                        .size(LabelSize::XSmall)
                                        .weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(comment_date)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(render_markdown(&comment.body, window, cx)),
                );
            }
        }

        content.into_any_element()
    }

    fn render_empty_state(
        &self,
        icon: IconName,
        title: &str,
        subtitle: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
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
                        Icon::new(icon)
                            .size(IconSize::Large)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new(SharedString::from(title.to_string()))
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(SharedString::from(subtitle.to_string()))
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            )
            .into_any_element()
    }

    fn render_loading_skeleton(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();
        let skeleton_bg = colors.ghost_element_background;

        div()
            .v_flex()
            .flex_1()
            .w_full()
            .p_2()
            .gap(px(2.))
            .children((0..6).map(move |i| {
                div()
                    .id(ElementId::NamedInteger("skeleton-row".into(), i))
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px_3()
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(6.))
                    .child(div().size(px(8.)).rounded_full().bg(skeleton_bg))
                    .child(div().w(px(40.)).h(px(12.)).rounded(px(4.)).bg(skeleton_bg))
                    .child(div().flex_1().h(px(12.)).rounded(px(4.)).bg(skeleton_bg))
            }))
            .into_any_element()
    }
}

impl Render for IssuesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = cx.colors().panel_background;
        let ghost_selected = cx.colors().ghost_element_selected;
        let text_accent = cx.colors().text_accent;
        let ghost_hover = cx.colors().ghost_element_hover;
        let border_transparent = cx.colors().border_transparent;

        let mut panel = div()
            .id("issues-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(panel_bg);

        panel = panel.child(self.render_toolbar(cx));

        if self.view_mode == IssuesPanelView::Detail {
            panel = panel.child(self.render_detail(window, cx));
            return panel.into_any_element();
        }

        if self.auth_required && self.github_token.as_deref().unwrap_or("").is_empty() {
            return panel
                .child(self.render_empty_state(
                    IconName::Settings,
                    "Sign in to view issues",
                    "This repository is private or rate-limited. Add a GitHub token in Settings — for organization repos you may need a fine-grained token approved by an org owner.",
                    cx,
                ))
                .into_any_element();
        }

        if self.is_loading {
            return panel
                .child(self.render_loading_skeleton(cx))
                .into_any_element();
        }

        if let Some(err) = &self.error_message {
            let err_text: SharedString = err.clone().into();
            return panel
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_4()
                        .child(
                            div()
                                .v_flex()
                                .w_full()
                                .max_w(px(480.))
                                .items_center()
                                .gap(px(12.))
                                .px(px(32.))
                                .py(px(24.))
                                .child(
                                    Icon::new(IconName::AlertTriangle)
                                        .size(IconSize::Large)
                                        .color(Color::Error),
                                )
                                .child(
                                    div().w_full().text_center().child(
                                        Label::new(err_text)
                                            .size(LabelSize::Small)
                                            .color(Color::Error),
                                    ),
                                )
                                .child(
                                    Button::new("retry-issues", "Retry")
                                        .icon(IconName::Refresh)
                                        .size(ButtonSize::Default)
                                        .style(ButtonStyle::Filled)
                                        .color(Color::Accent)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.retry_current_request(cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element();
        }

        if self.issues.is_empty() {
            let empty_msg = format!("No {} issues found", self.filter.label().to_lowercase());
            return panel
                .child(self.render_empty_state(
                    IconName::CheckCircle,
                    &empty_msg,
                    "Try a different filter or check back later",
                    cx,
                ))
                .into_any_element();
        }

        let issues_snapshot: Arc<[Issue]> = self.issues.clone();
        let selected_index = self.selected_index;
        let weak = cx.weak_entity();

        let selected_bg = ghost_selected;
        let selected_border = text_accent;
        let hover_bg = ghost_hover;
        let transparent_border = border_transparent;

        panel = panel.child(
            uniform_list(
                "issues-list",
                issues_snapshot.len(),
                move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                    range
                        .map(|ix| {
                            let issue = &issues_snapshot[ix];
                            let selected = selected_index == Some(ix);
                            let weak = weak.clone();

                            let (state_icon, state_color) = match issue.state {
                                IssueState::Open => (IconName::DotOutline, Color::Success),
                                IssueState::Closed => (IconName::CheckCircle, Color::Accent),
                            };

                            let number_text: SharedString = format!("#{}", issue.number).into();
                            let title: SharedString = issue.title.clone().into();
                            let author: SharedString = issue.author.clone().into();
                            let timestamp: SharedString = issue.created_at.clone().into();
                            let labels = issue.labels.clone();
                            let comments_count = issue.comments_count;

                            let mut row = div()
                                .id(ElementId::NamedInteger("issue-row".into(), ix as u64))
                                .h_flex()
                                .w_full()
                                .h(px(36.))
                                .px_3()
                                .gap(px(8.))
                                .items_center()
                                .flex_shrink_0()
                                .rounded(px(6.))
                                .mx_1()
                                .border_l_2()
                                .when(selected, |el| {
                                    el.bg(selected_bg).border_color(selected_border)
                                })
                                .when(!selected, |el| el.border_color(transparent_border))
                                .hover(move |s| s.bg(hover_bg))
                                .cursor_pointer()
                                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    weak.update(cx, |this, cx| {
                                        this.select_issue(ix, cx);
                                    })
                                    .ok();
                                })
                                .child(
                                    Icon::new(state_icon)
                                        .size(IconSize::XSmall)
                                        .color(state_color),
                                )
                                .child(
                                    Label::new(number_text)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(6.))
                                        .overflow_hidden()
                                        .child(
                                            Label::new(title).size(LabelSize::XSmall).truncate(),
                                        ),
                                );

                            if !labels.is_empty() {
                                let mut labels_row = div().h_flex().gap(px(3.)).flex_shrink_0();
                                for label in labels.iter().take(2) {
                                    let label_name: SharedString = label.name.clone().into();
                                    labels_row = labels_row
                                        .child(Badge::new(label_name).color(label_color(label)));
                                }
                                if labels.len() > 2 {
                                    let more: SharedString =
                                        format!("+{}", labels.len() - 2).into();
                                    labels_row = labels_row.child(
                                        Label::new(more)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    );
                                }
                                row = row.child(labels_row);
                            }

                            row = row.child(
                                div()
                                    .h_flex()
                                    .gap(px(4.))
                                    .flex_shrink_0()
                                    .items_center()
                                    .child(
                                        Label::new(author)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            );

                            row = row.child(
                                Label::new(timestamp)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                            );

                            if comments_count > 0 {
                                let count_text: SharedString = format!("{}", comments_count).into();
                                row = row.child(
                                    div()
                                        .h_flex()
                                        .gap(px(2.))
                                        .flex_shrink_0()
                                        .items_center()
                                        .child(
                                            Icon::new(IconName::GitCommit)
                                                .size(IconSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(count_text)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                );
                            }

                            row.into_any_element()
                        })
                        .collect()
                },
            )
            .flex_1()
            .size_full()
            .with_sizing_behavior(gpui::ListSizingBehavior::Infer)
            .track_scroll(&self.scroll_handle),
        );

        panel.into_any_element()
    }
}

pub(crate) fn github_issues_url(owner: &str, repo: &str, state: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/issues?state={}&per_page=100&sort=updated",
        owner, repo, state
    )
}

fn parse_github_issues(bodies: &[String]) -> Result<Vec<Issue>, GithubCollectionError> {
    let mut issues = Vec::new();
    for body in bodies {
        let json: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| GithubCollectionError::transport(e.to_string()))?;
        let items = json
            .as_array()
            .ok_or_else(|| GithubCollectionError::transport("Expected JSON array".into()))?;

        for item in items {
            if item.get("pull_request").is_some() {
                continue;
            }

            let number = item.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let body_text = item
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let state_str = item.get("state").and_then(|v| v.as_str()).unwrap_or("open");
            let state = if state_str == "closed" {
                IssueState::Closed
            } else {
                IssueState::Open
            };
            let author = item
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let created_at = item
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let comments_count = item.get("comments").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

            let mut labels = Vec::new();
            if let Some(label_array) = item.get("labels").and_then(|v| v.as_array()) {
                for label_item in label_array {
                    let name = label_item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let color = label_item
                        .get("color")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cccccc")
                        .to_string();
                    labels.push(IssueLabel { name, color });
                }
            }

            let created_display = format_github_date(&created_at);

            issues.push(Issue {
                number,
                title,
                body: body_text,
                state,
                author,
                created_at: created_display,
                labels,
                comments_count,
            });
        }
    }

    Ok(issues)
}

fn github_issue_search_url(owner: &str, repo: &str, query: &str) -> String {
    format!(
        "https://api.github.com/search/issues?q={}+repo:{}/{}&per_page=100&sort=updated",
        urlencoding::encode(query),
        owner,
        repo
    )
}

fn parse_github_issue_search(bodies: &[String]) -> Result<Vec<Issue>, GithubCollectionError> {
    let mut issues = Vec::new();
    for body in bodies {
        let json: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| GithubCollectionError::transport(e.to_string()))?;
        let items = json
            .get("items")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                GithubCollectionError::transport("Expected 'items' array in search response".into())
            })?;

        for item in items {
            // Filter out pull requests from issue search results
            if item.get("pull_request").is_some() {
                continue;
            }

            let number = item.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let body_text = item
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let state_str = item.get("state").and_then(|v| v.as_str()).unwrap_or("open");
            let state = if state_str == "closed" {
                IssueState::Closed
            } else {
                IssueState::Open
            };
            let author = item
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let created_at = item
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let comments_count = item.get("comments").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

            let mut labels = Vec::new();
            if let Some(label_array) = item.get("labels").and_then(|v| v.as_array()) {
                for label_item in label_array {
                    let name = label_item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let color = label_item
                        .get("color")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cccccc")
                        .to_string();
                    labels.push(IssueLabel { name, color });
                }
            }

            let created_display = format_github_date(&created_at);

            issues.push(Issue {
                number,
                title,
                body: body_text,
                state,
                author,
                created_at: created_display,
                labels,
                comments_count,
            });
        }
    }

    Ok(issues)
}

pub(crate) fn github_comments_url(owner: &str, repo: &str, issue_number: u64) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments?per_page=100",
        owner, repo, issue_number
    )
}

fn parse_github_comments(bodies: &[String]) -> Result<Vec<IssueComment>, String> {
    let mut comments = Vec::new();
    for body in bodies {
        let json: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let items = json.as_array().ok_or("Expected JSON array")?;

        for item in items {
            let author = item
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let body_text = item
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created_at = item
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let created_display = format_github_date(&created_at);

            comments.push(IssueComment {
                author,
                body: body_text,
                created_at: created_display,
            });
        }
    }

    Ok(comments)
}

fn log_payload_metadata(resource: &str, payload: &GithubCollectionPayload) {
    log::debug!(
        "GitHub {resource} response: cache={}, remaining={:?}, reset={:?}",
        payload.from_cache,
        payload.rate_limit.remaining,
        payload.rate_limit.reset_epoch_seconds
    );
}

/// Resolve a GitHub label's stored hex color into a renderable `Color`.
/// Falls back to `Color::Info` when the hex string is missing or malformed.
fn label_color(label: &IssueLabel) -> Color {
    hex_to_hsla_strict(&label.color)
        .map(Color::Custom)
        .unwrap_or(Color::Info)
}

fn format_github_date(date_str: &str) -> String {
    if date_str.len() >= 10 {
        date_str[..10].to_string()
    } else {
        date_str.to_string()
    }
}

fn token_fingerprint(token: Option<&str>) -> Option<u64> {
    token.filter(|token| !token.is_empty()).map(|token| {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        hasher.finish()
    })
}

/// Parse GitHub owner/repo from a remote URL.
/// Handles:
///   https://github.com/owner/repo.git
///   git@github.com:owner/repo.git
///   ssh://git@github.com/owner/repo.git
pub fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let path = if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        rest
    } else if url.contains("github.com:") {
        url.split("github.com:").nth(1)?
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        rest.split("github.com/").nth(1)?
    } else {
        return None;
    };

    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = path.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner, repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_filter_api_value() {
        assert_eq!(IssueFilter::Open.api_value(), "open");
        assert_eq!(IssueFilter::Closed.api_value(), "closed");
        assert_eq!(IssueFilter::All.api_value(), "all");
    }

    #[test]
    fn test_issue_filter_label() {
        assert_eq!(IssueFilter::Open.label(), "Open");
        assert_eq!(IssueFilter::Closed.label(), "Closed");
        assert_eq!(IssueFilter::All.label(), "All");
    }

    #[test]
    fn request_keys_partition_repo_auth_and_mode() {
        let identity = GithubRequestIdentity {
            owner: "owner".into(),
            repo: "repo".into(),
            auth_fingerprint: token_fingerprint(Some("token-a")),
        };
        let open = IssueListRequestKey {
            identity: identity.clone(),
            mode: IssueListMode::Filter(IssueFilter::Open),
        };
        let closed = IssueListRequestKey {
            identity: identity.clone(),
            mode: IssueListMode::Filter(IssueFilter::Closed),
        };
        let search = IssueListRequestKey {
            identity: identity.clone(),
            mode: IssueListMode::Search("crash".into()),
        };
        let other_auth = IssueListRequestKey {
            identity: GithubRequestIdentity {
                auth_fingerprint: token_fingerprint(Some("token-b")),
                ..identity
            },
            mode: IssueListMode::Filter(IssueFilter::Open),
        };

        assert_ne!(open, closed);
        assert_ne!(open, search);
        assert_ne!(open, other_auth);
    }

    #[test]
    fn token_fingerprint_treats_empty_as_unauthenticated() {
        assert_eq!(token_fingerprint(None), None);
        assert_eq!(token_fingerprint(Some("")), None);
        assert_ne!(
            token_fingerprint(Some("token-a")),
            token_fingerprint(Some("token-b"))
        );
    }

    #[test]
    fn loaded_state_depends_on_request_key_not_row_count() {
        let key = IssueListRequestKey {
            identity: GithubRequestIdentity {
                owner: "owner".into(),
                repo: "empty-repo".into(),
                auth_fingerprint: None,
            },
            mode: IssueListMode::Filter(IssueFilter::Open),
        };

        assert!(issue_list_key_is_loaded(Some(&key), Some(&key)));
        assert!(!issue_list_key_is_loaded(None, Some(&key)));
    }

    #[test]
    fn current_request_requires_matching_generation_and_key() {
        let key = IssueListRequestKey {
            identity: GithubRequestIdentity {
                owner: "owner".into(),
                repo: "repo".into(),
                auth_fingerprint: None,
            },
            mode: IssueListMode::Filter(IssueFilter::Open),
        };
        let other_key = IssueListRequestKey {
            mode: IssueListMode::Filter(IssueFilter::Closed),
            ..key.clone()
        };
        let active = (7, key.clone());

        assert!(request_is_current(Some(&active), 7, &key));
        assert!(!request_is_current(Some(&active), 6, &key));
        assert!(!request_is_current(Some(&active), 7, &other_key));
        assert!(!request_is_current(None, 7, &key));
    }

    #[test]
    fn selecting_active_filter_restores_normal_list_after_search() {
        assert!(filter_change_requires_fetch(
            &IssueFilter::Open,
            &IssueFilter::Open,
            true
        ));
        assert!(!filter_change_requires_fetch(
            &IssueFilter::Open,
            &IssueFilter::Open,
            false
        ));
        assert!(filter_change_requires_fetch(
            &IssueFilter::Open,
            &IssueFilter::Closed,
            false
        ));
    }

    #[test]
    fn test_format_github_date() {
        assert_eq!(format_github_date("2024-01-15T10:30:00Z"), "2024-01-15");
        assert_eq!(format_github_date("2024-03-20"), "2024-03-20");
        assert_eq!(format_github_date(""), "");
        assert_eq!(format_github_date("2024-01"), "2024-01");
        assert_eq!(format_github_date("2024-03-20T14:22:00Z"), "2024-03-20");
        assert_eq!(format_github_date("2024-06-01T08:00:00.000Z"), "2024-06-01");
    }

    #[test]
    fn test_parse_github_owner_repo_https() {
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("http://github.com/owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn test_parse_github_owner_repo_ssh() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("git@github.com:owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn test_parse_github_owner_repo_invalid() {
        assert_eq!(parse_github_owner_repo("https://github.com/owner"), None);
        assert_eq!(parse_github_owner_repo("https://github.com/owner/"), None);
        assert_eq!(parse_github_owner_repo("git@github.com:owner"), None);
        assert_eq!(
            parse_github_owner_repo("https://gitlab.com/owner/repo"),
            None
        );
    }
}
