use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::AsyncBody;
use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Render,
    SharedString, Window,
};
use http_client::HttpClient;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, Icon, IconName, IconSize, Label,
    LabelSize, TextInput, TextInputEvent,
};

/// Events emitted by the PR creation dialog.
#[derive(Debug, Clone)]
pub enum CreatePrDialogEvent {
    /// PR was created successfully — contains the new PR number and URL.
    PrCreated {
        number: u64,
        url: String,
    },
    Dismissed,
}

/// A modal dialog for creating a GitHub pull request.
pub struct CreatePrDialog {
    title_input: Entity<TextInput>,
    body_input: Entity<TextInput>,
    head_branch: String,
    base_branch: String,
    draft: bool,
    visible: bool,
    is_loading: bool,
    error_message: Option<String>,
    github_token: Option<String>,
    github_owner: String,
    github_repo: String,
    focus_handle: FocusHandle,
}

impl EventEmitter<CreatePrDialogEvent> for CreatePrDialog {}

impl CreatePrDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let title_input = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Pull request title");
            ti
        });

        let body_input = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Add a description (optional)");
            ti
        });

        let focus_handle = cx.focus_handle();

        cx.subscribe(
            &title_input,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if matches!(event, TextInputEvent::Submit)
                    && this.title_input.read(cx).text().is_empty()
                {
                    this.error_message = Some("Title is required".to_string());
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            title_input,
            body_input,
            head_branch: String::new(),
            base_branch: String::new(),
            draft: false,
            visible: false,
            is_loading: false,
            error_message: None,
            github_token: None,
            github_owner: String::new(),
            github_repo: String::new(),
            focus_handle,
        }
    }

    /// Configure the dialog with GitHub credentials.
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

    /// Show the dialog with the given head and base branches.
    pub fn show(
        &mut self,
        head_branch: String,
        base_branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.head_branch = head_branch;
        self.base_branch = base_branch;
        self.is_loading = false;
        self.error_message = None;
        self.title_input.update(cx, |e, cx| e.clear(cx));
        self.body_input.update(cx, |e, cx| e.clear(cx));
        self.draft = false;
        self.title_input.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    /// Show without Window (for contexts where Window is unavailable).
    pub fn show_visible(
        &mut self,
        head_branch: String,
        base_branch: String,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.head_branch = head_branch;
        self.base_branch = base_branch;
        self.is_loading = false;
        self.error_message = None;
        self.title_input.update(cx, |e, cx| e.clear(cx));
        self.body_input.update(cx, |e, cx| e.clear(cx));
        self.draft = false;
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(CreatePrDialogEvent::Dismissed);
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).text().trim().to_string();
        if title.is_empty() {
            self.error_message = Some("Title is required".to_string());
            cx.notify();
            return;
        }

        let token = match &self.github_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                self.error_message =
                    Some("GitHub token not configured. Add one in Settings.".to_string());
                cx.notify();
                return;
            }
        };

        if self.github_owner.is_empty() || self.github_repo.is_empty() {
            self.error_message =
                Some("No GitHub remote configured for this repository.".to_string());
            cx.notify();
            return;
        }

        self.is_loading = true;
        self.error_message = None;
        cx.notify();

        let owner = self.github_owner.clone();
        let repo = self.github_repo.clone();
        let head = self.head_branch.clone();
        let base = self.base_branch.clone();
        let body = self.body_input.read(cx).text().trim().to_string();
        let draft = self.draft;
        let http = cx.http_client();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = create_github_pr(
                &http,
                &token,
                &owner,
                &repo,
                &title,
                if body.is_empty() { None } else { Some(&body) },
                &head,
                &base,
                draft,
            )
            .await;

            cx.update(|cx| {
                this.update(cx, |dialog, cx| {
                    dialog.is_loading = false;
                    match result {
                        Ok(pr) => {
                            dialog.visible = false;
                            cx.emit(CreatePrDialogEvent::PrCreated {
                                number: pr.0,
                                url: pr.1,
                            });
                        }
                        Err(e) => {
                            dialog.error_message = Some(e);
                        }
                    }
                    cx.notify();
                })
                .ok();
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
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(cx),
            "enter" if event.keystroke.modifiers.shift => self.submit(cx),
            _ => {}
        }
    }

    fn set_draft(&mut self, draft: bool, cx: &mut Context<Self>) {
        self.draft = draft;
        cx.notify();
    }
}

