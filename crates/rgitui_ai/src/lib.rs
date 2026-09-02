//! AI commit-message generation.
//!
//! ## Threading
//!
//! Nothing here runs the provider call or a tool on the UI thread. The whole
//! provider dispatch — HTTP, JSON parsing, `git` spawns, `read_dir` walks —
//! lives inside one `cx.background_executor().spawn(...)`, and progress comes
//! back over an `mpsc` channel that a small foreground task drains into
//! [`AiEvent::ToolCallStarted`]. Only `this.update(...)` stays on the
//! foreground.
//!
//! ## Lifecycle
//!
//! Every generation carries a monotonic `generation` id and the repo path it
//! was requested for, mirroring `refresh_generation` in `rgitui_git`. The
//! workspace drops events from a superseded generation and routes the result
//! by repo path, so a message generated for one tab can never land in
//! another's commit box.

pub mod catalog;
mod http;
mod prompt;
mod provider;
mod tools;

use anyhow::{Context as _, Result};
use futures::channel::mpsc;
use futures::StreamExt;
use gpui::http_client::{AsyncBody, HttpClient, HttpRequestExt, Method, Request};
use gpui::{AsyncApp, BackgroundExecutor, Context, EventEmitter, Task, WeakEntity};
use rgitui_settings::{AiProvider, SettingsState};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use prompt::CommitStyle;
pub use provider::{effective_host, validate_base_url, BaseUrlError};
pub use tools::{
    anthropic_tool_definitions, denied_path, execute_tool, gemini_tool_definitions,
    openai_tool_definitions, DeniedReason, ToolBudget, ToolCall, ToolResult,
};

use http::{
    is_retryable, parse_retry_after, read_response_body, retry_delay, MAX_RETRIES, REQUEST_TIMEOUT,
};
use prompt::{build_prompt, collect_project_context};
use provider::{
    auth_style, build_request_body, gemini_endpoint, openai_compat_endpoint, AuthStyle,
    OpenAiCompatEndpoint, ANTHROPIC_ENDPOINT, ANTHROPIC_VERSION,
};
use tools::execute_tool_within;

/// Identifies one generation attempt, so a superseded or cross-tab result can
/// be dropped instead of overwriting the wrong commit box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationId {
    /// Monotonic per-`AiGenerator`. The same guard `apply_refresh_data` uses.
    pub sequence: u64,
    /// The checkout the request describes. Routing by this rather than by
    /// `active_tab` is what keeps a message for repo `foo` out of repo `bar`.
    pub repo_path: PathBuf,
}

/// Events emitted by the AI system. Every variant carries its [`GenerationId`]
/// so a listener can tell whose result it is looking at.
#[derive(Debug, Clone)]
pub enum AiEvent {
    GenerationStarted(GenerationId),
    /// A tool call is being executed, with a human-readable description
    /// ("Reading diff.rs", "Reading 20 recent commits").
    ToolCallStarted(GenerationId, String),
    GenerationCompleted(GenerationId, String),
    GenerationFailed(GenerationId, String),
    /// A generation the user cancelled. Distinct from a failure so the UI can
    /// clear its spinner without raising an error.
    GenerationCancelled(GenerationId),
    /// The request was refused by the client-side cooldown. Informational —
    /// it does not clear an in-flight generation's spinner, which the old
    /// code did, freeing the button to start a third request.
    RateLimited {
        wait: Duration,
    },
}

/// Minimum time between AI requests (client-side rate limiting).
const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum number of provider round trips in one generation.
const MAX_TOOL_ITERATIONS: usize = 4;

/// What a generation needs, gathered on the UI thread and then owned entirely
/// by the background task.
struct GenerateRequest {
    client: Arc<dyn HttpClient>,
    /// Used only for the retry backoff timer, so the retry schedule does not
    /// need a second async runtime alongside GPUI's.
    executor: BackgroundExecutor,
    provider: AiProvider,
    api_key: Option<String>,
    model: String,
    prompt: String,
    repo_path: PathBuf,
    use_tools: bool,
    base_url_override: String,
    openrouter_attribution: bool,
}

/// Reports tool progress from the background task to the foreground.
type StatusSender = mpsc::UnboundedSender<String>;

