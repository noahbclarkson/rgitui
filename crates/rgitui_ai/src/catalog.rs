//! Live, auto-updating model catalogue.
//!
//! Replaces the hardcoded model list that used to live inside a render
//! function in the settings view, where it could not be extended by the user,
//! could not be tested, and went stale silently.
//!
//! Everything that touches the network or the filesystem is an `async fn` or a
//! plain function taking explicit inputs, and every decision the picker makes
//! — filtering, ranking, freshness, pinned-model classification — is a pure
//! function with tests, per the convention in `CLAUDE.md`.
//!
//! Threading: [`fetch_models`] runs the round-trip *and* the JSON parse, so
//! callers must invoke it from `cx.background_executor().spawn(...)`. The
//! unfiltered OpenRouter payload is roughly 700 KB and must never be
//! deserialised on the render thread.

pub mod static_catalog;

use anyhow::{Context as _, Result};
use gpui::http_client::{AsyncBody, HttpClient, HttpRequestExt, Method, Request};
use rgitui_settings::{cache_dir, AiProvider};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use static_catalog::{bundled_catalog, enrich_from_static};

use crate::http::{read_response_body, CATALOG_REQUEST_TIMEOUT};

/// Whether a model can be given tools.
///
/// `Unknown` is load-bearing: OpenAI and DeepSeek genuinely do not report it,
/// and rendering that honestly beats a confidently wrong badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolSupport {
    Supported,
    Unsupported,
    Unknown,
}

impl ToolSupport {
    /// The badge text for the picker, or `None` when nothing honest can be
    /// said.
    pub fn badge(self) -> Option<&'static str> {
        match self {
            ToolSupport::Supported => Some("Tools"),
            ToolSupport::Unsupported => Some("No tools"),
            ToolSupport::Unknown => None,
        }
    }
}

/// One selectable model. Every field is something the picker renders or
/// filters on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The exact string sent as the request's `model` field.
    pub id: String,
    pub display_name: String,
    pub context_length: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// USD per million tokens. Only OpenRouter reports pricing; the rest leave
    /// these `None` and the picker renders a blank column, which reads as
    /// "unknown" correctly.
    pub prompt_price_per_mtok: Option<f64>,
    pub completion_price_per_mtok: Option<f64>,
    pub tool_support: ToolSupport,
    /// Emits text, as opposed to image/audio/embeddings.
    pub emits_text: bool,
    /// A `:free`/`:batch` variant or a `…-latest` alias rather than a distinct
    /// model. Hidden by default.
    pub is_variant: bool,
    pub created: Option<i64>,
}

impl ModelInfo {
    /// The one-line summary shown under the model field:
    /// `1M ctx · $0.25/$1.50 per Mtok · Tools`.
    pub fn summary_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(ctx) = self.context_length {
            parts.push(format!("{} ctx", format_context(ctx)));
        }
        if let (Some(prompt), Some(completion)) =
            (self.prompt_price_per_mtok, self.completion_price_per_mtok)
        {
            parts.push(format!(
                "${:.2}/${:.2} per Mtok",
                prompt.max(0.0),
                completion.max(0.0)
            ));
        }
        if let Some(badge) = self.tool_support.badge() {
            parts.push(badge.to_string());
        }
        parts.join(" · ")
    }

    /// The compact right-hand column of a picker row: `1M   $0.25/$1.50`.
    pub fn trailing_label(&self) -> String {
        let ctx = self.context_length.map(format_context).unwrap_or_default();
        match (self.prompt_price_per_mtok, self.completion_price_per_mtok) {
            (Some(prompt), Some(completion)) => format!(
                "{ctx}   ${:.2}/${:.2}",
                prompt.max(0.0),
                completion.max(0.0)
            )
            .trim_start()
            .to_string(),
            _ => ctx,
        }
    }

    pub fn is_free(&self) -> bool {
        self.prompt_price_per_mtok == Some(0.0)
    }
}

