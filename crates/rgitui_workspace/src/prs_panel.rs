use std::hash::{DefaultHasher, Hash, Hasher};
use std::str::FromStr;
use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::prelude::*;
use gpui::{
    div, http_client::AsyncBody, px, uniform_list, App, ClickEvent, Context, ElementId, Entity,
    EventEmitter, FocusHandle, KeyDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, WeakEntity, Window,
};
use http_client::HttpClient;

use crate::github_api::{format_github_detail_error, GithubCollectionError};
use crate::github_data_service::{
    GithubCollectionPayload, GithubDataKey, GithubDataService, GithubFetchPolicy,
};
use crate::markdown_view::render_markdown;
use crate::Workspace;
use rgitui_theme::{hex_to_hsla_strict, ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize,
    TextInput, TintColor,
};

#[derive(Clone, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: PrState,
    pub author: String,
    pub created_at: String,
    pub labels: Vec<PrLabel>,
    pub head_branch: String,
    pub base_branch: String,
    pub draft: bool,
    pub comments_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrState::Open => write!(f, "open"),
            PrState::Closed => write!(f, "closed"),
            PrState::Merged => write!(f, "merged"),
        }
    }
}

impl FromStr for PrState {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(PrState::Open),
            "closed" => Ok(PrState::Closed),
            "merged" => Ok(PrState::Merged),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrLabel {
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug)]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub enum PrsPanelEvent {
    PrSelected(PullRequest, Vec<PrComment>),
    ReviewSubmitted { number: u64, action: String },
}

/// The type of review action to submit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReviewAction {
    Approve,
    RequestChanges,
    Comment,
}

impl FromStr for ReviewAction {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "APPROVE" => Ok(ReviewAction::Approve),
            "REQUEST_CHANGES" => Ok(ReviewAction::RequestChanges),
            "COMMENT" => Ok(ReviewAction::Comment),
            _ => Err(()),
        }
    }
}

impl EventEmitter<PrsPanelEvent> for PrsPanel {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PrFilter {
    Open,
    Closed,
    All,
}

impl PrFilter {
    fn api_value(&self) -> &str {
        match self {
            PrFilter::Open => "open",
            PrFilter::Closed => "closed",
            PrFilter::All => "all",
        }
    }