/// AI commit message generator.
pub struct AiGenerator {
    /// The generation currently in flight, if any.
    active: Option<GenerationId>,
    /// Holds the running task. Dropping it cancels the generation — which is
    /// what makes both supersede-in-flight and the cancel button work.
    task: Option<Task<()>>,
    next_sequence: u64,
    /// Stamped on *completion*, not on dispatch. Stamping at dispatch let a
    /// 40-second generation permit a second concurrent one at t=5s.
    last_request_finished: Option<Instant>,
}

impl EventEmitter<AiEvent> for AiGenerator {}

impl Default for AiGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl AiGenerator {
    pub fn new() -> Self {
        Self {
            active: None,
            task: None,
            next_sequence: 0,
            last_request_finished: None,
        }
    }

    pub fn is_generating(&self) -> bool {
        self.active.is_some()
    }

    /// The generation currently in flight, if any.
    pub fn active(&self) -> Option<&GenerationId> {
        self.active.as_ref()
    }

    /// How long the caller must wait before the cooldown allows another
    /// request, or `None` if it may proceed now.
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        let finished = self.last_request_finished?;
        MIN_REQUEST_INTERVAL.checked_sub(finished.elapsed())
    }

    /// Abandon the in-flight generation. Dropping the task cancels the HTTP
    /// request and any pending tool execution with it.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active.take() else {
            return;
        };
        self.task = None;
        self.last_request_finished = Some(Instant::now());
        cx.emit(AiEvent::GenerationCancelled(id));
        cx.notify();
    }

    /// Human-readable description of a tool call for status display.
    pub fn describe_tool_call(call: &ToolCall) -> String {
        let args = &call.arguments;
        match call.name.as_str() {
            tools::TOOL_GET_FILE_CONTENT => {
                format!("Reading {}", args["path"].as_str().unwrap_or("?"))
            }
            tools::TOOL_GET_FILE_HISTORY => {
                format!("File history: {}", args["path"].as_str().unwrap_or("?"))
            }
            tools::TOOL_GET_RECENT_COMMITS => {
                format!(
                    "Reading {} recent commits",
                    args["count"].as_u64().unwrap_or(5)
                )
            }
            tools::TOOL_GET_DIFF => {
                format!("Reading {} diff", args["kind"].as_str().unwrap_or("staged"))
            }
            tools::TOOL_GET_BRANCH_LIST => "Listing branches".to_string(),
            tools::TOOL_GET_FILE_TREE => {
                format!("Scanning {}", args["path"].as_str().unwrap_or("."))
            }
            other => format!("Calling {}", other),
        }
    }

    /// Generate a commit message from a diff string and file summary.
    pub fn generate_commit_message(
        &mut self,
        diff: String,
        summary: String,
        repo_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<GenerationId> {
        self.generate_commit_message_with_tools(diff, summary, repo_path, false, cx)
    }

    /// Generate a commit message, optionally letting the model call tools.
    ///
    /// Returns the [`GenerationId`] that was started, or `None` when the
    /// request was refused — already generating, or inside the cooldown. The
    /// caller does not need to guard those cases itself, which is what keeps
    /// the button, Ctrl+G and the command palette from disagreeing.
    pub fn generate_commit_message_with_tools(
        &mut self,
        diff: String,
        summary: String,
        repo_path: PathBuf,
        use_tools: bool,
        cx: &mut Context<Self>,
    ) -> Option<GenerationId> {
        if self.is_generating() {
            return None;
        }
        if let Some(wait) = self.cooldown_remaining() {
            // Informational, and deliberately not a failure: reporting this as
            // `GenerationFailed` used to clear the spinner of a request that
            // was still running.
            cx.emit(AiEvent::RateLimited { wait });
            return None;
        }

        let settings_state = cx.global::<SettingsState>();
        let settings = settings_state.settings();
        let provider = settings.ai.provider;
        let api_key = settings_state.ai_api_key();
        let model = settings.ai.model.clone();
        let commit_style = CommitStyle::from_id(&settings.ai.commit_style).unwrap_or_default();
        let inject_project_context = settings.ai.inject_project_context;
        let base_url_override = settings.ai.base_url_override.clone();
        let openrouter_attribution = settings.ai.openrouter_attribution;

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let id = GenerationId {
            sequence: self.next_sequence,
            repo_path: repo_path.clone(),
        };
        self.active = Some(id.clone());
        cx.emit(AiEvent::GenerationStarted(id.clone()));
        cx.notify();

        let client = cx.http_client();
        let (status_tx, mut status_rx) = mpsc::unbounded::<String>();

        let task_id = id.clone();
        self.task = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                // Drain tool status onto the foreground while the request runs.
                // This is the only reason a foreground task exists at all.
                let status_id = task_id.clone();
                let status_this = this.clone();
                let status_pump = cx.spawn(async move |cx: &mut AsyncApp| {
                    while let Some(description) = status_rx.next().await {
                        let id = status_id.clone();
                        let _ = status_this.update(cx, |_this, cx| {
                            cx.emit(AiEvent::ToolCallStarted(id, description));
                            cx.notify();
                        });
                    }
                });

                let background_repo_path = task_id.repo_path.clone();
                let executor = cx.background_executor().clone();
                let result = executor
                    .clone()
                    .spawn(async move {
                        let project_context = if inject_project_context {
                            collect_project_context(&background_repo_path)
                        } else {
                            None
                        };
                        let request = GenerateRequest {
                            client,
                            executor,
                            provider,
                            api_key,
                            model,
                            prompt: build_prompt(
                                &diff,
                                &summary,
                                commit_style,
                                project_context.as_deref(),
                                use_tools,
                            ),
                            repo_path: background_repo_path,
                            use_tools,
                            base_url_override,
                            openrouter_attribution,
                        };
                        dispatch(&request, &status_tx).await
                    })
                    .await;

                // The sender is dropped with the background task, so the pump ends
                // on its own; awaiting it just keeps the task alive until then.
                status_pump.await;

                // `let _` rather than `?`: a released entity is not a generation
                // failure, and conflating the two makes "the window closed"
                // indistinguishable from a real error for any future caller.
                let _ = this.update(cx, |this, cx| {
                    // A superseded generation must not clear the state of the one
                    // that replaced it.
                    if this.active.as_ref() != Some(&task_id) {
                        return;
                    }
                    this.active = None;
                    this.last_request_finished = Some(Instant::now());
                    match result {
                        Ok(message) => {
                            cx.emit(AiEvent::GenerationCompleted(task_id.clone(), message))
                        }
                        Err(error) => cx.emit(AiEvent::GenerationFailed(
                            task_id.clone(),
                            describe_error(&error),
                        )),
                    }
                    cx.notify();
                });
            }),
        );

        Some(id)
    }
}

