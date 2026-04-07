mod tools;

use anyhow::{Context as _, Result};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, Response};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use rgitui_settings::SettingsState;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
pub use tools::{
    anthropic_tool_definitions, execute_tool, gemini_tool_definitions, openai_tool_definitions,
    ToolCall, ToolResult,
};

/// Commit message style options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitStyle {
    /// Conventional Commits format: feat(scope): description
    Conventional,
    /// Plain English descriptive format
    #[default]
    Descriptive,
    /// One-line brief format
    Brief,
}

impl std::str::FromStr for CommitStyle {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "conventional" => Self::Conventional,
            "brief" => Self::Brief,
            _ => Self::Descriptive,
        })
    }
}

/// Events emitted by the AI system.
#[derive(Debug, Clone)]
pub enum AiEvent {
    GenerationStarted,
    /// A tool call is being executed. Contains a human-readable description.
    ToolCallStarted(String),
    GenerationCompleted(String),
    GenerationFailed(String),
}

/// Minimum time between AI requests (rate limiting).
const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(5);

/// AI commit message generator.
pub struct AiGenerator {
    is_generating: bool,
    last_result: Option<String>,
    last_error: Option<String>,
    last_request_time: Option<Instant>,
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
            is_generating: false,
            last_result: None,
            last_error: None,
            last_request_time: None,
        }
    }

    pub fn is_generating(&self) -> bool {
        self.is_generating
    }

    pub fn last_result(&self) -> Option<&str> {
        self.last_result.as_deref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Generate a commit message from a diff string and file summary.
    pub fn generate_commit_message(
        &mut self,
        diff: String,
        summary: String,
        repo_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<Result<String>> {
        self.generate_commit_message_with_tools(diff, summary, repo_path, false, cx)
    }

    /// Human-readable description of a tool call for status display.
    fn describe_tool_call(call: &ToolCall) -> String {
        let args = &call.arguments;
        match call.name.as_str() {
            "get_file_content" => {
                let path = args["path"].as_str().unwrap_or("?");
                format!("Reading {}", path)
            }
            "get_file_history" => {
                let path = args["path"].as_str().unwrap_or("?");
                format!("File history: {}", path)
            }
            "get_recent_commits" => {
                let n = args["count"].as_u64().unwrap_or(5);
                format!("Reading {} recent commits", n)
            }
            "get_diff" => {
                let kind = args["kind"].as_str().unwrap_or("staged");
                format!("Reading {} diff", kind)
            }
            "get_branch_list" => "Listing branches".to_string(),
            "get_file_tree" => {
                let path = args["path"].as_str().unwrap_or(".");
                format!("Scanning {}", path)
            }
            _ => format!("Calling {}", call.name),
        }
    }

    /// Generate a commit message with optional tool-calling support.
    pub fn generate_commit_message_with_tools(
        &mut self,
        diff: String,
        summary: String,
        repo_path: PathBuf,
        use_tools: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<String>> {
        if let Some(last_time) = self.last_request_time {
            let elapsed = last_time.elapsed();
            if elapsed < MIN_REQUEST_INTERVAL {
                let wait = MIN_REQUEST_INTERVAL - elapsed;
                let err_msg = format!("Rate limited. Please wait {:.0}s.", wait.as_secs_f32());
                self.last_error = Some(err_msg.clone());
                cx.emit(AiEvent::GenerationFailed(err_msg.clone()));
                cx.notify();
                return cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
                    Err(anyhow::anyhow!(err_msg))
                });
            }
        }

        self.is_generating = true;
        self.last_error = None;
        self.last_request_time = Some(Instant::now());
        cx.emit(AiEvent::GenerationStarted);
        cx.notify();

        let settings_state = cx.global::<SettingsState>();
        let settings = settings_state.settings().clone();
        let api_key = settings_state.ai_api_key();
        let model = settings.ai.model.clone();
        let provider = settings.ai.provider.clone();
        let commit_style = settings
            .ai
            .commit_style
            .parse::<CommitStyle>()
            .unwrap_or_default();
        let inject_project_context = settings.ai.inject_project_context;

        let client = cx.http_client();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let project_context = if inject_project_context {
                let repo_path_for_context = repo_path.clone();
                cx.background_executor()
                    .spawn(async move { collect_project_context(&repo_path_for_context) })
                    .await
            } else {
                None
            };
            let ctx_ref = project_context.as_deref();
            // Callback for tool call status — emits events to update the status bar.
            let mut on_tool_call = |desc: &str| {
                let msg = desc.to_string();
                let _ = this.update(cx, |_this, cx| {
                    cx.emit(AiEvent::ToolCallStarted(msg));
                    cx.notify();
                });
            };
            let result = match provider.as_str() {
                "gemini" => {
                    if use_tools {
                        generate_gemini_with_tools(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                            &repo_path,
                            &mut on_tool_call,
                        )
                        .await
                    } else {
                        generate_gemini(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                        )
                        .await
                    }
                }
                "openai" => {
                    if use_tools {
                        generate_openai_with_tools(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                            &repo_path,
                            &mut on_tool_call,
                        )
                        .await
                    } else {
                        generate_openai(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                        )
                        .await
                    }
                }
                "anthropic" => {
                    if use_tools {
                        generate_anthropic_with_tools(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                            &repo_path,
                            &mut on_tool_call,
                        )
                        .await
                    } else {
                        generate_anthropic(
                            &client,
                            &api_key,
                            &model,
                            &diff,
                            &summary,
                            commit_style,
                            ctx_ref,
                        )
                        .await
                    }
                }
                other => Err(anyhow::anyhow!("Unknown AI provider: {}", other)),
            };

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.is_generating = false;
                    match &result {
                        Ok(msg) => {
                            this.last_result = Some(msg.clone());
                            cx.emit(AiEvent::GenerationCompleted(msg.clone()));
                        }
                        Err(e) => {
                            let err = e.to_string();
                            this.last_error = Some(err.clone());
                            cx.emit(AiEvent::GenerationFailed(err));
                        }
                    }
                    cx.notify();
                })
            })?;

            result
        })
    }
}