    fn label(&self) -> &str {
        match self {
            PrFilter::Open => "Open",
            PrFilter::Closed => "Closed",
            PrFilter::All => "All",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PrsPanelView {
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
struct PrListRequestKey {
    identity: GithubRequestIdentity,
    filter: PrFilter,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PrCommentsRequestKey {
    identity: GithubRequestIdentity,
    pr_number: u64,
}

fn pr_list_key_is_loaded(
    loaded: Option<&PrListRequestKey>,
    current: Option<&PrListRequestKey>,
) -> bool {
    loaded.is_some() && loaded == current
}

fn request_is_current<K: PartialEq>(active: Option<&(u64, K)>, generation: u64, key: &K) -> bool {
    active.is_some_and(|(active_generation, active_key)| {
        *active_generation == generation && active_key == key
    })
}

pub struct PrsPanel {
    github_data: Entity<GithubDataService>,
    prs: Arc<[PullRequest]>,
    selected_index: Option<usize>,
    is_loading: bool,
    error_message: Option<String>,
    /// Set when a fetch failed with 401/403/404 and no token is configured —
    /// drives a "sign in" empty state instead of a raw error.
    auth_required: bool,
    filter: PrFilter,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    github_token: Option<String>,
    github_owner: String,
    github_repo: String,
    view_mode: PrsPanelView,
    selected_pr: Option<PullRequest>,
    selected_comments: Vec<PrComment>,
    comments_loading: bool,
    /// Detail-scoped error surfaced when a PR's comments fail to load. Kept
    /// separate from `error_message` so a transient comment-fetch failure
    /// renders inside the detail view instead of leaking into the list view.
    comments_error: Option<String>,
    workspace: WeakEntity<Workspace>,
    last_fetched: Option<std::time::Instant>,
    loaded_list_key: Option<PrListRequestKey>,
    active_list_request: Option<(u64, PrListRequestKey)>,
    list_request_generation: u64,
    active_comments_request: Option<(u64, PrCommentsRequestKey)>,
    comments_request_generation: u64,
    review_submitting: bool,
    review_comment_input: Entity<TextInput>,
    review_result: Option<String>,
}

impl PrsPanel {
    pub(crate) fn new(
        cx: &mut Context<Self>,
        workspace: WeakEntity<Workspace>,
        github_data: Entity<GithubDataService>,
    ) -> Self {
        Self {
            github_data,
            prs: Vec::new().into(),
            selected_index: None,
            is_loading: false,
            error_message: None,
            auth_required: false,
            filter: PrFilter::Open,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            github_token: None,
            github_owner: String::new(),
            github_repo: String::new(),
            view_mode: PrsPanelView::List,
            selected_pr: None,
            selected_comments: Vec::new(),
            comments_loading: false,
            comments_error: None,
            workspace,
            last_fetched: None,
            loaded_list_key: None,
            active_list_request: None,
            list_request_generation: 0,
            active_comments_request: None,
            comments_request_generation: 0,
            review_submitting: false,
            review_comment_input: cx.new(|cx| {
                let mut ti = TextInput::new(cx).multiline();
                ti.set_placeholder("Leave a review comment (optional)");
                ti
            }),
            review_result: None,
        }
    }

    /// Open the GitHub PR creation dialog via the workspace.
    fn open_create_pr_dialog(&self, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.upgrade() else {
            return;
        };
        ws.update(cx, |ws, cx| {
            ws.open_create_pr_dialog(cx);
        });
    }

    pub fn has_prs_loaded(&self) -> bool {
        let current = self.current_list_key();
        pr_list_key_is_loaded(self.loaded_list_key.as_ref(), current.as_ref())
    }

    pub fn is_loading(&self) -> bool {
        self.active_list_request.is_some()
    }

    pub fn github_token(&self) -> Option<&str> {
        self.github_token.as_deref()
    }

    pub fn github_owner(&self) -> &str {
        &self.github_owner
    }

    pub fn github_repo(&self) -> &str {
        &self.github_repo
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

    fn current_list_key(&self) -> Option<PrListRequestKey> {
        Some(PrListRequestKey {
            identity: self.request_identity()?,
            filter: self.filter.clone(),
        })
    }

    fn invalidate_requests_and_data(&mut self, cx: &mut Context<Self>) {
        self.list_request_generation = self.list_request_generation.wrapping_add(1);
        self.comments_request_generation = self.comments_request_generation.wrapping_add(1);
        self.active_list_request = None;
        self.active_comments_request = None;
        self.is_loading = false;
        self.comments_loading = false;
        self.prs = Vec::new().into();
        self.loaded_list_key = None;
        self.last_fetched = None;
        self.selected_index = None;
        self.selected_pr = None;
        self.selected_comments.clear();
        self.comments_error = None;
        self.view_mode = PrsPanelView::List;
        self.review_submitting = false;
        self.review_result = None;
        self.review_comment_input
            .update(cx, |input, cx| input.clear(cx));
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

    /// Fetch the PR list, bypassing the freshness cache. Used after creating a PR
    /// so the new entry shows immediately instead of after the 60s cache expires.
    pub fn force_fetch_prs(&mut self, cx: &mut Context<Self>) {
        self.fetch_prs_with_policy(true, cx);
    }

    pub fn fetch_prs(&mut self, cx: &mut Context<Self>) {
        self.fetch_prs_with_policy(false, cx);
    }

    fn fetch_prs_with_policy(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message = Some("No GitHub remote configured".into());
            cx.notify();
            return;
        }

        // Public repositories are readable without a token, so attempt the
        // fetch unauthenticated when none is configured; auth failures are
        // reported through `auth_required` below.
        let token = self.github_token.clone().filter(|t| !t.is_empty());
        let key = self.current_list_key().expect("configured GitHub identity");
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
                    log::debug!("fetch_prs: cached, skipping");
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
        log::info!("fetch_prs: {}/{} filter={}", owner, repo, state);
        let http = cx.http_client();
        let url = github_prs_url(&owner, &repo, &state);
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
                    log_payload_metadata("pull requests", &payload);
                    let bodies = payload.bodies;
                    cx.background_executor()
                        .spawn(async move { parse_github_prs(&bodies) })
                        .await
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    let is_current = request_is_current(
                        panel.active_list_request.as_ref(),
                        request_generation,
                        &key,
                    );
                    if !is_current {
                        return;
                    }
                    panel.active_list_request = None;
                    panel.is_loading = false;
                    match result {
                        Ok(prs) => {
                            panel.prs = prs.into();
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            panel.loaded_list_key = Some(key);
                            log::info!("fetch_prs complete: {} PRs", panel.prs.len());
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

    fn select_pr(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(pr) = self.prs.get(index).cloned() else {
            return;
        };
        self.selected_index = Some(index);
        self.selected_pr = Some(pr.clone());
        self.selected_comments = Vec::new();
        self.comments_error = None;
        self.comments_loading = true;
        self.view_mode = PrsPanelView::Detail;
        cx.notify();

        let token = self.github_token.clone().filter(|t| !t.is_empty());

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let pr_number = pr.number;
        let Some(identity) = self.request_identity() else {
            return;
        };
        let request_key = PrCommentsRequestKey {
            identity,
            pr_number,
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
        let url = crate::issues_panel::github_comments_url(&owner, &repo, pr_number);
        let data_key = GithubDataKey::new(&owner, &repo, &url, token.as_deref());
        let fetch_task = self.github_data.update(cx, |service, cx| {
            service.fetch_collection(data_key, url, token, http, GithubFetchPolicy::UseFresh, cx)
        });
        let pr_for_event = pr;

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match fetch_task.await {
                Ok(payload) => {
                    log_payload_metadata("pull request comments", &payload);
                    let bodies = payload.bodies;
                    cx.background_executor()
                        .spawn(async move { parse_pr_comments(&bodies) })
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
                    ) && panel.selected_pr.as_ref().map(|pr| pr.number)
                        == Some(pr_number);
                    if !is_current {
                        return;
                    }
                    panel.active_comments_request = None;
                    panel.comments_loading = false;
                    match result {
                        Ok(comments) => {
                            panel.selected_comments = comments.clone();
                            cx.emit(PrsPanelEvent::PrSelected(pr_for_event, comments));
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
        self.view_mode = PrsPanelView::List;
        self.selected_pr = None;
        self.selected_comments = Vec::new();
        self.comments_error = None;
        self.review_result = None;
        self.review_comment_input.update(cx, |ti, cx| ti.clear(cx));
        cx.notify();
    }

    fn set_filter(&mut self, filter: PrFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.last_fetched = None;
            self.fetch_prs(cx);
        }
    }

    fn submit_review(&mut self, action: ReviewAction, cx: &mut Context<Self>) {
        let Some(pr) = self.selected_pr.clone() else {
            return;
        };
        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => return,
        };
        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let pr_number = pr.number;
        let http = cx.http_client();
        let body = self.review_comment_input.read(cx).text().to_string();

        self.review_submitting = true;
        self.review_result = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                submit_pr_review(&http, &token, &owner, &repo, pr_number, action, &body).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.review_submitting = false;
                    match result {
                        Ok(()) => {
                            let action_label = match action {
                                ReviewAction::Approve => "approved",
                                ReviewAction::RequestChanges => "requested changes on",
                                ReviewAction::Comment => "reviewed",
                            };
                            panel.review_result =
                                Some(format!("✓ Review submitted: {action_label} #{}", pr_number));
                            panel.review_comment_input.update(cx, |ti, cx| ti.clear(cx));
                            cx.emit(PrsPanelEvent::ReviewSubmitted {
                                number: pr_number,
                                action: action.to_string(),
                            });
                        }
                        Err(e) => {
                            panel.review_result = Some(format!("✗ {e}"));
                        }
                    }
                    cx.notify();
                })
                .ok();
            });
        })
        .detach();
    }

    fn render_review_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut el = div().v_flex().w_full().gap(px(8.));

        // Review comment input
        let comment_input = self.review_comment_input.clone();
        el = el.child(comment_input);

        // Review action buttons
        let mut buttons = div().h_flex().gap_2();

        let submitting = self.review_submitting;

        buttons = buttons.child(
            Button::new("pr-review-approve", "Approve")
                .style(ButtonStyle::Tinted(TintColor::Success))
                .size(ButtonSize::Compact)
                .disabled(submitting)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.submit_review(ReviewAction::Approve, cx);
                })),
        );

        buttons = buttons.child(
            Button::new("pr-review-request-changes", "Request Changes")
                .style(ButtonStyle::Tinted(TintColor::Error))
                .size(ButtonSize::Compact)
                .disabled(submitting)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.submit_review(ReviewAction::RequestChanges, cx);
                })),
        );