/// Turn an `anyhow` chain into the single actionable sentence the user sees.
///
/// The toast used to carry the whole context chain, which is precise and
/// unreadable. The provider layer already writes actionable sentences; this
/// just picks the outermost one and drops the plumbing.
fn describe_error(error: &anyhow::Error) -> String {
    error.to_string()
}

// ============================================================================
// Dispatch
// ============================================================================

/// Route to the right provider family. Three arms rather than the eight
/// near-identical ones this replaced: DeepSeek and OpenRouter are the OpenAI
/// shape with a different URL and, for OpenRouter, two optional headers.
async fn dispatch(req: &GenerateRequest, status: &StatusSender) -> Result<String> {
    match req.provider {
        AiProvider::Gemini => generate_gemini(req, status).await,
        AiProvider::Anthropic => generate_anthropic(req, status).await,
        other => {
            let endpoint =
                openai_compat_endpoint(other, &req.base_url_override, req.openrouter_attribution);
            generate_openai_compatible(req, &endpoint, status).await
        }
    }
}

fn api_key(req: &GenerateRequest) -> Result<&str> {
    req.api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .with_context(|| {
            format!(
                "No {} API key. Add one in Settings > AI.",
                req.provider.display_name()
            )
        })
}

/// Send one request, retrying rate limits and transient server errors with
/// backoff that honours `Retry-After`.
///
/// Returns the parsed JSON body. Every non-success status that survives the
/// retries is reported as a sentence the user can act on.
async fn send_json(
    req: &GenerateRequest,
    url: &str,
    extra_headers: &[(&str, &str)],
    body: &Value,
) -> Result<Value> {
    let key = api_key(req)?;
    let body_bytes = serde_json::to_vec(body)?;

    for attempt in 0..=MAX_RETRIES {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(url)
            .header("Content-Type", "application/json")
            // Without a deadline a provider that accepts the connection and
            // then stalls leaves the spinner running until the app restarts.
            .timeout(REQUEST_TIMEOUT);

        builder = match auth_style(req.provider) {
            AuthStyle::Bearer => builder.header("Authorization", format!("Bearer {key}")),
            AuthStyle::AnthropicHeader => builder
                .header("x-api-key", key)
                .header("anthropic-version", ANTHROPIC_VERSION),
            // A header, not `?key=`: query strings land in proxy logs,
            // TLS-inspecting middleboxes, and any error path that prints a URL.
            AuthStyle::GoogleHeader => builder.header("x-goog-api-key", key),
        };
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }

        let request = builder
            .body(AsyncBody::from(body_bytes.clone()))
            .with_context(|| {
                format!(
                    "Couldn't build the {} request.",
                    req.provider.display_name()
                )
            })?;

        let mut response = req.client.send(request).await.with_context(|| {
            format!(
                "Couldn't reach {}. Check your connection or proxy.",
                provider_host(req)
            )
        })?;

        let status = response.status();
        let retry_after = parse_retry_after(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        );
        let raw = read_response_body(&mut response).await?;

        if status.is_success() {
            return serde_json::from_slice(&raw).with_context(|| {
                format!("Couldn't read {}'s response.", req.provider.display_name())
            });
        }

        if is_retryable(status.as_u16()) && attempt < MAX_RETRIES {
            let delay = retry_delay(attempt, retry_after);
            log::warn!(
                "{} returned {}; retrying in {:?}",
                req.provider.display_name(),
                status,
                delay
            );
            req.executor.timer(delay).await;
            continue;
        }

        return Err(status_error(req, status.as_u16(), &raw));
    }

    unreachable!("the retry loop always returns on its final attempt")
}