async fn read_response_body(response: &mut Response<AsyncBody>) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .context("Failed to read response body")?;
    Ok(body)
}

/// Generate a commit message using the Gemini API.
async fn generate_gemini(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Gemini API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style, project_context);

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let json_body = serde_json::json!({
        "contents": [{
            "parts": [{
                "text": prompt
            }]
        }],
        "generationConfig": {
            "temperature": 0.3,
            "maxOutputTokens": 2048,
            "topP": 0.8
        }
    });

    let body_bytes = serde_json::to_vec(&json_body)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(&url)
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(body_bytes))
        .context("Failed to build Gemini request")?;

    let mut response = client
        .send(request)
        .await
        .context("Failed to send request to Gemini API")?;

    let status = response.status();
    let body = read_response_body(&mut response).await?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        anyhow::bail!("Gemini API error ({}): {}", status, text);
    }

    let json: GeminiResponse =
        serde_json::from_slice(&body).context("Failed to parse Gemini response")?;

    let text = json
        .candidates
        .first()
        .and_then(|c| c.content.parts.first())
        .map(|p| p.text.trim().to_string())
        .context("No text in Gemini response")?;

    Ok(text)
}

/// Generate a commit message using the OpenAI API.
async fn generate_openai(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("OpenAI API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style, project_context);

    let json_body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": prompt
        }],
        "temperature": 0.3,
        "max_tokens": 2048
    });

    let body_bytes = serde_json::to_vec(&json_body)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(body_bytes))
        .context("Failed to build OpenAI request")?;

    let mut response = client
        .send(request)
        .await
        .context("Failed to send request to OpenAI API")?;

    let status = response.status();
    let body = read_response_body(&mut response).await?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        anyhow::bail!("OpenAI API error ({}): {}", status, text);
    }

    let json: serde_json::Value =
        serde_json::from_slice(&body).context("Failed to parse OpenAI response")?;
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .context("No text in OpenAI response")?
        .trim()
        .to_string();

    Ok(text)
}

