use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::prelude::*;
use gpui::{
    div, http_client::AsyncBody, px, uniform_list, App, ClickEvent, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Render, SharedString, UniformListScrollHandle, WeakEntity, Window,
};
use http_client::HttpClient;

use crate::github_api::{format_github_collection_error, format_github_detail_error};
use crate::Workspace;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
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

impl EventEmitter<PrsPanelEvent> for PrsPanel {}

#[derive(Clone, Debug, PartialEq)]
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

pub struct PrsPanel {
    prs: Vec<PullRequest>,
    selected_index: Option<usize>,
    is_loading: bool,
    error_message: Option<String>,
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
    workspace: WeakEntity<Workspace>,
    last_fetched: Option<std::time::Instant>,
    review_submitting: bool,
    review_comment_input: Entity<TextInput>,
    review_result: Option<String>,
}

impl PrsPanel {
    pub fn new(cx: &mut Context<Self>, workspace: WeakEntity<Workspace>) -> Self {
        Self {
            prs: Vec::new(),
            selected_index: None,
            is_loading: false,
            error_message: None,
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
            workspace,
            last_fetched: None,
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
        !self.prs.is_empty()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
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

    pub fn configure(
        &mut self,
        token: Option<String>,
        owner: String,
        repo: String,
        cx: &mut Context<Self>,
    ) {
        self.github_token = token;
        self.github_owner = owner;
        self.github_repo = repo;
        cx.notify();
    }

    pub fn fetch_prs(&mut self, cx: &mut Context<Self>) {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message = Some("No GitHub remote configured".into());
            cx.notify();
            return;
        }

        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                self.error_message =
                    Some("Configure a GitHub token in settings to view pull requests".into());
                cx.notify();
                return;
            }
        };

        // Skip refetch if data was loaded recently (within 60 seconds).
        if !self.prs.is_empty() {
            if let Some(last) = self.last_fetched {
                if last.elapsed() < std::time::Duration::from_secs(60) {
                    log::debug!("fetch_prs: cached, skipping");
                    return;
                }
            }
        }

        self.is_loading = true;
        self.error_message = None;
        cx.notify();

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let state = self.filter.api_value().to_string();
        log::info!("fetch_prs: {}/{} filter={}", owner, repo, state);
        let http = cx.http_client();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = fetch_github_prs(&http, &token, &owner, &repo, &state).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.is_loading = false;
                    match result {
                        Ok(prs) => {
                            panel.prs = prs;
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            log::info!("fetch_prs complete: {} PRs", panel.prs.len());
                        }
                        Err(e) => {
                            panel.error_message = Some(e);
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
        self.comments_loading = true;
        self.view_mode = PrsPanelView::Detail;
        cx.notify();

        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => return,
        };

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let pr_number = pr.number;
        let http = cx.http_client();
        let pr_for_event = pr;

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = fetch_pr_comments(&http, &token, &owner, &repo, pr_number).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.comments_loading = false;
                    match result {
                        Ok(comments) => {
                            panel.selected_comments = comments.clone();
                            cx.emit(PrsPanelEvent::PrSelected(pr_for_event, comments));
                        }
                        Err(e) => {
                            panel.error_message = Some(e);
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
        self.view_mode = PrsPanelView::List;
        self.selected_pr = None;
        self.selected_comments = Vec::new();
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
        let _colors = cx.colors();
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _colors = cx.colors();

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
                        this.fetch_prs(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                labels_row = labels_row.child(Badge::new(label_name).color(Color::Info));
            }
            card = card.child(labels_row);
        }

        if let Some(body) = &pr.body {
            if !body.is_empty() {
                let body_text: SharedString = body.clone().into();
                card = card.child(
                    div()
                        .w_full()
                        .pt_2()
                        .mt_1()
                        .border_t_1()
                        .border_color(cx.colors().border_variant)
                        .child(
                            Label::new(body_text)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
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
                let comment_body: SharedString = comment.body.clone().into();

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
                        .child(
                            Label::new(comment_body)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
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
        let _colors = cx.colors();
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
        let _colors = cx.colors();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = cx.colors().panel_background;
        let ghost_selected = cx.colors().ghost_element_selected;
        let text_accent = cx.colors().text_accent;
        let ghost_hover = cx.colors().ghost_element_hover;
        let border_transparent = cx.colors().border_transparent;

        let mut panel = div()
            .id("prs-panel")
            .track_focus(&self.focus_handle)
            .v_flex()
            .size_full()
            .bg(panel_bg);

        panel = panel.child(self.render_toolbar(cx));

        if self.view_mode == PrsPanelView::Detail {
            panel = panel.child(self.render_detail(cx));
            return panel.into_any_element();
        }

        if self.github_token.is_none() || self.github_token.as_deref() == Some("") {
            return panel
                .child(self.render_empty_state(
                    IconName::Settings,
                    "GitHub token required",
                    "Configure a GitHub token in settings to view pull requests",
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
                    div().flex_1().flex().items_center().justify_center().child(
                        div()
                            .v_flex()
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
                                Label::new(err_text)
                                    .size(LabelSize::Small)
                                    .color(Color::Error),
                            )
                            .child(
                                Button::new("retry-prs", "Retry")
                                    .icon(IconName::Refresh)
                                    .size(ButtonSize::Default)
                                    .style(ButtonStyle::Filled)
                                    .color(Color::Accent)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.fetch_prs(cx);
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

        let prs_snapshot: Vec<PullRequest> = self.prs.clone();
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
                                    labels_row =
                                        labels_row.child(Badge::new(label_name).color(Color::Info));
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

async fn fetch_github_prs(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    state: &str,
) -> Result<Vec<PullRequest>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/pulls?state={}&per_page=50&sort=updated",
        owner, repo, state
    );

    let request = http_client::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .body(Default::default())
        .map_err(|e| e.to_string())?;

    let response = http.send(request).await.map_err(|e| e.to_string())?;

    let status = response.status();
    let mut body = String::new();
    let mut reader = response.into_body();
    reader
        .read_to_string(&mut body)
        .await
        .map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format_github_collection_error(status, &body, owner, repo));
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let items = json.as_array().ok_or("Expected JSON array")?;

    let mut prs = Vec::new();
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

    Ok(prs)
}

async fn fetch_pr_comments(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<Vec<PrComment>, String> {
    // Use the issues comments endpoint — works for PRs since they're also issues
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments?per_page=50",
        owner, repo, pr_number
    );

    let request = http_client::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .body(Default::default())
        .map_err(|e| e.to_string())?;

    let response = http.send(request).await.map_err(|e| e.to_string())?;

    let status = response.status();
    let mut body = String::new();
    let mut reader = response.into_body();
    reader
        .read_to_string(&mut body)
        .await
        .map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format_github_detail_error(status, &body));
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let items = json.as_array().ok_or("Expected JSON array")?;

    let mut comments = Vec::new();
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

    Ok(comments)
}

fn format_github_date(date_str: &str) -> String {
    if date_str.len() >= 10 {
        date_str[..10].to_string()
    } else {
        date_str.to_string()
    }
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
}
