use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Render,
    SharedString, UniformListScrollHandle, Window,
};
use http_client::HttpClient;

use crate::github_api::{format_github_collection_error, format_github_detail_error};
use gpui::Entity;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
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

#[derive(Clone, Debug, PartialEq)]
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

pub struct IssuesPanel {
    issues: Vec<Issue>,
    selected_index: Option<usize>,
    is_loading: bool,
    error_message: Option<String>,
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
    last_fetched: Option<std::time::Instant>,
    search_query: Option<String>,
    is_searching: bool,
    query_input: Entity<TextInput>,
}

impl IssuesPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            issues: Vec::new(),
            selected_index: None,
            is_loading: false,
            error_message: None,
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
            last_fetched: None,
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
        !self.issues.is_empty()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
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

    pub fn fetch_issues(&mut self, cx: &mut Context<Self>) {
        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message = Some("No GitHub remote configured".into());
            cx.notify();
            return;
        }

        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                self.error_message =
                    Some("Configure a GitHub token in settings to view issues".into());
                cx.notify();
                return;
            }
        };

        // Skip refetch if data was loaded recently (within 60 seconds).
        if !self.issues.is_empty() {
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
        cx.notify();

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let state = self.filter.api_value().to_string();
        log::info!("fetch_issues: {}/{} filter={}", owner, repo, state);
        let http = cx.http_client();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = fetch_github_issues(&http, &token, &owner, &repo, &state).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.is_loading = false;
                    match result {
                        Ok(issues) => {
                            panel.issues = issues;
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            log::info!("fetch_issues complete: {} issues", panel.issues.len());
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

    fn select_issue(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(issue) = self.issues.get(index).cloned() else {
            return;
        };
        self.selected_index = Some(index);
        self.selected_issue = Some(issue.clone());
        self.selected_comments = Vec::new();
        self.comments_loading = true;
        self.view_mode = IssuesPanelView::Detail;
        cx.notify();

        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => return,
        };

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let issue_number = issue.number;
        let http = cx.http_client();
        let issue_for_event = issue;

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = fetch_github_comments(&http, &token, &owner, &repo, issue_number).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.comments_loading = false;
                    match result {
                        Ok(comments) => {
                            panel.selected_comments = comments.clone();
                            cx.emit(IssuesPanelEvent::IssueSelected(issue_for_event, comments));
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
        self.view_mode = IssuesPanelView::List;
        self.selected_issue = None;
        self.selected_comments = Vec::new();
        cx.notify();
    }

    fn set_filter(&mut self, filter: IssueFilter, cx: &mut Context<Self>) {
        self.is_searching = false;
        self.search_query = None;
        if self.filter != filter {
            self.filter = filter;
            self.last_fetched = None;
            self.fetch_issues(cx);
        }
    }

    fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.is_searching = !self.is_searching;
        if !self.is_searching {
            self.search_query = None;
            // Refetch normal list when exiting search
            if self.issues.is_empty() {
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
            cx.notify();
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
        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                self.error_message =
                    Some("Configure a GitHub token in settings to view issues".into());
                cx.notify();
                return;
            }
        };
        self.is_loading = true;
        self.error_message = None;
        cx.notify();
        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let http = cx.http_client();
        log::info!("search_issues: {}/{} query='{}'", owner, repo, query);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = fetch_github_issue_search(&http, &token, &owner, &repo, &query).await;
            cx.update(|cx| {
                let _ = this.update(cx, |panel, cx| {
                    panel.is_loading = false;
                    match result {
                        Ok(issues) => {
                            panel.issues = issues;
                            panel.selected_index = None;
                            panel.last_fetched = Some(std::time::Instant::now());
                            log::info!("search_issues complete: {} results", panel.issues.len());
                        }
                        Err(e) => {
                            panel.error_message = Some(e);
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
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
                Icon::new(IconName::GitPullRequest)
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
                        this.fetch_issues(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                labels_row = labels_row.child(Badge::new(label_name).color(Color::Info));
            }
            card = card.child(labels_row);
        }

        if let Some(body) = &issue.body {
            if !body.is_empty() {
                let body_text: SharedString = body.clone().into();
                card = card.child(
                    div()
                        .w_full()
                        .pt_2()
                        .mt_1()
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(
                            Label::new(body_text)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = cx.colors().panel_background;
        let ghost_selected = cx.colors().ghost_element_selected;
        let text_accent = cx.colors().text_accent;
        let ghost_hover = cx.colors().ghost_element_hover;
        let border_transparent = cx.colors().border_transparent;

        let mut panel = div()
            .id("issues-panel")
            .track_focus(&self.focus_handle)
            .v_flex()
            .size_full()
            .bg(panel_bg);

        panel = panel.child(self.render_toolbar(cx));

        if self.view_mode == IssuesPanelView::Detail {
            panel = panel.child(self.render_detail(cx));
            return panel.into_any_element();
        }

        if self.github_token.is_none() || self.github_token.as_deref() == Some("") {
            return panel
                .child(self.render_empty_state(
                    IconName::Settings,
                    "GitHub token required",
                    "Configure a GitHub token in settings to view issues",
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
                                Button::new("retry-issues", "Retry")
                                    .icon(IconName::Refresh)
                                    .size(ButtonSize::Default)
                                    .style(ButtonStyle::Filled)
                                    .color(Color::Accent)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.fetch_issues(cx);
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

        let issues_snapshot: Vec<Issue> = self.issues.clone();
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

async fn fetch_github_issues(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    state: &str,
) -> Result<Vec<Issue>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues?state={}&per_page=50&sort=updated",
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

    let mut issues = Vec::new();
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

    Ok(issues)
}

async fn fetch_github_issue_search(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    query: &str,
) -> Result<Vec<Issue>, String> {
    let url = format!(
        "https://api.github.com/search/issues?q={}+repo:{}/{}&per_page=50&sort=updated",
        urlencoding::encode(query),
        owner,
        repo
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
    let items = json
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or("Expected 'items' array in search response")?;

    let mut issues = Vec::new();
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

    Ok(issues)
}

async fn fetch_github_comments(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    issue_number: u64,
) -> Result<Vec<IssueComment>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments?per_page=50",
        owner, repo, issue_number
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

        comments.push(IssueComment {
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