/// Render a context window the way the provider docs do: `1M`, `128K`, `4096`.
pub fn format_context(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_000_000.0;
        // A round million reads as `1M`; anything else keeps two decimals, so
        // 1,048,576 renders as `1.05M` rather than being rounded away.
        if (millions - millions.round()).abs() < 0.005 {
            format!("{}M", millions.round() as u64)
        } else {
            format!("{millions:.2}M")
        }
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

// ============================================================================
// Cache envelope
// ============================================================================

/// Bump to invalidate stale cache files after a [`ModelInfo`] shape change.
pub const CATALOG_SCHEMA: u32 = 1;
const CATALOG_TTL_SECS: i64 = 24 * 60 * 60;
const CATALOG_STALE_SECS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCatalog {
    pub schema: u32,
    /// Unix seconds.
    pub fetched_at: i64,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogFreshness {
    Fresh,
    Stale,
    Expired,
}

/// Pure — no clock, no filesystem, so the boundaries are directly testable.
pub fn freshness(fetched_at: i64, now: i64) -> CatalogFreshness {
    let age = now.saturating_sub(fetched_at);
    // A timestamp from the future means the clock moved backwards; treat it as
    // fresh rather than re-fetching in a loop.
    if age < 0 || age < CATALOG_TTL_SECS {
        CatalogFreshness::Fresh
    } else if age < CATALOG_STALE_SECS {
        CatalogFreshness::Stale
    } else {
        CatalogFreshness::Expired
    }
}

/// Where a rendered catalogue came from. The picker labels it so the user can
/// tell "three weeks old" from "shipped with the app".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    Live,
    Cache { fetched_at: i64 },
    Bundled,
}

/// Live > cache > bundled.
pub fn resolve_catalog(
    provider: AiProvider,
    cached: Option<CachedCatalog>,
) -> (Vec<ModelInfo>, CatalogSource) {
    match cached {
        Some(catalog) if catalog.schema == CATALOG_SCHEMA && !catalog.models.is_empty() => {
            let fetched_at = catalog.fetched_at;
            (catalog.models, CatalogSource::Cache { fetched_at })
        }
        _ => (bundled_catalog(provider), CatalogSource::Bundled),
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn catalog_path(provider: AiProvider) -> PathBuf {
    cache_dir()
        .join("models")
        .join(format!("{}.json", provider.id()))
}

/// Read a provider's cached catalogue. A few KB of JSON — cheap enough to read
/// synchronously when the picker opens.
pub fn read_cached(provider: AiProvider) -> Option<CachedCatalog> {
    let json = std::fs::read_to_string(catalog_path(provider)).ok()?;
    let catalog: CachedCatalog = serde_json::from_str(&json).ok()?;
    (catalog.schema == CATALOG_SCHEMA).then_some(catalog)
}

/// Write a provider's catalogue, temp-file-then-rename so a crash mid-write
/// cannot leave a truncated file that then fails to parse forever.
pub fn write_cached(provider: AiProvider, catalog: &CachedCatalog) -> Result<()> {
    let path = catalog_path(provider);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(catalog)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    if std::fs::rename(&tmp, &path).is_err() {
        std::fs::write(&path, &json)?;
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(())
}

// ============================================================================
// Fetching
// ============================================================================

/// Whether fetching this provider's catalogue requires the user's API key.
/// Only OpenRouter's is public.
pub fn catalog_needs_key(provider: AiProvider) -> bool {
    !matches!(provider, AiProvider::OpenRouter)
}

/// The recommended OpenRouter query: tool-capable text models sorted by coding
/// ability, which is exactly the axis that matters for a commit-message
/// generator. 145 KB rather than the 697 KB full dump.
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models\
     ?supported_parameters=tools&output_modalities=text&sort=coding-high-to-low&limit=60";

/// The unfiltered OpenRouter catalogue, behind "load all" in the picker.
const OPENROUTER_ALL_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// Fetch a provider's model list. Runs the round-trip **and** the parse — call
/// it from a background task.
pub async fn fetch_models(
    provider: AiProvider,
    client: &Arc<dyn HttpClient>,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    fetch_models_inner(provider, client, api_key, false).await
}

/// As [`fetch_models`], but asks OpenRouter for its entire catalogue rather
/// than the recommended 60-model slice.
pub async fn fetch_all_models(
    provider: AiProvider,
    client: &Arc<dyn HttpClient>,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    fetch_models_inner(provider, client, api_key, true).await
}

async fn fetch_models_inner(
    provider: AiProvider,
    client: &Arc<dyn HttpClient>,
    api_key: Option<&str>,
    load_all: bool,
) -> Result<Vec<ModelInfo>> {
    if catalog_needs_key(provider) && api_key.map(str::trim).unwrap_or("").is_empty() {
        anyhow::bail!(
            "{} needs an API key before its model list can be loaded.",
            provider.display_name()
        );
    }

    let (url, mut builder) = match provider {
        AiProvider::OpenRouter => {
            let url = if load_all {
                OPENROUTER_ALL_MODELS_URL
            } else {
                OPENROUTER_MODELS_URL
            };
            (url.to_string(), Request::builder())
        }
        AiProvider::Anthropic => (
            // The default limit is 20; without this the list renders silently
            // truncated.
            "https://api.anthropic.com/v1/models?limit=100".to_string(),
            Request::builder().header("anthropic-version", crate::provider::ANTHROPIC_VERSION),
        ),
        AiProvider::Gemini => (
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200".to_string(),
            Request::builder(),
        ),
        AiProvider::OpenAi => (
            "https://api.openai.com/v1/models".to_string(),
            Request::builder(),
        ),
        AiProvider::DeepSeek => (
            "https://api.deepseek.com/models".to_string(),
            Request::builder(),
        ),
    };

    if let Some(key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
        builder = match crate::provider::auth_style(provider) {
            crate::provider::AuthStyle::Bearer => {
                builder.header("Authorization", format!("Bearer {key}"))
            }
            crate::provider::AuthStyle::AnthropicHeader => builder.header("x-api-key", key),
            crate::provider::AuthStyle::GoogleHeader => builder.header("x-goog-api-key", key),
        };
    }

    let request = builder
        .method(Method::GET)
        .uri(&url)
        .timeout(CATALOG_REQUEST_TIMEOUT)
        .body(AsyncBody::from(Vec::new()))
        .with_context(|| format!("Failed to build the {} model-list request", provider))?;

    let mut response = client
        .send(request)
        .await
        .with_context(|| format!("Couldn't reach {}", provider.default_host()))?;

    let status = response.status();
    let body = read_response_body(&mut response).await?;
    if !status.is_success() {
        anyhow::bail!(
            "{} rejected the model-list request ({}).",
            provider.display_name(),
            status
        );
    }

    let json: serde_json::Value = serde_json::from_slice(&body)
        .with_context(|| format!("Couldn't read {}'s model list", provider.display_name()))?;

    let mut models = parse_models(provider, &json);
    enrich_from_static(provider, &mut models);
    if models.is_empty() {
        anyhow::bail!("{} returned no usable models.", provider.display_name());
    }
    Ok(models)
}

/// Parse a provider's `/models` payload. Pure, so the real captured fixtures
/// in the test module exercise exactly what the network path does.
pub fn parse_models(provider: AiProvider, json: &serde_json::Value) -> Vec<ModelInfo> {
    match provider {
        AiProvider::OpenRouter => parse_openrouter(json),
        AiProvider::Anthropic => parse_anthropic(json),
        AiProvider::Gemini => parse_gemini(json),
        AiProvider::OpenAi => parse_openai(json),
        AiProvider::DeepSeek => parse_deepseek(json),
    }
}

/// OpenRouter reports `pricing.*` as USD-per-token, encoded as JSON *strings*.
/// Five models report negative prices (BYOK rebate rows), so nothing here may
/// assume a non-negative value.
fn price_per_mtok(value: Option<&serde_json::Value>) -> Option<f64> {
    let raw = value?.as_str()?;
    let per_token: f64 = raw.parse().ok()?;
    Some(per_token * 1_000_000.0)
}

/// Whether an OpenRouter slug names a variant rather than a distinct model.
pub fn is_openrouter_variant(id: &str, alias_target: Option<&str>) -> bool {
    alias_target.is_some()
        || id.contains(":free")
        || id.contains(":batch")
        || id.contains(":extended")
        || id.ends_with("-latest")
}

fn parse_openrouter(json: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = json["data"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_string();
            let supported: Vec<&str> = item["supported_parameters"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let output_modalities: Vec<&str> = item["architecture"]["output_modalities"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let emits_text = output_modalities.is_empty() || output_modalities.contains(&"text");

            let context_length = item["context_length"]
                .as_u64()
                .or_else(|| item["top_provider"]["context_length"].as_u64())
                .map(|value| value as u32);

            Some(ModelInfo {
                display_name: item["name"].as_str().unwrap_or(&id).to_string(),
                context_length,
                max_output_tokens: item["top_provider"]["max_completion_tokens"]
                    .as_u64()
                    .map(|value| value as u32),
                prompt_price_per_mtok: price_per_mtok(item["pricing"].get("prompt")),
                completion_price_per_mtok: price_per_mtok(item["pricing"].get("completion")),
                tool_support: if supported.contains(&"tools") {
                    ToolSupport::Supported
                } else {
                    ToolSupport::Unsupported
                },
                emits_text,
                is_variant: is_openrouter_variant(&id, item["alias_target"].as_str()),
                created: item["created"].as_i64(),
                id,
            })
        })
        .collect()
}

fn parse_anthropic(json: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = json["data"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_string();
            let supports_tools = item["capabilities"]["tool_use"].as_bool();
            Some(ModelInfo {
                display_name: item["display_name"].as_str().unwrap_or(&id).to_string(),
                context_length: item["max_input_tokens"].as_u64().map(|v| v as u32),
                max_output_tokens: item["max_tokens"]
                    .as_u64()
                    .or_else(|| item["max_output_tokens"].as_u64())
                    .map(|v| v as u32),
                prompt_price_per_mtok: None,
                completion_price_per_mtok: None,
                tool_support: match supports_tools {
                    Some(true) => ToolSupport::Supported,
                    Some(false) => ToolSupport::Unsupported,
                    // Every current Claude model takes tools, but say so only
                    // when the API says so.
                    None => ToolSupport::Unknown,
                },
                emits_text: true,
                is_variant: false,
                created: None,
                id,
            })
        })
        .collect()
}

fn parse_gemini(json: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = json["models"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            // `models/gemini-3.1-flash-lite` -> `gemini-3.1-flash-lite`.
            let raw = item["name"].as_str()?;
            let id = raw.strip_prefix("models/").unwrap_or(raw).to_string();

            // Gemini has no function-calling flag, so filter on generation
            // methods instead: this is what drops embedding and TTS models.
            let methods: Vec<&str> = item["supportedGenerationMethods"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if !methods.is_empty() && !methods.contains(&"generateContent") {
                return None;
            }

            Some(ModelInfo {
                display_name: item["displayName"].as_str().unwrap_or(&id).to_string(),
                context_length: item["inputTokenLimit"].as_u64().map(|v| v as u32),
                max_output_tokens: item["outputTokenLimit"].as_u64().map(|v| v as u32),
                prompt_price_per_mtok: None,
                completion_price_per_mtok: None,
                // Not advertised — say nothing rather than guess.
                tool_support: ToolSupport::Unknown,
                emits_text: true,
                is_variant: id.ends_with("-latest"),
                created: None,
                id,
            })
        })
        .collect()
}

/// OpenAI's `/models` returns embeddings, TTS, whisper, image and fine-tune
/// models alongside chat models, with no field distinguishing them.
const OPENAI_NON_CHAT_PREFIXES: &[&str] = &[
    "text-embedding-",
    "dall-e",
    "whisper-",
    "tts-",
    "omni-moderation-",
    "text-moderation-",
    "gpt-image-",
    "sora-",
    "babbage-",
    "davinci-",
    "codex-mini",
];

fn parse_openai(json: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = json["data"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_string();
            if OPENAI_NON_CHAT_PREFIXES
                .iter()
                .any(|prefix| id.starts_with(prefix))
            {
                return None;
            }
            // The one gift of this endpoint: it flags models with a scheduled
            // shutdown, which is exactly the set that produces an opaque 404
            // at generation time.
            if item["shutdown_date"].is_string() {
                return None;
            }
            // The o-series 400s against the body this app builds — it needs
            // `max_completion_tokens` and rejects a non-default temperature —
            // so offering it would be offering a guaranteed failure.
            if is_openai_o_series(&id) {
                return None;
            }
            Some(ModelInfo {
                display_name: id.clone(),
                context_length: None,
                max_output_tokens: None,
                prompt_price_per_mtok: None,
                completion_price_per_mtok: None,
                tool_support: ToolSupport::Unknown,
                emits_text: true,
                is_variant: id.ends_with("-latest"),
                created: item["created"].as_i64(),
                id,
            })
        })
        .collect()
}

/// `o1`, `o3`, `o4-mini`, … — but not `openai/…` or anything else that merely
/// starts with the letter.
pub fn is_openai_o_series(id: &str) -> bool {
    let mut chars = id.chars();
    if chars.next() != Some('o') {
        return false;
    }
    let rest: String = chars.collect();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return false;
    }
    let tail = &rest[digits.len()..];
    tail.is_empty() || tail.starts_with('-')
}

fn parse_deepseek(json: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = json["data"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_string();
            Some(ModelInfo {
                display_name: id.clone(),
                context_length: None,
                max_output_tokens: None,
                prompt_price_per_mtok: None,
                completion_price_per_mtok: None,
                tool_support: ToolSupport::Unknown,
                emits_text: true,
                is_variant: false,
                created: None,
                id,
            })
        })
        .collect()
}

// ============================================================================
// Filtering and ranking
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFilter {
    /// Defaults to mirroring `settings.ai.use_tools`.
    pub tools_only: bool,
    pub text_only: bool,
    pub free_only: bool,
    pub show_variants: bool,
}

impl Default for ModelFilter {
    fn default() -> Self {
        Self {
            tools_only: false,
            text_only: true,
            free_only: false,
            show_variants: false,
        }
    }
}

/// Filter a catalogue down to what the picker should offer. Pure, no GPUI.
///
/// Ordering is preserved, because the server's own order is already the most
/// useful one there is — OpenRouter's is `coding-high-to-low`, which is
/// exactly the axis that matters for a commit-message generator. Ranking a
/// typed query is the picker's job, and it uses the app's single shared
/// `fuzzy_score` so the two cannot rank the same query differently.
pub fn filter_models(models: &[ModelInfo], filter: ModelFilter) -> Vec<&ModelInfo> {
    models
        .iter()
        .filter(|model| !filter.text_only || model.emits_text)
        // `Unknown` survives `tools_only` deliberately. Dropping it would empty
        // the Gemini and OpenAI lists entirely, since neither advertises tool
        // support.
        .filter(|model| !filter.tools_only || model.tool_support != ToolSupport::Unsupported)
        .filter(|model| !filter.free_only || model.is_free())
        .filter(|model| filter.show_variants || !model.is_variant)
        .collect()
}

// ============================================================================
// Pinned-model validation
// ============================================================================

/// What the catalogue can say about the model the user has pinned.
///
/// `settings.ai.model` is never auto-rewritten from any of these: it is user
/// intent, and silently retargeting it is how someone ends up billed for a
/// model they did not choose.
#[derive(Debug, Clone, PartialEq)]
pub enum PinnedModelStatus {
    Known(Box<ModelInfo>),
    Missing {
        suggestion: Option<String>,
    },
    /// In the catalogue but in conflict with current settings.
    Incompatible {
        reason: String,
    },
    /// No catalogue to check against — offline, no key, or bundled-only.
    Unverified,
}

/// Classify a pinned model. Pure; `use_tools` comes from settings so the
/// conflict surfaces here rather than as an opaque 404 at request time.
pub fn classify_pinned(
    pinned: &str,
    catalog: &[ModelInfo],
    source: CatalogSource,
    use_tools: bool,
) -> PinnedModelStatus {
    let pinned = pinned.trim();
    if pinned.is_empty() {
        return PinnedModelStatus::Missing { suggestion: None };
    }

    if let Some(model) = catalog.iter().find(|model| model.id == pinned) {
        if use_tools && model.tool_support == ToolSupport::Unsupported {
            return PinnedModelStatus::Incompatible {
                reason: format!(
                    "`{}` does not support tool calling. Turn off \"Let the model read files\" \
                     or choose a model marked Tools.",
                    model.id
                ),
            };
        }
        return PinnedModelStatus::Known(Box::new(model.clone()));
    }

    // A bundled or absent catalogue is not evidence the model is gone, and
    // crying wolf offline is worse than staying quiet.
    if matches!(source, CatalogSource::Bundled) || catalog.is_empty() {
        return PinnedModelStatus::Unverified;
    }

    PinnedModelStatus::Missing {
        suggestion: closest_model_id(pinned, catalog),
    }
}

/// Minimum shared prefix before a "did you mean" is offered. Below this the
/// two ids are not the same model family and a suggestion would be noise.
const MIN_SUGGESTION_PREFIX: usize = 4;

/// The catalogue id most likely to be what a retired pin was replaced by.
///
/// Ranked by shared leading characters, not by [`fuzzy_score`]: fuzzy
/// subsequence matching answers "does the user's typing appear in this id",
/// which is right for a search box and wrong here. `openai/gpt-4o-mini`
/// contains a `4` that no successor id has, so fuzzy matching finds nothing at
/// all — while the shared `openai/gpt-` prefix is exactly the signal wanted.
pub fn closest_model_id(pinned: &str, catalog: &[ModelInfo]) -> Option<String> {
    let pinned = pinned.trim().to_ascii_lowercase();
    catalog
        .iter()
        .map(|model| {
            (
                common_prefix_len(&pinned, &model.id.to_ascii_lowercase()),
                model,
            )
        })
        .filter(|(shared, _)| *shared >= MIN_SUGGESTION_PREFIX)
        // Longest shared prefix wins; ties break toward the shorter id, which
        // is the plain model rather than a dated snapshot of it.
        .min_by_key(|(shared, model)| (usize::MAX - shared, model.id.len()))
        .map(|(_, model)| model.id.clone())
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// How long a catalogue fetch may take before it is abandoned.
pub const CATALOG_TIMEOUT: Duration = CATALOG_REQUEST_TIMEOUT;

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            context_length: Some(128_000),
            max_output_tokens: Some(4096),
            prompt_price_per_mtok: None,
            completion_price_per_mtok: None,
            tool_support: ToolSupport::Supported,
            emits_text: true,
            is_variant: false,
            created: None,
        }
    }

    // ── OpenRouter fixture ────────────────────────────────────────
    //
    // Trimmed from a real response, keeping exactly the records that broke
    // naive derives: a negative-price BYOK rebate row, a null
    // `top_provider.context_length`, a record missing `reasoning` and
    // `benchmarks` entirely, an alias row, and a non-text model.

    const OPENROUTER_FIXTURE: &str = r#"{
      "data": [
        {
          "id": "google/gemini-3.1-flash-lite",
          "canonical_slug": "google/gemini-3.1-flash-lite",
          "name": "Google: Gemini 3.1 Flash Lite",
          "created": 1771200000,
          "description": "Fast and cheap.",
          "context_length": 1048576,
          "architecture": { "output_modalities": ["text"], "input_modalities": ["text"] },
          "pricing": { "prompt": "0.00000025", "completion": "0.0000015" },
          "top_provider": { "context_length": 1048576, "max_completion_tokens": 65536 },
          "supported_parameters": ["tools", "temperature", "max_tokens"],
          "reasoning": { "supported": true }
        },
        {
          "id": "someone/byok-rebate-model",
          "name": "BYOK Rebate",
          "created": 1770000000,
          "context_length": 200000,
          "architecture": { "output_modalities": ["text"] },
          "pricing": { "prompt": "-0.0000001", "completion": "-0.0000002" },
          "top_provider": { "context_length": null, "max_completion_tokens": null },
          "supported_parameters": ["tools"]
        },
        {
          "id": "openai/gpt-5.6-luna:free",
          "name": "GPT-5.6 Luna (free)",
          "created": 1772000000,
          "context_length": 1047576,
          "architecture": { "output_modalities": ["text"] },
          "pricing": { "prompt": "0", "completion": "0" },
          "top_provider": { "context_length": 1047576, "max_completion_tokens": 32768 },
          "supported_parameters": ["tools"]
        },
        {
          "id": "vendor/aliased-model",
          "name": "Aliased",
          "created": 1769000000,
          "context_length": 8192,
          "architecture": { "output_modalities": ["text"] },
          "pricing": { "prompt": "0.000001", "completion": "0.000002" },
          "top_provider": { "context_length": 8192, "max_completion_tokens": 4096 },
          "supported_parameters": ["tools"],
          "alias_target": "vendor/real-model"
        },
        {
          "id": "vendor/image-model",
          "name": "Image Only",
          "created": 1768000000,
          "context_length": 4096,
          "architecture": { "output_modalities": ["image"] },
          "pricing": { "prompt": "0.00001", "completion": "0.00002" },
          "top_provider": { "context_length": 4096, "max_completion_tokens": 1024 },
          "supported_parameters": ["temperature"]
        }
      ]
    }"#;

    fn openrouter_models() -> Vec<ModelInfo> {
        parse_models(
            AiProvider::OpenRouter,
            &serde_json::from_str(OPENROUTER_FIXTURE).unwrap(),
        )
    }

    #[test]
    fn openrouter_fixture_parses_every_record() {
        assert_eq!(openrouter_models().len(), 5);
    }

    #[test]
    fn openrouter_prices_convert_from_per_token_strings_to_per_mtok() {
        let models = openrouter_models();
        let flash = &models[0];
        assert_eq!(flash.prompt_price_per_mtok, Some(0.25));
        assert_eq!(flash.completion_price_per_mtok, Some(1.5));
    }

    /// Five real models report negative prices (BYOK rebate rows), so nothing
    /// may assume `>= 0`.
    #[test]
    fn openrouter_negative_prices_are_preserved_not_clamped_away() {
        let models = openrouter_models();
        let rebate = models.iter().find(|m| m.id.contains("byok")).unwrap();
        let price = rebate.prompt_price_per_mtok.unwrap();
        assert!(price < 0.0, "a negative price must survive parsing");
        assert!((price - -0.1).abs() < 1e-9, "got {price}");
    }

    #[test]
    fn a_null_top_provider_context_falls_back_to_the_top_level_field() {
        let models = openrouter_models();
        let rebate = models.iter().find(|m| m.id.contains("byok")).unwrap();
        assert_eq!(rebate.context_length, Some(200_000));
        assert_eq!(rebate.max_output_tokens, None);
    }

    #[test]
    fn a_record_missing_optional_objects_still_parses() {
        // The BYOK row carries no `reasoning` and no `benchmarks`; 181 of 421
        // real records are missing at least one.
        assert!(openrouter_models().iter().any(|m| m.id.contains("byok")));
    }

    #[test]
    fn free_and_alias_rows_are_marked_as_variants() {
        let models = openrouter_models();
        assert!(
            models
                .iter()
                .find(|m| m.id.ends_with(":free"))
                .unwrap()
                .is_variant
        );
        assert!(
            models
                .iter()
                .find(|m| m.id.contains("aliased"))
                .unwrap()
                .is_variant
        );
        assert!(!models[0].is_variant);
    }

    #[test]
    fn variant_detection_covers_every_documented_form() {
        assert!(is_openrouter_variant("a/b:free", None));
        assert!(is_openrouter_variant("a/b:batch", None));
        assert!(is_openrouter_variant("a/b-latest", None));
        assert!(is_openrouter_variant("a/b", Some("a/c")));
        assert!(!is_openrouter_variant("a/b", None));
    }

    #[test]
    fn a_non_text_model_is_flagged_and_filtered_out_by_default() {
        let models = openrouter_models();
        let image = models.iter().find(|m| m.id.contains("image")).unwrap();
        assert!(!image.emits_text);
        let kept = filter_models(&models, ModelFilter::default());
        assert!(!kept.iter().any(|m| m.id.contains("image")));
    }

    // ── other providers ───────────────────────────────────────────

    #[test]
    fn anthropic_parses_display_name_and_limits() {
        let json = serde_json::json!({
            "data": [{
                "id": "claude-haiku-4-5",
                "display_name": "Claude Haiku 4.5",
                "max_input_tokens": 200000,
                "max_tokens": 64000,
                "capabilities": { "tool_use": true }
            }]
        });
        let models = parse_models(AiProvider::Anthropic, &json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].display_name, "Claude Haiku 4.5");
        assert_eq!(models[0].context_length, Some(200_000));
        assert_eq!(models[0].tool_support, ToolSupport::Supported);
    }

    #[test]
    fn gemini_strips_the_models_prefix_and_drops_non_generative_models() {
        let json = serde_json::json!({
            "models": [
                {
                    "name": "models/gemini-3.1-flash-lite",
                    "displayName": "Gemini 3.1 Flash Lite",
                    "inputTokenLimit": 1048576,
                    "outputTokenLimit": 65536,
                    "supportedGenerationMethods": ["generateContent", "countTokens"]
                },
                {
                    "name": "models/text-embedding-004",
                    "displayName": "Embedding 004",
                    "supportedGenerationMethods": ["embedContent"]
                }
            ]
        });
        let models = parse_models(AiProvider::Gemini, &json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-3.1-flash-lite");
        // Gemini advertises no tool flag; say nothing rather than guess.
        assert_eq!(models[0].tool_support, ToolSupport::Unknown);
    }

    #[test]
    fn openai_drops_non_chat_models_and_anything_with_a_shutdown_date() {
        let json = serde_json::json!({
            "data": [
                { "id": "gpt-5.6-luna", "created": 1 },
                { "id": "text-embedding-3-small", "created": 2 },
                { "id": "whisper-1", "created": 3 },
                { "id": "gpt-4o-mini", "created": 4, "shutdown_date": "2026-12-11" },
                { "id": "o4-mini", "created": 5 }
            ]
        });
        let ids: Vec<String> = parse_models(AiProvider::OpenAi, &json)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["gpt-5.6-luna".to_string()]);
    }

    #[test]
    fn o_series_detection_does_not_catch_ordinary_names() {
        assert!(is_openai_o_series("o3"));
        assert!(is_openai_o_series("o4-mini"));
        assert!(is_openai_o_series("o1-preview"));
        assert!(!is_openai_o_series("openai/gpt-5"));
        assert!(!is_openai_o_series("omni-moderation-latest"));
        assert!(!is_openai_o_series("gpt-5"));
    }

    #[test]
    fn a_payload_with_no_data_array_yields_an_empty_list_not_a_panic() {
        for provider in AiProvider::ALL {
            assert!(parse_models(*provider, &serde_json::json!({ "error": "nope" })).is_empty());
        }
    }

    // ── filtering ─────────────────────────────────────────────────

    fn filter_fixture() -> Vec<ModelInfo> {
        let mut supported = model("vendor/tools-model");
        supported.tool_support = ToolSupport::Supported;
        let mut unsupported = model("vendor/no-tools-model");
        unsupported.tool_support = ToolSupport::Unsupported;
        let mut unknown = model("vendor/unknown-tools-model");
        unknown.tool_support = ToolSupport::Unknown;
        let mut free = model("vendor/free-model:free");
        free.prompt_price_per_mtok = Some(0.0);
        free.completion_price_per_mtok = Some(0.0);
        free.is_variant = true;
        let mut image = model("vendor/image-model");
        image.emits_text = false;
        vec![supported, unsupported, unknown, free, image]
    }

    #[test]
    fn tools_only_keeps_unknown_so_gemini_and_openai_lists_do_not_empty() {
        let models = filter_fixture();
        let filter = ModelFilter {
            tools_only: true,
            ..ModelFilter::default()
        };
        let ids: Vec<&str> = filter_models(&models, filter)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();
        assert!(ids.contains(&"vendor/tools-model"));
        assert!(ids.contains(&"vendor/unknown-tools-model"));
        assert!(!ids.contains(&"vendor/no-tools-model"));
    }

    #[test]
    fn variants_are_hidden_by_default_and_revealed_on_request() {
        let models = filter_fixture();
        let hidden = filter_models(&models, ModelFilter::default());
        assert!(!hidden.iter().any(|m| m.is_variant));

        let shown = filter_models(
            &models,
            ModelFilter {
                show_variants: true,
                ..ModelFilter::default()
            },
        );
        assert!(shown.iter().any(|m| m.is_variant));
    }

    #[test]
    fn free_only_needs_a_reported_price_of_exactly_zero() {
        let models = filter_fixture();
        let filter = ModelFilter {
            free_only: true,
            show_variants: true,
            ..ModelFilter::default()
        };
        let ids: Vec<&str> = filter_models(&models, filter)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, vec!["vendor/free-model:free"]);
    }

    #[test]
    fn every_flag_combination_terminates_and_never_exceeds_the_input() {
        let models = filter_fixture();
        for tools_only in [false, true] {
            for text_only in [false, true] {
                for free_only in [false, true] {
                    for show_variants in [false, true] {
                        let filter = ModelFilter {
                            tools_only,
                            text_only,
                            free_only,
                            show_variants,
                        };
                        assert!(filter_models(&models, filter).len() <= models.len());
                    }
                }
            }
        }
    }

    #[test]
    fn filtering_preserves_the_servers_order() {
        let models = openrouter_models();
        let kept = filter_models(
            &models,
            ModelFilter {
                show_variants: true,
                ..ModelFilter::default()
            },
        );
        assert_eq!(kept[0].id, models[0].id);
    }

    // ── freshness and cache resolution ────────────────────────────

    #[test]
    fn freshness_boundaries() {
        let now = 1_000_000;
        assert_eq!(freshness(now, now), CatalogFreshness::Fresh);
        assert_eq!(
            freshness(now - CATALOG_TTL_SECS + 1, now),
            CatalogFreshness::Fresh
        );
        assert_eq!(
            freshness(now - CATALOG_TTL_SECS, now),
            CatalogFreshness::Stale
        );
        assert_eq!(
            freshness(now - CATALOG_STALE_SECS + 1, now),
            CatalogFreshness::Stale
        );
        assert_eq!(
            freshness(now - CATALOG_STALE_SECS, now),
            CatalogFreshness::Expired
        );
    }

    #[test]
    fn a_future_timestamp_reads_as_fresh_rather_than_refetching_forever() {
        assert_eq!(freshness(2_000_000, 1_000_000), CatalogFreshness::Fresh);
    }

    #[test]
    fn resolve_prefers_the_cache_and_falls_back_to_bundled() {
        let cached = CachedCatalog {
            schema: CATALOG_SCHEMA,
            fetched_at: 42,
            models: vec![model("cached/model")],
        };
        let (models, source) = resolve_catalog(AiProvider::OpenAi, Some(cached));
        assert_eq!(models[0].id, "cached/model");
        assert_eq!(source, CatalogSource::Cache { fetched_at: 42 });

        let (models, source) = resolve_catalog(AiProvider::OpenAi, None);
        assert_eq!(source, CatalogSource::Bundled);
        assert!(models
            .iter()
            .any(|m| m.id == AiProvider::OpenAi.default_model()));
    }

    #[test]
    fn a_cache_from_an_older_schema_is_discarded() {
        let cached = CachedCatalog {
            schema: CATALOG_SCHEMA + 1,
            fetched_at: 42,
            models: vec![model("cached/model")],
        };
        let (_, source) = resolve_catalog(AiProvider::OpenAi, Some(cached));
        assert_eq!(source, CatalogSource::Bundled);
    }

    #[test]
    fn an_empty_cache_falls_back_rather_than_rendering_nothing() {
        let cached = CachedCatalog {
            schema: CATALOG_SCHEMA,
            fetched_at: 42,
            models: Vec::new(),
        };
        let (models, source) = resolve_catalog(AiProvider::Gemini, Some(cached));
        assert_eq!(source, CatalogSource::Bundled);
        assert!(!models.is_empty());
    }

    #[test]
    fn only_openrouters_catalogue_is_public() {
        assert!(!catalog_needs_key(AiProvider::OpenRouter));
        for provider in [
            AiProvider::Gemini,
            AiProvider::OpenAi,
            AiProvider::Anthropic,
            AiProvider::DeepSeek,
        ] {
            assert!(catalog_needs_key(provider));
        }
    }

    // ── pinned-model classification ───────────────────────────────

    #[test]
    fn a_pinned_model_in_the_catalogue_is_known() {
        let models = vec![model("vendor/a")];
        assert!(matches!(
            classify_pinned("vendor/a", &models, CatalogSource::Live, true),
            PinnedModelStatus::Known(_)
        ));
    }

    #[test]
    fn a_missing_pin_suggests_the_closest_match_but_never_applies_it() {
        let models = vec![model("openai/gpt-5.6-sol"), model("vendor/unrelated")];
        match classify_pinned("openai/gpt-4o-mini", &models, CatalogSource::Live, false) {
            PinnedModelStatus::Missing { suggestion } => {
                assert_eq!(suggestion.as_deref(), Some("openai/gpt-5.6-sol"));
            }
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn a_suggestion_is_ranked_by_shared_family_not_by_search_relevance() {
        let models = vec![
            model("anthropic/claude-haiku-4.5"),
            model("openai/gpt-5.6-sol"),
            model("openai/gpt-5.6-luna"),
        ];
        // Longest shared prefix wins, and the tie between the two `gpt-5.6-`
        // ids breaks toward the shorter id.
        assert_eq!(
            closest_model_id("openai/gpt-5.6-x", &models).as_deref(),
            Some("openai/gpt-5.6-sol")
        );
        assert_eq!(
            closest_model_id("anthropic/claude-opus-4.6", &models).as_deref(),
            Some("anthropic/claude-haiku-4.5")
        );
    }

    /// A pin with nothing in common with anything on offer gets no
    /// suggestion. An unrelated id presented as "use this instead" is worse
    /// than saying nothing.
    #[test]
    fn no_suggestion_is_offered_when_nothing_is_in_the_same_family() {
        let models = vec![model("anthropic/claude-haiku-4.5")];
        assert_eq!(closest_model_id("zzz/unrelated-model", &models), None);
        assert_eq!(closest_model_id("", &models), None);
    }

    #[test]
    fn suggestions_ignore_case() {
        let models = vec![model("openai/gpt-5.6-luna")];
        assert_eq!(
            closest_model_id("OpenAI/GPT-5.6-Sol", &models).as_deref(),
            Some("openai/gpt-5.6-luna")
        );
    }

    /// The highest-value check: an opaque runtime 404 becomes a settings-time
    /// warning.
    #[test]
    fn a_tool_incompatible_pin_is_caught_before_the_request_is_ever_sent() {
        let mut no_tools = model("mistralai/mistral-7b-instruct");
        no_tools.tool_support = ToolSupport::Unsupported;
        match classify_pinned(
            "mistralai/mistral-7b-instruct",
            &[no_tools],
            CatalogSource::Live,
            true,
        ) {
            PinnedModelStatus::Incompatible { reason } => {
                assert!(reason.contains("does not support tool calling"));
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn the_same_pin_is_fine_once_tools_are_off() {
        let mut no_tools = model("mistralai/mistral-7b-instruct");
        no_tools.tool_support = ToolSupport::Unsupported;
        assert!(matches!(
            classify_pinned(
                "mistralai/mistral-7b-instruct",
                &[no_tools],
                CatalogSource::Live,
                false
            ),
            PinnedModelStatus::Known(_)
        ));
    }

    /// A bundled or absent catalogue is not evidence a model is gone. Crying
    /// wolf offline is worse than staying quiet.
    #[test]
    fn a_bundled_or_empty_catalogue_never_warns_about_an_unknown_pin() {
        assert_eq!(
            classify_pinned("anything", &[model("x")], CatalogSource::Bundled, true),
            PinnedModelStatus::Unverified
        );
        assert_eq!(
            classify_pinned("anything", &[], CatalogSource::Live, true),
            PinnedModelStatus::Unverified
        );
    }

    #[test]
    fn an_empty_pin_reads_as_missing_with_nothing_to_suggest() {
        assert_eq!(
            classify_pinned("   ", &[model("x")], CatalogSource::Live, true),
            PinnedModelStatus::Missing { suggestion: None }
        );
    }

    // ── presentation helpers ──────────────────────────────────────

    #[test]
    fn context_windows_render_the_way_provider_docs_do() {
        assert_eq!(format_context(1_048_576), "1.05M");
        assert_eq!(format_context(1_000_000), "1M");
        assert_eq!(format_context(128_000), "128K");
        assert_eq!(format_context(4_096), "4K");
        assert_eq!(format_context(512), "512");
    }

    #[test]
    fn the_summary_line_omits_what_a_provider_does_not_report() {
        let mut info = model("vendor/a");
        info.tool_support = ToolSupport::Unknown;
        assert_eq!(info.summary_line(), "128K ctx");

        info.prompt_price_per_mtok = Some(0.25);
        info.completion_price_per_mtok = Some(1.5);
        info.tool_support = ToolSupport::Supported;
        assert_eq!(
            info.summary_line(),
            "128K ctx · $0.25/$1.50 per Mtok · Tools"
        );
    }
}