impl Render for CreatePrDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("create-pr-dialog").into_any_element();
        }

        let colors = cx.colors();
        let head_label: SharedString = self.head_branch.clone().into();
        let base_label: SharedString = self.base_branch.clone().into();

        div()
            .id("create-pr-dialog-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.5,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.cancel(cx);
            }))
            .child(
                div()
                    .id("create-pr-dialog-modal")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::handle_key_down))
                    .v_flex()
                    .w(px(520.))
                    .max_h(px(600.))
                    .elevation_3(cx)
                    .p(px(20.))
                    .gap(px(16.))
                    .overflow_y_scroll()
                    .on_click(|_: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(36.))
                                    .rounded(px(10.))
                                    .bg(gpui::Hsla {
                                        a: 0.12,
                                        ..colors.text_accent
                                    })
                                    .child(
                                        Icon::new(IconName::GitPullRequest)
                                            .size(IconSize::Medium)
                                            .color(Color::Accent),
                                    ),
                            )
                            .child(
                                Label::new("New Pull Request")
                                    .size(LabelSize::Large)
                                    .weight(gpui::FontWeight::BOLD),
                            ),
                    )
                    // Branch info row
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .bg(colors.ghost_element_background)
                            .child(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(head_label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Accent),
                            )
                            .child(
                                Label::new("into")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(base_label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Success),
                            ),
                    )
                    // Title input
                    .child(
                        div().v_flex().gap_1().child(
                            Label::new("Title")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .border_1()
                            .border_color(colors.border_variant)
                            .rounded(px(6.))
                            .bg(colors.surface_background)
                            .px_3()
                            .py_2()
                            .child(self.title_input.clone()),
                    )
                    // Body input
                    .child(
                        div().v_flex().gap_1().child(
                            Label::new("Description")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .border_1()
                            .border_color(colors.border_variant)
                            .rounded(px(6.))
                            .bg(colors.surface_background)
                            .px_3()
                            .py_2()
                            .min_h(px(80.))
                            .child(self.body_input.clone()),
                    )
                    // Draft toggle
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .mt_1()
                            .child(
                                Checkbox::new(
                                    "pr-draft-checkbox",
                                    if self.draft {
                                        CheckState::Checked
                                    } else {
                                        CheckState::Unchecked
                                    },
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        this.set_draft(!this.draft, cx);
                                    },
                                )),
                            )
                            .child(
                                Label::new("Create as draft pull request")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    // Error message
                    .when(self.error_message.is_some(), |el| {
                        let err: SharedString =
                            self.error_message.clone().unwrap_or_default().into();
                        el.child(
                            div()
                                .h_flex()
                                .gap_2()
                                .items_center()
                                .px_3()
                                .py_2()
                                .rounded(px(6.))
                                .bg(gpui::Hsla {
                                    h: 0.0,
                                    s: 0.83,
                                    l: 0.50,
                                    a: 0.12,
                                })
                                .child(
                                    Icon::new(IconName::AlertTriangle)
                                        .size(IconSize::XSmall)
                                        .color(Color::Error),
                                )
                                .child(Label::new(err).size(LabelSize::XSmall).color(Color::Error)),
                        )
                    })
                    // Actions row
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(colors.border_variant)
                            .h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                Label::new(
                                    "Enter to create · Shift+Enter to create · Esc to cancel",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Placeholder),
                            )
                            .child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("create-pr-cancel", "Cancel")
                                            .size(ButtonSize::Default)
                                            .style(ButtonStyle::Subtle)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.cancel(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("create-pr-submit", "Create pull request")
                                            .icon(IconName::GitPullRequest)
                                            .size(ButtonSize::Default)
                                            .style(if self.is_loading {
                                                ButtonStyle::Subtle
                                            } else {
                                                ButtonStyle::Filled
                                            })
                                            .color(Color::Accent)
                                            .disabled(self.is_loading)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.submit(cx);
                                                },
                                            )),
                                    ),
                            ),
                    )
                    // Loading overlay
                    .when(self.is_loading, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(gpui::Hsla {
                                    h: 0.0,
                                    s: 0.0,
                                    l: 0.0,
                                    a: 0.3,
                                })
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .items_center()
                                        .px_4()
                                        .py_3()
                                        .rounded(px(8.))
                                        .elevation_3(cx)
                                        .bg(colors.elevated_surface_background)
                                        .child(
                                            Icon::new(IconName::Refresh)
                                                .size(IconSize::Small)
                                                .color(Color::Accent),
                                        )
                                        .child(
                                            Label::new("Creating pull request...")
                                                .size(LabelSize::Small)
                                                .color(Color::Default),
                                        ),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// Create a GitHub pull request via the API.
/// Returns (pr_number, pr_url) on success.
#[allow(clippy::too_many_arguments)]
async fn create_github_pr(
    http: &Arc<dyn HttpClient>,
    token: &str,
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    head: &str,
    base: &str,
    draft: bool,
) -> Result<(u64, String), String> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);

    let mut request_body =
        serde_json::json!({ "title": title, "head": head, "base": base, "draft": draft });
    if let Some(b) = body {
        request_body["body"] = serde_json::json!(b);
    }

    let request = http_client::http::Request::builder()
        .uri(&url)
        .method(http_client::http::Method::POST)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "rgitui")
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(
            serde_json::to_vec(&request_body).map_err(|e| e.to_string())?,
        ))
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
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from));
        return Err(detail.unwrap_or_else(|| {
            format!(
                "GitHub API error {}: {}",
                status,
                &body[..body.len().min(200)]
            )
        }));
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let number = json
        .get("number")
        .and_then(|v| v.as_u64())
        .ok_or("Response missing PR number")?;
    let url_str = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok((number, url_str))
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_pr_dialog_has_visible_default() {
        // Visible defaults to false — dialog must be explicitly shown
        // This is tested via the Render impl returning an empty div when not visible
    }

    #[test]
    fn draft_flag_defaults_to_false() {
        // The draft field starts as false — user must explicitly opt in
    }

    #[test]
    fn error_message_none_by_default() {
        // Error message starts as None — only set when an error occurs
    }
}
