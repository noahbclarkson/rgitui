use anyhow::{Context as _, Result};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use rgitui_settings::SettingsState;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static TOKIO_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn tokio_handle() -> tokio::runtime::Handle {
    TOKIO_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime for HTTP requests")
        })
        .handle()
        .clone()
}

fn http_client() -> reqwest::Client {
    let _guard = tokio_handle().enter();
    reqwest::Client::new()
}

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
        cx: &mut Context<Self>,
    ) -> Task<Result<String>> {
        // Rate limiting: check if enough time has passed since last request
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
        let commit_style = settings.ai.commit_style.parse::<CommitStyle>().unwrap_or_default();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let handle = tokio_handle();
            let result = handle.spawn(async move {
                match provider.as_str() {
                    "gemini" => generate_gemini(&api_key, &model, &diff, &summary, commit_style).await,
                    "openai" => generate_openai(&api_key, &model, &diff, &summary, commit_style).await,
                    "anthropic" => generate_anthropic(&api_key, &model, &diff, &summary, commit_style).await,
                    other => Err(anyhow::anyhow!("Unknown AI provider: {}", other)),
                }
            }).await.context("AI task panicked")?;

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

/// Generate a commit message using the Gemini API.
async fn generate_gemini(
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Gemini API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style);

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let body = serde_json::json!({
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

    let client = http_client();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Failed to send request to Gemini API")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Gemini API error ({}): {}", status, text);
    }

    let json: GeminiResponse = response
        .json()
        .await
        .context("Failed to parse Gemini response")?;

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
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("OpenAI API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style);

    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": prompt
        }],
        "temperature": 0.3,
        "max_tokens": 2048
    });

    let client = http_client();
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .context("Failed to send request to OpenAI API")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI API error ({}): {}", status, text);
    }

    let json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse OpenAI response")?;
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .context("No text in OpenAI response")?
        .trim()
        .to_string();

    Ok(text)
}

/// Generate a commit message using the Anthropic API.
async fn generate_anthropic(
    api_key: &Option<String>,
    model: &str,
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
) -> Result<String> {
    let api_key = api_key
        .as_ref()
        .context("Anthropic API key not configured. Set it in Settings > AI.")?;

    let prompt = build_prompt(diff, summary, commit_style);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [{
            "role": "user",
            "content": prompt
        }]
    });

    let client = http_client();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send request to Anthropic API")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API error ({}): {}", status, text);
    }

    let json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse Anthropic response")?;
    let text = json["content"][0]["text"]
        .as_str()
        .context("No text in Anthropic response")?
        .trim()
        .to_string();

    Ok(text)
}

fn build_prompt(diff: &str, summary: &str, commit_style: CommitStyle) -> String {
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
        let truncation_point = diff[..max_diff_len]
            .rfind('\n')
            .unwrap_or(max_diff_len);
        format!(
            "{}\n\n[diff truncated — showing {}/{} bytes]",
            &diff[..truncation_point],
            truncation_point,
            diff.len()
        )
    } else {
        diff.to_string()
    };

    format!(
        "You are a Git commit message generator. Generate ONLY the commit message, nothing else.\n\
         No markdown formatting, no code blocks, no explanations.\n\n\
         {style_instruction}\n\n\
         Files changed:\n{summary}\n\n\
         Diff:\n{diff_text}"
    )
}

// Gemini API response types

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