        buttons = buttons.child(
            Button::new("pr-review-comment", "Comment")
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .disabled(submitting)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.submit_review(ReviewAction::Comment, cx);
                })),
        );

        if submitting {
            buttons = buttons.child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Refresh)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Submitting...")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        el = el.child(div().v_flex().w_full().gap_2().child(buttons));

        // Result message
        if let Some(msg) = &self.review_result {
            let is_success = msg.starts_with('✓');
            el =
                el.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(if is_success {
                            cx.status().success_background
                        } else {
                            cx.status().error_background
                        })
                        .child(Label::new(msg.clone()).size(LabelSize::Small).color(
                            if is_success {
                                Color::Success
                            } else {
                                Color::Error
                            },
                        )),
                );
        }

        div()
            .w_full()
            .px(px(2.))
            .py_3()
            .border_t_1()
            .border_color(cx.colors().border_variant)
            .child(
                Label::new("Review Actions")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            )
            .child(el)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        if self.view_mode == PrsPanelView::Detail {
            if key == "escape" {
                self.go_back(cx);
                cx.stop_propagation();
            }
            return;
        }

        let count = self.prs.len();
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
                    self.select_pr(index, cx);
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.view_mode == PrsPanelView::Detail {
            return div()
                .h_flex()
                .w_full()
                .h(px(32.))
                .px(px(12.))
                .gap(px(6.))
                .items_center()
                .bg(cx.colors().surface_background)
                .border_b_1()
                .border_color(cx.colors().border_variant)
                .child(
                    IconButton::new("prs-back", IconName::ChevronLeft)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go_back(cx);
                        })),
                )
                .child(
                    Label::new("Pull Request Detail")
                        .size(LabelSize::Small)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Default),
                )
                .into_any_element();
        }

        let is_open = self.filter == PrFilter::Open;
        let is_closed = self.filter == PrFilter::Closed;
        let is_all = self.filter == PrFilter::All;

        let pr_count: SharedString = format!("{}", self.prs.len()).into();

        div()
            .h_flex()
            .w_full()
            .h(px(32.))
            .px(px(12.))
            .gap(px(6.))
            .items_center()
            .bg(cx.colors().surface_background)
            .border_b_1()
            .border_color(cx.colors().border_variant)
            .child(
                Icon::new(IconName::GitPullRequest)
                    .size(IconSize::XSmall)
                    .color(Color::Accent),
            )
            .child(
                Label::new("Pull Requests")
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Default),
            )
            .when(!self.prs.is_empty(), |el| {
                el.child(Badge::new(pr_count).color(Color::Muted))
            })
            .child(div().flex_1())
            .child(
                div()
                    .h_flex()
                    .h(px(22.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(cx.colors().border_variant)
                    .overflow_hidden()
                    .child(
                        Button::new("pr-filter-open", "Open")
                            .size(ButtonSize::Compact)
                            .style(if is_open {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Transparent
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_filter(PrFilter::Open, cx);
                            })),
                    )
                    .child(
                        Button::new("pr-filter-closed", "Closed")
                            .size(ButtonSize::Compact)
                            .style(if is_closed {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Transparent
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_filter(PrFilter::Closed, cx);
                            })),
                    )
                    .child(
                        Button::new("pr-filter-all", "All")
                            .size(ButtonSize::Compact)
                            .style(if is_all {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Transparent
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_filter(PrFilter::All, cx);
                            })),
                    ),
            )
            .child(
                Button::new("prs-new", "New pull request")
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Filled)
                    .color(Color::Accent)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_create_pr_dialog(cx);
                    })),
            )
            .child(
                IconButton::new("prs-refresh", IconName::Refresh)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.force_fetch_prs(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(pr) = &self.selected_pr else {
            return div().into_any_element();
        };

        // We'll get colors inline to avoid borrow conflicts with render_review_actions
        let title: SharedString = pr.title.clone().into();
        let number_text: SharedString = format!("#{}", pr.number).into();
        let author: SharedString = pr.author.clone().into();
        let created: SharedString = pr.created_at.clone().into();

        let (state_label, state_icon, state_color) = match pr.state {
            PrState::Open => ("Open", IconName::GitPullRequest, Color::Success),
            PrState::Closed => ("Closed", IconName::GitPullRequest, Color::Error),
            PrState::Merged => ("Merged", IconName::GitMerge, Color::Accent),
        };

        let mut content = div()
            .id("pr-detail-scroll")
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
            .bg(cx.colors().elevated_surface_background)
            .rounded(px(8.))
            .border_1()
            .border_color(cx.colors().border_variant);

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

        // State + draft badge + author + date row
        let mut meta_row = div().h_flex().gap_2().items_center().child(
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
        );

        if pr.draft {
            meta_row = meta_row.child(Badge::new("Draft").color(Color::Muted));
        }

        meta_row = meta_row
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
            );

        card = card.child(meta_row);

        // Branch info
        let branch_text: SharedString = format!("{} -> {}", pr.head_branch, pr.base_branch).into();
        card = card.child(
            div()
                .h_flex()
                .gap(px(4.))
                .items_center()
                .child(
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new(branch_text)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if !pr.labels.is_empty() {
            let mut labels_row = div().h_flex().gap(px(4.)).flex_wrap();
            for label in &pr.labels {
                let label_name: SharedString = label.name.clone().into();
                labels_row = labels_row.child(Badge::new(label_name).color(label_color(label)));
            }
            card = card.child(labels_row);
        }

        if let Some(body) = &pr.body {
            if !body.is_empty() {
                card = card.child(
                    div()
                        .w_full()
                        .pt_2()
                        .mt_1()
                        .border_t_1()
                        .border_color(cx.colors().border_variant)
                        .child(render_markdown(body, window, cx)),
                );
            }
        }

        content = content.child(card);

        // Review actions — only for open PRs
        if matches!(pr.state, PrState::Open) {
            content = content.child(self.render_review_actions(cx));
        }

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
                        .id(ElementId::NamedInteger("pr-comment".into(), i as u64))
                        .v_flex()
                        .w_full()
                        .px(px(14.))
                        .py(px(12.))
                        .gap(px(8.))
                        .bg(cx.colors().elevated_surface_background)
                        .rounded(px(8.))
                        .border_1()
                        .border_color(cx.colors().border_variant)
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
                                        .bg(cx.colors().ghost_element_selected)
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
                    .bg(cx.colors().ghost_element_background)
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
        let skeleton_bg = cx.colors().ghost_element_background;

        div()
            .v_flex()
            .flex_1()
            .w_full()
            .p_2()
            .gap(px(2.))
            .children((0..6).map(move |i| {
                div()
                    .id(ElementId::NamedInteger("pr-skeleton-row".into(), i))
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

impl Render for PrsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = cx.colors().panel_background;
        let ghost_selected = cx.colors().ghost_element_selected;
        let text_accent = cx.colors().text_accent;
        let ghost_hover = cx.colors().ghost_element_hover;
        let border_transparent = cx.colors().border_transparent;

        let mut panel = div()
            .id("prs-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(panel_bg);

        panel = panel.child(self.render_toolbar(cx));

        if self.view_mode == PrsPanelView::Detail {
            panel = panel.child(self.render_detail(window, cx));
            return panel.into_any_element();
        }

        if self.auth_required && self.github_token.as_deref().unwrap_or("").is_empty() {
            return panel
                .child(self.render_empty_state(
                    IconName::Settings,
                    "Sign in to view pull requests",
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
                                    Button::new("retry-prs", "Retry")
                                        .icon(IconName::Refresh)
                                        .size(ButtonSize::Default)
                                        .style(ButtonStyle::Filled)
                                        .color(Color::Accent)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.force_fetch_prs(cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element();
        }

        if self.prs.is_empty() {
            let empty_msg = format!(
                "No {} pull requests found",
                self.filter.label().to_lowercase()
            );
            return panel
                .child(self.render_empty_state(
                    IconName::CheckCircle,
                    &empty_msg,
                    "Try a different filter or check back later",
                    cx,
                ))
                .into_any_element();
        }

        let prs_snapshot: Arc<[PullRequest]> = self.prs.clone();
        let selected_index = self.selected_index;
        let weak = cx.weak_entity();

        let selected_bg = ghost_selected;
        let selected_border = text_accent;
        let hover_bg = ghost_hover;
        let transparent_border = border_transparent;

        panel = panel.child(
            uniform_list(
                "prs-list",
                prs_snapshot.len(),
                move |range: std::ops::Range<usize>, _window: &mut Window, _cx: &mut App| {
                    range
                        .map(|ix| {
                            let pr = &prs_snapshot[ix];
                            let selected = selected_index == Some(ix);
                            let weak = weak.clone();

                            let (state_icon, state_color) = match pr.state {
                                PrState::Open => (IconName::GitPullRequest, Color::Success),
                                PrState::Closed => (IconName::GitPullRequest, Color::Error),
                                PrState::Merged => (IconName::GitMerge, Color::Accent),
                            };

                            let number_text: SharedString = format!("#{}", pr.number).into();
                            let title: SharedString = pr.title.clone().into();
                            let author: SharedString = pr.author.clone().into();
                            let timestamp: SharedString = pr.created_at.clone().into();
                            let labels = pr.labels.clone();
                            let comments_count = pr.comments_count;
                            let is_draft = pr.draft;

                            let mut row = div()
                                .id(ElementId::NamedInteger("pr-row".into(), ix as u64))
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
                                        this.select_pr(ix, cx);
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

                            if is_draft {
                                row = row.child(Badge::new("Draft").color(Color::Muted));
                            }

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

pub(crate) fn github_prs_url(owner: &str, repo: &str, state: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/pulls?state={}&per_page=100&sort=updated",
        owner, repo, state
    )
}

fn parse_github_prs(bodies: &[String]) -> Result<Vec<PullRequest>, GithubCollectionError> {
    let mut prs = Vec::new();
    for body in bodies {
        let json: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| GithubCollectionError::transport(e.to_string()))?;
        let items = json
            .as_array()
            .ok_or_else(|| GithubCollectionError::transport("Expected JSON array".into()))?;

        for item in items {
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
            let merged_at = item.get("merged_at").and_then(|v| v.as_str());
            let state = if merged_at.is_some() {
                PrState::Merged
            } else if state_str == "closed" {
                PrState::Closed
            } else {
                PrState::Open
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
            let draft = item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);
            let comments_count = item.get("comments").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

            let head_branch = item
                .get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let base_branch = item
                .get("base")
                .and_then(|b| b.get("ref"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

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
                    labels.push(PrLabel { name, color });
                }
            }

            let created_display = format_github_date(&created_at);

            prs.push(PullRequest {
                number,
                title,
                body: body_text,
                state,
                author,
                created_at: created_display,
                labels,
                head_branch,
                base_branch,
                draft,
                comments_count,
            });
        }
    }

    Ok(prs)
}

fn parse_pr_comments(bodies: &[String]) -> Result<Vec<PrComment>, String> {
    // Use the issues comments endpoint — works for PRs since they're also issues
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

            comments.push(PrComment {
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
fn label_color(label: &PrLabel) -> Color {
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

impl std::fmt::Display for ReviewAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewAction::Approve => write!(f, "APPROVE"),
            ReviewAction::RequestChanges => write!(f, "REQUEST_CHANGES"),
            ReviewAction::Comment => write!(f, "COMMENT"),
        }
    }
}

/// Submit a review on a pull request.
/// POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews
async fn submit_pr_review(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u64,
    action: ReviewAction,
    body: &str,
) -> Result<(), String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/pulls/{}/reviews",
        owner, repo, pr_number
    );

    let json_body = serde_json::json!({
        "body": body,
        "event": action.to_string(),
    });

    let request = http_client::http::Request::builder()
        .method(http_client::http::Method::POST)
        .uri(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(
            serde_json::to_string(&json_body).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;

    let response = http.send(request).await.map_err(|e| e.to_string())?;
    let status = response.status();
    let mut body_str = String::new();
    let mut reader = response.into_body();
    reader
        .read_to_string(&mut body_str)
        .await
        .map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format_github_detail_error(status, &body_str));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // PrFilter::api_value

    #[test]
    fn pr_filter_api_value_open() {
        assert_eq!(PrFilter::Open.api_value(), "open");
    }

    #[test]
    fn pr_filter_api_value_closed() {
        assert_eq!(PrFilter::Closed.api_value(), "closed");
    }

    #[test]
    fn pr_filter_api_value_all() {
        assert_eq!(PrFilter::All.api_value(), "all");
    }

    // PrFilter::label

    #[test]
    fn pr_filter_label_open() {
        assert_eq!(PrFilter::Open.label(), "Open");
    }

    #[test]
    fn pr_filter_label_closed() {
        assert_eq!(PrFilter::Closed.label(), "Closed");
    }

    #[test]
    fn pr_filter_label_all() {
        assert_eq!(PrFilter::All.label(), "All");
    }

    #[test]
    fn request_keys_partition_filter_repo_and_auth() {
        let identity = GithubRequestIdentity {
            owner: "owner".into(),
            repo: "repo".into(),
            auth_fingerprint: token_fingerprint(Some("token-a")),
        };
        let open = PrListRequestKey {
            identity: identity.clone(),
            filter: PrFilter::Open,
        };
        let closed = PrListRequestKey {
            identity: identity.clone(),
            filter: PrFilter::Closed,
        };
        let other_auth = PrListRequestKey {
            identity: GithubRequestIdentity {
                auth_fingerprint: token_fingerprint(Some("token-b")),
                ..identity
            },
            filter: PrFilter::Open,
        };

        assert_ne!(open, closed);
        assert_ne!(open, other_auth);
    }

    #[test]
    fn loaded_state_depends_on_request_key_not_row_count() {
        let key = PrListRequestKey {
            identity: GithubRequestIdentity {
                owner: "owner".into(),
                repo: "empty-repo".into(),
                auth_fingerprint: None,
            },
            filter: PrFilter::Open,
        };

        assert!(pr_list_key_is_loaded(Some(&key), Some(&key)));
        assert!(!pr_list_key_is_loaded(None, Some(&key)));
    }

    #[test]
    fn current_request_requires_matching_generation_and_key() {
        let key = PrListRequestKey {
            identity: GithubRequestIdentity {
                owner: "owner".into(),
                repo: "repo".into(),
                auth_fingerprint: None,
            },
            filter: PrFilter::Open,
        };
        let other_key = PrListRequestKey {
            filter: PrFilter::Closed,
            ..key.clone()
        };
        let active = (11, key.clone());

        assert!(request_is_current(Some(&active), 11, &key));
        assert!(!request_is_current(Some(&active), 10, &key));
        assert!(!request_is_current(Some(&active), 11, &other_key));
        assert!(!request_is_current(None, 11, &key));
    }

    // format_github_date

    #[test]
    fn format_github_date_full_iso() {
        assert_eq!(format_github_date("2024-01-15T10:30:00Z"), "2024-01-15");
    }

    #[test]
    fn format_github_date_already_short() {
        assert_eq!(format_github_date("2024-01-15"), "2024-01-15");
    }

    #[test]
    fn format_github_date_empty() {
        assert_eq!(format_github_date(""), "");
    }

    #[test]
    fn format_github_date_truncated() {
        // len < 10, returns as-is
        assert_eq!(format_github_date("2024-1-5"), "2024-1-5");
    }

    #[test]
    fn format_github_date_exactly_10_chars() {
        assert_eq!(format_github_date("2024-01-15"), "2024-01-15");
    }

    #[test]
    fn format_github_date_much_longer() {
        // longer than 10, slices first 10
        assert_eq!(format_github_date("2024-01-15T10:30:00Z"), "2024-01-15");
    }

    // ReviewAction::to_string

    #[test]
    fn review_action_to_string_approve() {
        assert_eq!(ReviewAction::Approve.to_string(), "APPROVE");
    }

    #[test]
    fn review_action_to_string_request_changes() {
        assert_eq!(ReviewAction::RequestChanges.to_string(), "REQUEST_CHANGES");
    }

    #[test]
    fn review_action_to_string_comment() {
        assert_eq!(ReviewAction::Comment.to_string(), "COMMENT");
    }

    // PrState

    #[test]
    fn pr_state_to_string_open() {
        assert_eq!(PrState::Open.to_string(), "open");
    }

    #[test]
    fn pr_state_to_string_closed() {
        assert_eq!(PrState::Closed.to_string(), "closed");
    }

    #[test]
    fn pr_state_to_string_merged() {
        assert_eq!(PrState::Merged.to_string(), "merged");
    }

    #[test]
    fn pr_state_from_str_open() {
        assert_eq!(PrState::from_str("open"), Ok(PrState::Open));
    }

    #[test]
    fn pr_state_from_str_closed() {
        assert_eq!(PrState::from_str("closed"), Ok(PrState::Closed));
    }

    #[test]
    fn pr_state_from_str_merged() {
        assert_eq!(PrState::from_str("merged"), Ok(PrState::Merged));
    }

    #[test]
    fn pr_state_from_str_unknown() {
        assert_eq!(PrState::from_str("banana"), Err(()));
    }

    // PrLabel

    #[test]
    fn pr_label_creation() {
        let label = PrLabel {
            name: "bug".into(),
            color: "ff0000".into(),
        };
        assert_eq!(label.name, "bug");
        assert_eq!(label.color, "ff0000");
    }

    // PullRequest

    #[test]
    fn pull_request_creation() {
        let pr = PullRequest {
            number: 42,
            title: "Add feature".into(),
            body: Some("Implementation details".into()),
            state: PrState::Open,
            author: "alice".into(),
            created_at: "2024-01-15T10:30:00Z".into(),
            labels: vec![PrLabel {
                name: "enhancement".into(),
                color: "00ff00".into(),
            }],
            head_branch: "feature-x".into(),
            base_branch: "main".into(),
            draft: false,
            comments_count: 3,
        };
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Add feature");
        assert!(pr.body.is_some());
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.labels.len(), 1);
        assert!(!pr.draft);
        assert_eq!(pr.comments_count, 3);
    }

    // PrComment

    #[test]
    fn pr_comment_creation() {
        let comment = PrComment {
            body: "Looks good!".into(),
            author: "bob".into(),
            created_at: "2024-02-01T12:00:00Z".into(),
        };
        assert_eq!(comment.body, "Looks good!");
        assert_eq!(comment.author, "bob");
    }
}
