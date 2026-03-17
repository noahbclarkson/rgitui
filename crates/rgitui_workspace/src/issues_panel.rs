use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Render,
    SharedString, UniformListScrollHandle, Window,
};
use http_client::HttpClient;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, Icon, IconButton, IconName, IconSize, Label, LabelSize,
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

        self.is_loading = true;
        self.error_message = None;
        cx.notify();

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let state = self.filter.api_value().to_string();
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
            let result =
                fetch_github_comments(&http, &token, &owner, &repo, issue_number).await;
            cx.update(|cx| {
                this.update(cx, |panel, cx| {
                    panel.comments_loading = false;
                    match result {
                        Ok(comments) => {
                            panel.selected_comments = comments.clone();
                            cx.emit(IssuesPanelEvent::IssueSelected(
                                issue_for_event,
                                comments,
                            ));
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
        if self.filter != filter {
            self.filter = filter;
            self.fetch_issues(cx);
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if self.view_mode == IssuesPanelView::Detail {
            return div()
                .h_flex()
                .w_full()
                .h(px(28.))
                .px(px(10.))
                .gap(px(4.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    IconButton::new("issues-back", IconName::ChevronLeft)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Transparent)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go_back(cx);
                        })),
                )
                .child(
                    Label::new("Issue Detail")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                )
                .into_any_element();
        }

        let is_open = self.filter == IssueFilter::Open;
        let is_closed = self.filter == IssueFilter::Closed;
        let is_all = self.filter == IssueFilter::All;

        div()
            .h_flex()
            .w_full()
            .h(px(28.))
            .px(px(10.))
            .gap(px(4.))
            .items_center()
            .bg(colors.surface_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                Icon::new(IconName::DotOutline)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new("Issues")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::SEMIBOLD)
                    .color(Color::Muted),
            )
            .child(div().flex_1())
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
            )
            .child(
                IconButton::new("issues-refresh", IconName::Refresh)
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Transparent)
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

        let (state_label, state_color) = match issue.state {
            IssueState::Open => ("Open", Color::Success),
            IssueState::Closed => ("Closed", Color::Accent),
        };

        let mut content = div()
            .id("issue-detail-scroll")
            .v_flex()
            .flex_1()
            .overflow_y_scroll();

        let mut card = div()
            .v_flex()
            .w_full()
            .mx_2()
            .mt_2()
            .px_3()
            .py_2()
            .gap(px(6.))
            .bg(colors.elevated_surface_background)
            .rounded(px(6.))
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
                            .size(LabelSize::Small)
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
                .child(Badge::new(SharedString::from(state_label)).color(state_color))
                .child(
                    Label::new(author)
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(
                    Label::new(created)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if !issue.labels.is_empty() {
            let mut labels_row = div().h_flex().gap_1().flex_wrap();
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
                        .pt_1()
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(
                            Label::new(body_text)
                                .size(LabelSize::XSmall)
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
                    .py_3()
                    .justify_center()
                    .child(
                        Label::new("Loading comments...")
                            .size(LabelSize::XSmall)
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
                    .h(px(28.))
                    .px(px(10.))
                    .mt_1()
                    .gap(px(4.))
                    .items_center()
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new(comments_header)
                            .size(LabelSize::XSmall)
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
                        .mx_2()
                        .mt_1()
                        .px_3()
                        .py_2()
                        .gap(px(4.))
                        .bg(colors.elevated_surface_background)
                        .rounded(px(6.))
                        .border_1()
                        .border_color(colors.border_variant)
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .items_center()
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
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                );
            }
        }

        content = content.child(div().h(px(8.)));

        content.into_any_element()
    }
}

impl Render for IssuesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Capture all needed colors upfront to avoid borrow conflicts
        let panel_bg = cx.colors().panel_background;
        let text_color = cx.colors().text;
        let ghost_selected = cx.colors().ghost_element_selected;
        let text_accent = cx.colors().text_accent;
        let ghost_hover = cx.colors().ghost_element_hover;

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
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .v_flex()
                                .items_center()
                                .gap(px(8.))
                                .px(px(24.))
                                .py(px(16.))
                                .rounded(px(8.))
                                .bg(gpui::Hsla {
                                    a: 0.03,
                                    ..text_color
                                })
                                .child(
                                    Icon::new(IconName::Settings)
                                        .size(IconSize::Large)
                                        .color(Color::Placeholder),
                                )
                                .child(
                                    Label::new("GitHub token required")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(
                                        "Configure a GitHub token in settings to view issues",
                                    )
                                    .size(LabelSize::XSmall)
                                    .color(Color::Placeholder),
                                ),
                        ),
                )
                .into_any_element();
        }

        if self.is_loading {
            return panel
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new("Loading issues...")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
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
                        .child(
                            div()
                                .v_flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    Label::new(err_text)
                                        .size(LabelSize::Small)
                                        .color(Color::Error),
                                )
                                .child(
                                    Button::new("retry-issues", "Retry")
                                        .size(ButtonSize::Compact)
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.fetch_issues(cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element();
        }

        if self.issues.is_empty() {
            let empty_msg: SharedString =
                format!("No {} issues found", self.filter.label().to_lowercase()).into();
            return panel
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(empty_msg)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
                .into_any_element();
        }

        let issues_snapshot: Vec<Issue> = self.issues.clone();
        let selected_index = self.selected_index;
        let weak = cx.weak_entity();

        let selected_bg = ghost_selected;
        let selected_border = text_accent;
        let hover_bg = ghost_hover;

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
                                IssueState::Closed => (IconName::Check, Color::Accent),
                            };

                            let number_text: SharedString =
                                format!("#{}", issue.number).into();
                            let title: SharedString = issue.title.clone().into();
                            let labels = issue.labels.clone();
                            let comments_count = issue.comments_count;

                            let mut row = div()
                                .id(ElementId::NamedInteger(
                                    "issue-row".into(),
                                    ix as u64,
                                ))
                                .h_flex()
                                .w_full()
                                .h(px(30.))
                                .px_3()
                                .gap(px(6.))
                                .items_center()
                                .flex_shrink_0()
                                .border_l_2()
                                .when(selected, |el| {
                                    el.bg(selected_bg).border_color(selected_border)
                                })
                                .when(!selected, |el| {
                                    el.border_color(gpui::Hsla {
                                        h: 0.0,
                                        s: 0.0,
                                        l: 0.0,
                                        a: 0.0,
                                    })
                                })
                                .hover(move |s| s.bg(hover_bg))
                                .cursor_pointer()
                                .on_click(
                                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                        weak.update(cx, |this, cx| {
                                            this.select_issue(ix, cx);
                                        })
                                        .ok();
                                    },
                                )
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
                                        .gap(px(4.))
                                        .overflow_hidden()
                                        .child(
                                            Label::new(title)
                                                .size(LabelSize::XSmall)
                                                .truncate(),
                                        ),
                                );

                            if !labels.is_empty() {
                                let mut labels_row =
                                    div().h_flex().gap(px(2.)).flex_shrink_0();
                                for label in labels.iter().take(3) {
                                    let label_name: SharedString =
                                        label.name.clone().into();
                                    labels_row = labels_row
                                        .child(Badge::new(label_name).color(Color::Info));
                                }
                                row = row.child(labels_row);
                            }

                            if comments_count > 0 {
                                let count_text: SharedString =
                                    format!("{}", comments_count).into();
                                row = row.child(
                                    div()
                                        .h_flex()
                                        .gap(px(2.))
                                        .flex_shrink_0()
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

    if !response.status().is_success() {
        return Err(format!("GitHub API error: {}", response.status()));
    }

    let mut body = String::new();
    let mut reader = response.into_body();
    reader
        .read_to_string(&mut body)
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let items = json.as_array().ok_or("Expected JSON array")?;

    let mut issues = Vec::new();
    for item in items {
        if item.get("pull_request").is_some() {
            continue;
        }

        let number = item
            .get("number")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let body_text = item
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let state_str = item
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("open");
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
        let comments_count = item
            .get("comments")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

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

    if !response.status().is_success() {
        return Err(format!("GitHub API error: {}", response.status()));
    }

    let mut body = String::new();
    let mut reader = response.into_body();
    reader
        .read_to_string(&mut body)
        .await
        .map_err(|e| e.to_string())?;

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