fn provider_host(req: &GenerateRequest) -> String {
    effective_host(req.provider, &req.base_url_override)
}

/// Map a failing status onto the sentence that tells the user what to do.
fn status_error(req: &GenerateRequest, status: u16, body: &[u8]) -> anyhow::Error {
    let name = req.provider.display_name();
    match status {
        401 | 403 => {
            anyhow::anyhow!("{name} did not accept this API key. Check it in Settings > AI.")
        }
        // OpenRouter answers 404 when no live endpoint for the model supports
        // tool use. That is an opaque failure unless it is named.
        404 if req.provider == AiProvider::OpenRouter && req.use_tools => anyhow::anyhow!(
            "{} does not support tool calling. Turn off \"Let the model read files\" in \
             Settings > AI, or pick a model marked Tools in the model list.",
            req.model
        ),
        404 => anyhow::anyhow!(
            "{} isn't available on this key. Pick another model or refresh the list.",
            req.model
        ),
        429 => anyhow::anyhow!("Rate limited by {name}. This usually clears in a minute."),
        500..=599 => anyhow::anyhow!("{name} is having trouble ({status}). Try again shortly."),
        _ => {
            let detail = provider_error_message(body)
                .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(300).collect());
            anyhow::anyhow!("{name} rejected the request ({status}): {detail}")
        }
    }
}

/// Pull the provider's own error sentence out of a body, whatever shape it
/// arrived in. Also covers the case where a gateway answers HTTP 200 with a
/// top-level `error` object.
fn provider_error_message(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    error_message_in(&json)
}