/// Generate a commit message using the Anthropic API.
async fn generate_anthropic(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Anthropic API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style, project_context);

    let json_body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [{
            "role": "user",
            "content": prompt
        }]
    });

    let body_bytes = serde_json::to_vec(&json_body)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key.as_str())
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(body_bytes))
        .context("Failed to build Anthropic request")?;

    let mut response = client
        .send(request)
        .await
        .context("Failed to send request to Anthropic API")?;

    let status = response.status();
    let body = read_response_body(&mut response).await?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        anyhow::bail!("Anthropic API error ({}): {}", status, text);
    }

    let json: serde_json::Value =
        serde_json::from_slice(&body).context("Failed to parse Anthropic response")?;
    let text = json["content"][0]["text"]
        .as_str()
        .context("No text in Anthropic response")?
        .trim()
        .to_string();

    Ok(text)
}

fn build_prompt(
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
) -> String {
    let style_instruction = match commit_style {
        CommitStyle::Conventional => {
            "Use the Conventional Commits format: <type>(<scope>): <description>\n\
             Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore\n\
             Keep the first line under 72 characters.\n\
             Add a blank line then a detailed body that explains what changed and why.\n\
             List the key changes as bullet points if there are multiple distinct changes."
        }
        CommitStyle::Descriptive => {
            "Write a clear, descriptive commit message.\n\
             First line: imperative mood summary under 72 characters.\n\
             Add a blank line then a detailed body that explains what changed and why.\n\
             List the key changes as bullet points if there are multiple distinct changes."
        }
        CommitStyle::Brief => {
            "Write a concise commit message in imperative mood.\n\
             Keep it to a single line under 72 characters."
        }
    };

    let max_diff_len = 200_000;
    let diff_text = if diff.len() > max_diff_len {
        let truncation_point = diff[..max_diff_len].rfind('\n').unwrap_or(max_diff_len);
        format!(
            "{}\n\n[diff truncated -- showing {}/{} bytes]",
            &diff[..truncation_point],
            truncation_point,
            diff.len()
        )
    } else {
        diff.to_string()
    };

    let context_section = match project_context {
        Some(context) => format!("Project Context:\n{context}\n\n"),
        None => String::new(),
    };

    format!(
        "You are a Git commit message generator. Generate ONLY the commit message, nothing else.\n\
         No markdown formatting, no code blocks, no explanations.\n\n\
         {style_instruction}\n\n\
         {context_section}\
         Files changed:\n{summary}\n\n\
         Diff:\n{diff_text}"
    )
}

const PROJECT_CONTEXT_FILES: &[&str] = &["README.md", "CLAUDE.md", "AGENTS.md"];
const MAX_PROJECT_CONTEXT_BYTES: usize = 50_000;

fn collect_project_context(repo_path: &Path) -> Option<String> {
    let mut combined = String::new();

    for filename in PROJECT_CONTEXT_FILES {
        let file_path = repo_path.join(filename);
        if let Ok(contents) = std::fs::read_to_string(&file_path) {
            if !contents.trim().is_empty() {
                combined.push_str(&format!("=== {filename} ===\n{contents}\n\n"));
            }
        }
    }

    if combined.is_empty() {
        return None;
    }

    if combined.len() > MAX_PROJECT_CONTEXT_BYTES {
        let truncation_point = combined[..MAX_PROJECT_CONTEXT_BYTES]
            .rfind('\n')
            .unwrap_or(MAX_PROJECT_CONTEXT_BYTES);
        combined.truncate(truncation_point);
        combined.push_str("\n\n[project context truncated]");
    }

    Some(combined)
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: String,
}

// ============================================================================
// Tool-calling versions of generation functions
// ============================================================================

/// Maximum number of tool-calling iterations to prevent infinite loops.
const MAX_TOOL_ITERATIONS: usize = 3;

