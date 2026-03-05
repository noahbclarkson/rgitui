use anyhow::{Context as _, Result};
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use rgitui_settings::SettingsState;
use serde::Deserialize;
use std::time::{Duration, Instant};

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

impl CommitStyle {
    pub fn from_str(s: &str) -> Self {
        match s {
            "conventional" => Self::Conventional,
            "brief" => Self::Brief,
            _ => Self::Descriptive,
        }
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

        let settings = cx.global::<SettingsState>().settings().clone();
        let api_key = settings.ai.api_key.clone();
        let model = settings.ai.model.clone();
        let provider = settings.ai.provider.clone();
        let commit_style = CommitStyle::from_str(&settings.ai.commit_style);

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = match provider.as_str() {
                "gemini" => {
                    generate_gemini(&api_key, &model, &diff, &summary, commit_style).await
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
            "maxOutputTokens": 512,
            "topP": 0.8
        }
    });

    let client = reqwest::Client::new();
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

fn build_prompt(diff: &str, summary: &str, commit_style: CommitStyle) -> String {
    let style_instruction = match commit_style {
        CommitStyle::Conventional => {
            "Use the Conventional Commits format: <type>(<scope>): <description>\n\
             Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore\n\
             Keep the first line under 72 characters.\n\
             Add a blank line then a brief body if needed (2-3 lines max)."
        }
        CommitStyle::Descriptive => {
            "Write a clear, descriptive commit message.\n\
             First line: imperative mood summary under 72 characters.\n\
             Add a blank line then a brief body if needed (2-3 lines max)."
        }
        CommitStyle::Brief => {
            "Write a concise commit message in imperative mood.\n\
             Keep it to a single line under 72 characters."
        }
    };

    // Truncate diff if too large
    let max_diff_len = 8000;
    let diff_text = if diff.len() > max_diff_len {
        format!("{}...\n[diff truncated]", &diff[..max_diff_len])
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