fn error_message_in(json: &Value) -> Option<String> {
    json["error"]["message"]
        .as_str()
        .or_else(|| json["error"]["msg"].as_str())
        .or_else(|| json["error"].as_str())
        .or_else(|| json["message"].as_str())
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

// ============================================================================
// OpenAI-compatible family: OpenAI, DeepSeek, OpenRouter
// ============================================================================

async fn generate_openai_compatible(
    req: &GenerateRequest,
    endpoint: &OpenAiCompatEndpoint,
    status: &StatusSender,
) -> Result<String> {
    let tools = req
        .use_tools
        .then(|| Value::Array(openai_tool_definitions()));
    let mut history: Vec<Value> = Vec::new();
    let mut budget = ToolBudget::new();

    for _ in 0..MAX_TOOL_ITERATIONS {
        let body = build_request_body(
            endpoint.provider,
            &req.model,
            &req.prompt,
            tools.as_ref(),
            &history,
        );
        let json = send_json(req, &endpoint.url, &endpoint.extra_headers, &body).await?;

        // A gateway can answer 200 with an error object; without this the
        // failure surfaces as the useless "No text in ... response".
        if let Some(message) = error_message_in(&json) {
            anyhow::bail!("{} reported: {}", endpoint.provider.display_name(), message);
        }

        let message = &json["choices"][0]["message"];
        let finish_reason = json["choices"][0]["finish_reason"].as_str().unwrap_or("");
        history.push(message.clone());

        let tool_calls: Vec<Value> = message["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if finish_reason == "tool_calls" || !tool_calls.is_empty() {
            // A `tool_calls` finish reason with no array used to push the
            // assistant turn and loop until the iteration cap, then report the
            // generic "Max tool iterations reached".
            if tool_calls.is_empty() {
                anyhow::bail!(
                    "{} asked to call a tool but sent no tool call. Try turning off \
                     \"Let the model read files\" in Settings > AI.",
                    endpoint.provider.display_name()
                );
            }
            for call in &tool_calls {
                let function = &call["function"];
                let tool_call = ToolCall {
                    id: call["id"].as_str().unwrap_or_default().to_string(),
                    name: function["name"].as_str().unwrap_or_default().to_string(),
                    arguments: serde_json::from_str(function["arguments"].as_str().unwrap_or("{}"))
                        .unwrap_or_else(|_| serde_json::json!({})),
                };
                let result = run_tool(&tool_call, &req.repo_path, &mut budget, status);
                history.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": result.call_id,
                    "content": tool_output(&result),
                }));
            }
            continue;
        }

        if let Some(content) = message["content"].as_str() {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }

        if finish_reason == "length" {
            anyhow::bail!(
                "The response hit the output token limit before finishing. Try a shorter diff."
            );
        }
        if finish_reason == "content_filter" {
            anyhow::bail!(
                "{} filtered this response. Try a different model.",
                endpoint.provider.display_name()
            );
        }

        anyhow::bail!(
            "{} returned no commit message (finish_reason={}).",
            endpoint.provider.display_name(),
            if finish_reason.is_empty() {
                "unset"
            } else {
                finish_reason
            }
        );
    }

    Err(iterations_exhausted())
}

// ============================================================================
// Anthropic
// ============================================================================

async fn generate_anthropic(req: &GenerateRequest, status: &StatusSender) -> Result<String> {
    let tools = req
        .use_tools
        .then(|| Value::Array(anthropic_tool_definitions()));
    let mut history: Vec<Value> = Vec::new();
    let mut budget = ToolBudget::new();

    for _ in 0..MAX_TOOL_ITERATIONS {
        // `build_request_body` seeds the opening user turn. Sending
        // `messages: []` is a 400, which is why this provider had never once
        // produced a commit message.
        let body = build_request_body(
            AiProvider::Anthropic,
            &req.model,
            &req.prompt,
            tools.as_ref(),
            &history,
        );
        let json = send_json(req, ANTHROPIC_ENDPOINT, &[], &body).await?;

        let stop_reason = json["stop_reason"].as_str().unwrap_or("");
        let content = json["content"].as_array().cloned().unwrap_or_default();

        if stop_reason == "tool_use" {
            let mut results: Vec<Value> = Vec::new();
            for block in &content {
                if block["type"].as_str() != Some("tool_use") {
                    continue;
                }
                let tool_call = ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    name: block["name"].as_str().unwrap_or_default().to_string(),
                    arguments: block["input"].clone(),
                };
                let result = run_tool(&tool_call, &req.repo_path, &mut budget, status);
                results.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": result.call_id,
                    "content": tool_output(&result),
                }));
            }

            if results.is_empty() {
                anyhow::bail!(
                    "Anthropic asked to call a tool but sent no tool call. Try turning off \
                     \"Let the model read files\" in Settings > AI."
                );
            }

            history.push(serde_json::json!({ "role": "assistant", "content": content }));
            history.push(serde_json::json!({ "role": "user", "content": results }));
            continue;
        }

        if let Some(text) = first_text_block(&content) {
            return Ok(text);
        }

        if stop_reason == "max_tokens" {
            anyhow::bail!(
                "The response hit the output token limit before finishing. Try a shorter diff."
            );
        }

        anyhow::bail!(
            "Anthropic returned no commit message (stop_reason={}).",
            if stop_reason.is_empty() {
                "unset"
            } else {
                stop_reason
            }
        );
    }

    Err(iterations_exhausted())
}