/// Generate a commit message using Anthropic's API with tool-calling support.
#[allow(clippy::too_many_arguments)]
async fn generate_anthropic_with_tools(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
    repo_path: &Path,
    on_tool_call: &mut dyn FnMut(&str),
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Anthropic API key not configured. Set it in Settings > AI.")?;

    let system_prompt = build_tool_prompt(diff, summary, commit_style, project_context);
    let tools = anthropic_tool_definitions();
    let mut messages: Vec<serde_json::Value> = vec![];

    for _ in 0..MAX_TOOL_ITERATIONS {
        let json_body = serde_json::json!({
            "model": model,
            "max_tokens": 2048,
            "system": system_prompt,
            "messages": messages.clone(),
            "tools": tools,
        });

        let body_bytes = serde_json::to_vec(&json_body)?;
        let request = Request::builder()
            .method(Method::POST)
            .uri("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key.as_str())
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .body(AsyncBody::from(body_bytes))
            .context("Failed to build Anthropic request")?;

        let mut response = client
            .send(request)
            .await
            .context("Failed to send request to Anthropic API")?;

        let status = response.status();
        let body = read_response_body(&mut response).await?;

        if !status.is_success() {
            let text = String::from_utf8_lossy(&body);
            anyhow::bail!("Anthropic API error ({}): {}", status, text);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&body).context("Failed to parse Anthropic response")?;

        // Check for stop reason
        let stop_reason = json["stop_reason"].as_str().unwrap_or("");

        if stop_reason == "end_turn" {
            // Extract text content
            if let Some(content) = json["content"].as_array() {
                for block in content {
                    if block["type"].as_str() == Some("text") {
                        if let Some(text) = block["text"].as_str() {
                            return Ok(text.trim().to_string());
                        }
                    }
                }
            }
            anyhow::bail!("No text in Anthropic response");
        }

        if stop_reason == "tool_use" {
            // Extract tool calls and execute them
            let mut tool_results = vec![];
            let mut assistant_content: Vec<serde_json::Value> = vec![];

            if let Some(content) = json["content"].as_array() {
                for block in content {
                    assistant_content.push(block.clone());

                    if block["type"].as_str() == Some("tool_use") {
                        let tool_call = ToolCall {
                            id: block["id"].as_str().unwrap_or_default().to_string(),
                            name: block["name"].as_str().unwrap_or_default().to_string(),
                            arguments: block["input"].clone(),
                        };

                        on_tool_call(&AiGenerator::describe_tool_call(&tool_call));
                        let result = execute_tool(&tool_call, repo_path);
                        let result_content = match result.result {
                            Ok(output) => output,
                            Err(e) => format!("Error: {}", e),
                        };

                        tool_results.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": result.call_id,
                            "content": result_content
                        }));
                    }
                }
            }

            // Add assistant message with tool calls
            messages.push(serde_json::json!({
                "role": "assistant",
                "content": assistant_content
            }));

            // Add user message with tool results
            messages.push(serde_json::json!({
                "role": "user",
                "content": tool_results
            }));

            continue;
        }

        // Unknown stop reason - try to extract text anyway
        if let Some(content) = json["content"].as_array() {
            for block in content {
                if block["type"].as_str() == Some("text") {
                    if let Some(text) = block["text"].as_str() {
                        return Ok(text.trim().to_string());
                    }
                }
            }
        }

        anyhow::bail!("Unexpected Anthropic response: stop_reason={}", stop_reason);
    }

    anyhow::bail!("Max tool iterations reached without generating commit message")
}