fn first_text_block(content: &[Value]) -> Option<String> {
    content
        .iter()
        .filter(|block| block["type"].as_str() == Some("text"))
        .filter_map(|block| block["text"].as_str())
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map(str::to_string)
}

// ============================================================================
// Gemini
// ============================================================================

async fn generate_gemini(req: &GenerateRequest, status: &StatusSender) -> Result<String> {
    let tools = req.use_tools.then(gemini_tool_definitions);
    let url = gemini_endpoint(&req.model);
    let mut history: Vec<Value> = Vec::new();
    let mut budget = ToolBudget::new();

    for _ in 0..MAX_TOOL_ITERATIONS {
        let body = build_request_body(
            AiProvider::Gemini,
            &req.model,
            &req.prompt,
            tools.as_ref(),
            &history,
        );
        let raw = send_json(req, &url, &[], &body).await?;
        let json: GeminiResponse =
            serde_json::from_value(raw).context("Couldn't read Gemini's response.")?;

        if let Some(reason) = json
            .prompt_feedback
            .as_ref()
            .and_then(|feedback| feedback.block_reason.as_deref())
        {
            anyhow::bail!("Gemini blocked this prompt ({reason}).");
        }

        let candidate = json
            .candidates
            .first()
            .context("Gemini returned no candidates.")?;
        let parts = candidate
            .content
            .as_ref()
            .map(|content| content.parts.as_slice())
            .unwrap_or_default();

        // Iterate every part. Gemini routinely returns a thought part followed
        // by a `functionCall`, or several parallel calls in one turn; taking
        // only the first dropped the rest and fell through to a parse error
        // whenever a thought part came first.
        let mut model_parts: Vec<Value> = Vec::new();
        let mut response_parts: Vec<Value> = Vec::new();
        for part in parts {
            let Some(call) = &part.function_call else {
                continue;
            };
            let tool_call = ToolCall {
                id: format!("gemini_{}", model_parts.len()),
                name: call.name.clone(),
                arguments: call.args.clone(),
            };
            let result = run_tool(&tool_call, &req.repo_path, &mut budget, status);

            // The thought signature must be echoed back verbatim so the model
            // retains reasoning continuity across the round trip.
            let mut model_part = serde_json::json!({
                "functionCall": { "name": call.name, "args": call.args }
            });
            if let Some(signature) = &part.thought_signature {
                model_part["thoughtSignature"] = Value::String(signature.clone());
            }
            model_parts.push(model_part);

            response_parts.push(serde_json::json!({
                "functionResponse": {
                    "name": call.name,
                    "response": { "content": tool_output(&result) }
                }
            }));
        }

        if !model_parts.is_empty() {
            history.push(serde_json::json!({ "role": "model", "parts": model_parts }));
            // The current REST API expects function results on a `user` turn;
            // `role: "function"` is a legacy spelling from other SDKs.
            history.push(serde_json::json!({ "role": "user", "parts": response_parts }));
            continue;
        }

        let text: String = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect::<Vec<_>>()
            .join("");
        let text = text.trim();
        if !text.is_empty() {
            return Ok(text.to_string());
        }

        // Everything below used to surface as the same "Failed to parse Gemini
        // response", which told the user nothing about what to do.
        match candidate.finish_reason.as_deref() {
            Some("MAX_TOKENS") => anyhow::bail!(
                "The response hit the output token limit before finishing. Try a shorter diff."
            ),
            Some("SAFETY") => anyhow::bail!("Gemini blocked this response (SAFETY)."),
            Some("RECITATION") => anyhow::bail!("Gemini blocked this response (RECITATION)."),
            Some(other) => anyhow::bail!("Gemini returned no commit message ({other})."),
            None => anyhow::bail!("Gemini returned no commit message."),
        }
    }

    Err(iterations_exhausted())
}

fn iterations_exhausted() -> anyhow::Error {
    anyhow::anyhow!(
        "The model kept asking for more context and never wrote a message. Try turning off \
         \"Let the model read files\" in Settings > AI."
    )
}

/// Execute a tool, reporting it to the UI first so the chip shows what is
/// happening rather than an opaque spinner.
fn run_tool(
    call: &ToolCall,
    repo_path: &Path,
    budget: &mut ToolBudget,
    status: &StatusSender,
) -> ToolResult {
    let _ = status.unbounded_send(AiGenerator::describe_tool_call(call));
    execute_tool_within(call, repo_path, budget)
}

/// A tool's output as the model sees it. Failures are handed back as text so
/// the model can adapt rather than the whole generation collapsing.
fn tool_output(result: &ToolResult) -> String {
    match &result.result {
        Ok(output) => output.clone(),
        Err(error) => format!("Error: {error}"),
    }
}

// ============================================================================
// Gemini response types
// ============================================================================

/// Every nested field is optional.
///
/// The non-tools variant used to require `candidates`, `content`, `parts` and
/// `text`, so a blocked prompt, a `SAFETY` finish, a thought-only part or a
/// `MAX_TOKENS` stop all produced the same opaque parse error.
#[derive(Debug, Default, Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "promptFeedback")]
    prompt_feedback: Option<GeminiPromptFeedback>,
}