/// Generate a commit message using OpenAI's API with function calling support.
#[allow(clippy::too_many_arguments)]
async fn generate_openai_with_tools(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
    repo_path: &Path,
    on_tool_call: &mut dyn FnMut(&str),
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("OpenAI API key not configured. Set it in Settings > AI.")?;

    let system_prompt = build_tool_prompt(diff, summary, commit_style, project_context);
    let tools = openai_tool_definitions();
    let mut messages: Vec<serde_json::Value> = vec![
        serde_json::json!({
            "role": "system",
            "content": system_prompt
        }),
        serde_json::json!({
            "role": "user",
            "content": "Generate a commit message for these changes."
        }),
    ];

    for _ in 0..MAX_TOOL_ITERATIONS {
        let json_body = serde_json::json!({
            "model": model,
            "messages": messages.clone(),
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.3,
            "max_tokens": 2048
        });

        let body_bytes = serde_json::to_vec(&json_body)?;
        let request = Request::builder()
            .method(Method::POST)
            .uri("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .body(AsyncBody::from(body_bytes))
            .context("Failed to build OpenAI request")?;

        let mut response = client
            .send(request)
            .await
            .context("Failed to send request to OpenAI API")?;

        let status = response.status();
        let body = read_response_body(&mut response).await?;

        if !status.is_success() {
            let text = String::from_utf8_lossy(&body);
            anyhow::bail!("OpenAI API error ({}): {}", status, text);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&body).context("Failed to parse OpenAI response")?;

        let message = &json["choices"][0]["message"];
        let finish_reason = json["choices"][0]["finish_reason"].as_str().unwrap_or("");

        // Add assistant message to history
        messages.push(message.clone());

        if finish_reason == "stop" {
            if let Some(content) = message["content"].as_str() {
                return Ok(content.trim().to_string());
            }
            anyhow::bail!("No text in OpenAI response");
        }

        if finish_reason == "tool_calls" {
            if let Some(tool_calls) = message["tool_calls"].as_array() {
                for tool_call in tool_calls {
                    let function = &tool_call["function"];
                    let tool_call_obj = ToolCall {
                        id: tool_call["id"].as_str().unwrap_or_default().to_string(),
                        name: function["name"].as_str().unwrap_or_default().to_string(),
                        arguments: serde_json::from_str(
                            function["arguments"].as_str().unwrap_or("{}"),
                        )
                        .unwrap_or(serde_json::json!({})),
                    };

                    on_tool_call(&AiGenerator::describe_tool_call(&tool_call_obj));
                    let result = execute_tool(&tool_call_obj, repo_path);
                    let result_content = match result.result {
                        Ok(output) => output,
                        Err(e) => format!("Error: {}", e),
                    };

                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": result.call_id,
                        "content": result_content
                    }));
                }
            }
            continue;
        }

        // Try to extract content anyway
        if let Some(content) = message["content"].as_str() {
            return Ok(content.trim().to_string());
        }

        anyhow::bail!(
            "Unexpected OpenAI response: finish_reason={}",
            finish_reason
        );
    }

    anyhow::bail!("Max tool iterations reached without generating commit message")
}

/// Generate a commit message using Gemini's API with function calling support.
#[allow(clippy::too_many_arguments)]
async fn generate_gemini_with_tools(
    client: &Arc<dyn HttpClient>,
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
    repo_path: &Path,
    on_tool_call: &mut dyn FnMut(&str),
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Gemini API key not configured. Set it in Settings > AI.")?;

    let prompt = build_tool_prompt(diff, summary, commit_style, project_context);
    let tools = vec![gemini_tool_definitions()];

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let mut contents = vec![serde_json::json!({
        "role": "user",
        "parts": [{ "text": prompt }]
    })];

    for _ in 0..MAX_TOOL_ITERATIONS {
        let json_body = serde_json::json!({
            "contents": contents.clone(),
            "tools": tools.clone(),
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 2048,
                "topP": 0.8
            }
        });

        let body_bytes = serde_json::to_vec(&json_body)?;
        let request = Request::builder()
            .method(Method::POST)
            .uri(&url)
            .header("Content-Type", "application/json")
            .body(AsyncBody::from(body_bytes))
            .context("Failed to build Gemini request")?;

        let mut response = client
            .send(request)
            .await
            .context("Failed to send request to Gemini API")?;

        let status = response.status();
        let body = read_response_body(&mut response).await?;

        if !status.is_success() {
            let text = String::from_utf8_lossy(&body);
            anyhow::bail!("Gemini API error ({}): {}", status, text);
        }

        let json: GeminiToolResponse =
            serde_json::from_slice(&body).context("Failed to parse Gemini response")?;

        let candidate = json
            .candidates
            .first()
            .context("No candidates in Gemini response")?;

        // Check for function call
        if let Some(part) = candidate.content.parts.first() {
            if let Some(fc) = &part.function_call {
                let tool_call = ToolCall {
                    id: format!("gemini_{}", uuid::Uuid::new_v4()),
                    name: fc.name.clone(),
                    arguments: fc.args.clone(),
                };

                on_tool_call(&AiGenerator::describe_tool_call(&tool_call));
                let result = execute_tool(&tool_call, repo_path);
                let result_content = match result.result {
                    Ok(output) => output,
                    Err(e) => format!("Error: {}", e),
                };

                // Echo the model's function call with its thoughtSignature.
                // The signature must be returned verbatim so the model retains
                // reasoning continuity across tool-call round trips.
                let sig = part.thought_signature.clone();
                let model_part: serde_json::Value = if let Some(ref s) = sig {
                    serde_json::json!({
                        "functionCall": {
                            "name": fc.name,
                            "args": fc.args
                        },
                        "thoughtSignature": s
                    })
                } else {
                    serde_json::json!({
                        "functionCall": {
                            "name": fc.name,
                            "args": fc.args
                        }
                    })
                };
                contents.push(serde_json::json!({
                    "role": "model",
                    "parts": [model_part]
                }));

                // Add function response
                contents.push(serde_json::json!({
                    "role": "function",
                    "parts": [{
                        "functionResponse": {
                            "name": fc.name,
                            "response": { "content": result_content }
                        }
                    }]
                }));

                continue;
            }

            // Text response
            if !part.text.is_empty() {
                return Ok(part.text.trim().to_string());
            }
        }

        anyhow::bail!("Unexpected Gemini response format");
    }

    anyhow::bail!("Max tool iterations reached without generating commit message")
}