#[derive(Debug, Deserialize)]
struct GeminiPromptFeedback {
    #[serde(default, rename = "blockReason")]
    block_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
    /// Opaque signature of the model's reasoning about this function call.
    /// Echoed back verbatim so reasoning continuity survives the round trip.
    #[serde(default, rename = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_gemini(json: Value) -> GeminiResponse {
        serde_json::from_value(json).expect("Gemini response must not fail to parse")
    }

    // ── M3/X4: the four shapes that all produced one opaque error ──

    #[test]
    fn a_safety_blocked_prompt_parses_and_names_the_reason() {
        let json = parse_gemini(serde_json::json!({
            "promptFeedback": { "blockReason": "SAFETY" }
        }));
        assert!(json.candidates.is_empty());
        assert_eq!(
            json.prompt_feedback.unwrap().block_reason.as_deref(),
            Some("SAFETY")
        );
    }

    #[test]
    fn a_candidate_with_no_content_parses() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{ "finishReason": "SAFETY" }]
        }));
        assert!(json.candidates[0].content.is_none());
        assert_eq!(json.candidates[0].finish_reason.as_deref(), Some("SAFETY"));
    }

    #[test]
    fn a_thought_part_carrying_no_text_parses() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "thoughtSignature": "abc" }] }
            }]
        }));
        let part = &json.candidates[0].content.as_ref().unwrap().parts[0];
        assert_eq!(part.text, None);
        assert_eq!(part.thought_signature.as_deref(), Some("abc"));
    }

    #[test]
    fn a_max_tokens_stop_with_empty_parts_parses() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{ "content": { "parts": [] }, "finishReason": "MAX_TOKENS" }]
        }));
        assert!(json.candidates[0]
            .content
            .as_ref()
            .unwrap()
            .parts
            .is_empty());
    }

    // ── X2: every part, not just the first ────────────────────────

    #[test]
    fn a_leading_thought_part_does_not_hide_the_function_call_behind_it() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{ "content": { "parts": [
                { "text": "" },
                { "functionCall": { "name": "get_diff", "args": { "kind": "staged" } } }
            ]}}]
        }));
        let parts = &json.candidates[0].content.as_ref().unwrap().parts;
        assert_eq!(parts.len(), 2);
        let calls: Vec<&str> = parts
            .iter()
            .filter_map(|p| p.function_call.as_ref())
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(calls, vec!["get_diff"]);
    }

    #[test]
    fn parallel_function_calls_in_one_turn_are_all_visible() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{ "content": { "parts": [
                { "functionCall": { "name": "get_diff", "args": {} } },
                { "functionCall": { "name": "get_branch_list", "args": {} } }
            ]}}]
        }));
        let calls: Vec<&str> = json.candidates[0]
            .content
            .as_ref()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| p.function_call.as_ref())
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(calls, vec!["get_diff", "get_branch_list"]);
    }

    #[test]
    fn multi_part_text_is_available_in_order() {
        let json = parse_gemini(serde_json::json!({
            "candidates": [{ "content": { "parts": [
                { "text": "feat: " },
                { "text": "do the thing" }
            ]}}]
        }));
        let text: String = json.candidates[0]
            .content
            .as_ref()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| p.text.as_deref())
            .collect();
        assert_eq!(text, "feat: do the thing");
    }

    // ── error extraction ──────────────────────────────────────────

    #[test]
    fn a_top_level_error_object_is_recognised_in_every_common_shape() {
        assert_eq!(
            error_message_in(&serde_json::json!({ "error": { "message": "bad key" } })).as_deref(),
            Some("bad key")
        );
        assert_eq!(
            error_message_in(&serde_json::json!({ "error": "plain string" })).as_deref(),
            Some("plain string")
        );
        assert_eq!(
            error_message_in(&serde_json::json!({ "message": "top level" })).as_deref(),
            Some("top level")
        );
    }

    #[test]
    fn an_ordinary_completion_is_not_mistaken_for_an_error() {
        let json = serde_json::json!({
            "choices": [{ "message": { "content": "feat: x" }, "finish_reason": "stop" }]
        });
        assert_eq!(error_message_in(&json), None);
    }

    #[test]
    fn an_empty_error_message_is_treated_as_absent() {
        assert_eq!(
            error_message_in(&serde_json::json!({ "error": { "message": "   " } })),
            None
        );
    }

    // ── Anthropic content blocks ──────────────────────────────────

    #[test]
    fn the_first_non_empty_text_block_wins_over_a_leading_thinking_block() {
        let content = vec![
            serde_json::json!({ "type": "thinking", "thinking": "hmm" }),
            serde_json::json!({ "type": "text", "text": "   " }),
            serde_json::json!({ "type": "text", "text": "  feat: x  " }),
        ];
        assert_eq!(first_text_block(&content).as_deref(), Some("feat: x"));
    }

    #[test]
    fn a_tool_use_only_response_yields_no_text() {
        let content = vec![serde_json::json!({ "type": "tool_use", "name": "get_diff" })];
        assert_eq!(first_text_block(&content), None);
    }

    // ── tool descriptions ─────────────────────────────────────────

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments,
        }
    }

    #[test]
    fn every_tool_gets_a_legible_progress_description() {
        let cases = [
            (
                call("get_file_content", serde_json::json!({ "path": "diff.rs" })),
                "Reading diff.rs",
            ),
            (
                call("get_file_history", serde_json::json!({ "path": "lib.rs" })),
                "File history: lib.rs",
            ),
            (
                call("get_recent_commits", serde_json::json!({ "count": 20 })),
                "Reading 20 recent commits",
            ),
            (
                call("get_diff", serde_json::json!({ "kind": "unstaged" })),
                "Reading unstaged diff",
            ),
            (
                call("get_branch_list", serde_json::json!({})),
                "Listing branches",
            ),
            (
                call("get_file_tree", serde_json::json!({ "path": "src" })),
                "Scanning src",
            ),
        ];
        for (tool_call, expected) in cases {
            assert_eq!(AiGenerator::describe_tool_call(&tool_call), expected);
        }
    }

    #[test]
    fn malformed_tool_arguments_still_produce_a_description() {
        assert_eq!(
            AiGenerator::describe_tool_call(&call("get_file_content", serde_json::json!({}))),
            "Reading ?"
        );
        assert_eq!(
            AiGenerator::describe_tool_call(&call("get_diff", Value::Null)),
            "Reading staged diff"
        );
        assert_eq!(
            AiGenerator::describe_tool_call(&call(
                "get_recent_commits",
                serde_json::json!({ "count": "many" })
            )),
            "Reading 5 recent commits"
        );
        assert_eq!(
            AiGenerator::describe_tool_call(&call("future_tool", serde_json::json!({}))),
            "Calling future_tool"
        );
    }

    // ── tool output plumbing ──────────────────────────────────────

    #[test]
    fn a_failed_tool_is_reported_to_the_model_rather_than_ending_the_generation() {
        let result = ToolResult {
            call_id: "1".into(),
            result: Err("Refused: .env looks like a credentials file".into()),
        };
        assert!(tool_output(&result).starts_with("Error: Refused"));
    }
}