/// Build a prompt that encourages tool usage when appropriate.
fn build_tool_prompt(
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
) -> String {
    let style_instruction = match commit_style {
        CommitStyle::Conventional => {
            "Use the Conventional Commits format: <type>(<scope>): <description>\n\
             Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore\n\
             Keep the first line under 72 characters.\n\
             Add a blank line then a detailed body that explains what changed and why.\n\
             List the key changes as bullet points if there are multiple distinct changes."
        }
        CommitStyle::Descriptive => {
            "Write a clear, descriptive commit message.\n\
             First line: imperative mood summary under 72 characters.\n\
             Add a blank line then a detailed body that explains what changed and why.\n\
             List the key changes as bullet points if there are multiple distinct changes."
        }
        CommitStyle::Brief => {
            "Write a concise commit message in imperative mood.\n\
             Keep it to a single line under 72 characters."
        }
    };

    let max_diff_len = 200_000;
    let diff_text = if diff.len() > max_diff_len {
        let truncation_point = diff[..max_diff_len].rfind('\n').unwrap_or(max_diff_len);
        format!(
            "{}\n\n[diff truncated -- showing {}/{} bytes]",
            &diff[..truncation_point],
            truncation_point,
            diff.len()
        )
    } else {
        diff.to_string()
    };

    let context_section = match project_context {
        Some(context) => format!("Project Context:\n{context}\n\n"),
        None => String::new(),
    };

    format!(
        "You are a Git commit message generator. Generate ONLY the commit message, nothing else.\n\
         No markdown formatting, no code blocks, no explanations.\n\n\
         {style_instruction}\n\n\
         You have access to tools to get more context about the repository. Use them if you need to:\n\
         - Understand what a changed file does (get_file_content)\n\
         - See the commit message style used in this project (get_recent_commits)\n\
         - Understand how a file has evolved (get_file_history)\n\n\
         Only use tools if the diff is unclear and you need more context. If the changes are self-explanatory, generate the commit message directly.\n\n\
         {context_section}\
         Files changed:\n{summary}\n\n\
         Diff:\n{diff_text}"
    )
}

/// Gemini response with potential function calls.
#[derive(Debug, Deserialize)]
struct GeminiToolResponse {
    candidates: Vec<GeminiToolCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiToolCandidate {
    content: GeminiToolContent,
}

#[derive(Debug, Deserialize)]
struct GeminiToolContent {
    parts: Vec<GeminiToolPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiToolPart {
    #[serde(default)]
    text: String,
    #[serde(rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
    /// Opaque signature of the model's reasoning about this function call.
    /// Must be echoed back verbatim in the next request so the model retains
    /// reasoning continuity across tool-call round trips.
    #[serde(default, rename = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}
