//! Endpoint resolution and request-body construction, per provider.
//!
//! Everything here is a pure function over `(provider, model, …)` so the shape
//! of every outgoing request is unit-testable without a network, a display, or
//! an API key. The Anthropic tool loop shipping an empty `messages` array —
//! which meant that provider had never once worked — is precisely the class of
//! bug these functions exist to make visible.

use rgitui_settings::AiProvider;
use serde_json::Value;

/// Attribution headers for OpenRouter's public model leaderboard. Optional and
/// never functional; the user can turn them off in Settings.
pub(crate) const OPENROUTER_ATTRIBUTION: &[(&str, &str)] = &[
    ("HTTP-Referer", "https://github.com/noahbclarkson/rgitui"),
    ("X-Title", "rgitui"),
];

/// How a provider expects the API key to be presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthStyle {
    /// `Authorization: Bearer <key>`
    Bearer,
    /// `x-api-key: <key>` plus `anthropic-version`
    AnthropicHeader,
    /// `x-goog-api-key: <key>`. Deliberately a header and not the `?key=`
    /// query parameter the code used to build: query strings land in proxy
    /// logs, TLS-inspecting middleboxes and any error path that prints a URL.
    GoogleHeader,
}

/// How this provider expects its API key to be presented.
pub(crate) fn auth_style(provider: AiProvider) -> AuthStyle {
    match provider {
        AiProvider::Gemini => AuthStyle::GoogleHeader,
        AiProvider::Anthropic => AuthStyle::AnthropicHeader,
        AiProvider::OpenAi | AiProvider::DeepSeek | AiProvider::OpenRouter => AuthStyle::Bearer,
    }
}

/// A resolved OpenAI-compatible chat endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiCompatEndpoint {
    /// Human-readable name used in user-facing error messages.
    pub provider: AiProvider,
    pub url: String,
    /// Appended after `Authorization` and `Content-Type`.
    pub extra_headers: Vec<(&'static str, &'static str)>,
}

/// The built-in chat-completions URL for an OpenAI-compatible provider.
fn builtin_openai_compat_url(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenAi => "https://api.openai.com/v1/chat/completions",
        AiProvider::DeepSeek => "https://api.deepseek.com/chat/completions",
        AiProvider::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
        // Gemini and Anthropic have their own request shapes and never route
        // through here; the OpenAI URL is the only sane placeholder and the
        // caller is guarded by `is_openai_compatible`.
        AiProvider::Gemini | AiProvider::Anthropic => "https://api.openai.com/v1/chat/completions",
    }
}

/// Resolve the endpoint for an OpenAI-compatible provider, honouring a
/// validated `base_url_override` when one is set.
///
/// The override applies only to this family: Gemini and Anthropic keep their
/// fixed endpoints, so pointing the field at a gateway can never silently
/// redirect a request the gateway does not understand.
pub(crate) fn openai_compat_endpoint(
    provider: AiProvider,
    base_url_override: &str,
    openrouter_attribution: bool,
) -> OpenAiCompatEndpoint {
    let url = match normalize_base_url(base_url_override) {
        Some(base) if provider.is_openai_compatible() => format!("{base}/chat/completions"),
        _ => builtin_openai_compat_url(provider).to_string(),
    };

    let extra_headers = if provider == AiProvider::OpenRouter && openrouter_attribution {
        OPENROUTER_ATTRIBUTION.to_vec()
    } else {
        Vec::new()
    };

    OpenAiCompatEndpoint {
        provider,
        url,
        extra_headers,
    }
}

/// Trim a user-supplied base URL to the form the endpoint builder expects:
/// no trailing slash, and no trailing `/chat/completions` the user may have
/// pasted from a curl example. Returns `None` for an empty field, which means
/// "use the built-in URL".
fn normalize_base_url(base_url_override: &str) -> Option<String> {
    let trimmed = base_url_override.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let trimmed = trimmed
        .strip_suffix("/chat/completions")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Why a `base_url_override` was rejected, phrased as something the user can
/// act on. Settings shows this inline; nothing is persisted until it passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseUrlError {
    NotAUrl,
    InsecureScheme,
    HasQueryOrFragment,
}

impl BaseUrlError {
    pub fn message(&self) -> &'static str {
        match self {
            BaseUrlError::NotAUrl => {
                "Enter a full URL, for example https://my-gateway.example.com/v1"
            }
            BaseUrlError::InsecureScheme => {
                "Use https://. Plain http:// is only allowed for localhost and 127.0.0.1."
            }
            BaseUrlError::HasQueryOrFragment => {
                "Remove the query string or #fragment — only the base path is used."
            }
        }
    }
}

/// Validate a user-supplied base URL.
///
/// An empty field is valid and means "use the built-in URL"; the default is
/// deliberately never stored in the field, so it cannot freeze at whatever
/// shipped.
pub fn validate_base_url(value: &str) -> Result<(), BaseUrlError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(BaseUrlError::HasQueryOrFragment);
    }

    let (scheme, rest) = match trimmed.split_once("://") {
        Some(parts) => parts,
        None => return Err(BaseUrlError::NotAUrl),
    };
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return Err(BaseUrlError::NotAUrl);
    }

    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(()),
        // Ollama's `/v1` on the loopback interface is the main local case, and
        // it does not serve TLS.
        "http" if is_loopback_host(host) => Ok(()),
        "http" => Err(BaseUrlError::InsecureScheme),
        _ => Err(BaseUrlError::NotAUrl),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host.split(':').next().unwrap_or(host);
    matches!(bare, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// The host a request will actually reach, for the settings warning that says
/// plainly where the user's API key is being sent.
pub fn effective_host(provider: AiProvider, base_url_override: &str) -> String {
    match normalize_base_url(base_url_override) {
        Some(base) if provider.is_openai_compatible() => base
            .split_once("://")
            .map(|(_, rest)| rest.split('/').next().unwrap_or(rest).to_string())
            .unwrap_or(base),
        _ => provider.default_host().to_string(),
    }
}

/// The Gemini `generateContent` URL. The key travels in a header, never here.
pub(crate) fn gemini_endpoint(model: &str) -> String {
    format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent")
}

pub(crate) const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The maximum tokens a commit message may consume. Generous enough for a
/// reasoning model's hidden tokens plus a real message body.
pub(crate) const MAX_OUTPUT_TOKENS: u32 = 4096;

/// The opening user turn of every conversation.
///
/// Anthropic rejects an empty `messages` array with a 400, which is why that
/// provider had never once produced a commit message.
pub(crate) const OPENING_USER_TURN: &str = "Generate a commit message for these changes.";

/// Build the first-iteration request body for a provider.
///
/// `messages` is the conversation so far — empty on the first iteration, which
/// is the case every provider must handle by seeding an opening turn.
pub(crate) fn build_request_body(
    provider: AiProvider,
    model: &str,
    prompt: &str,
    tools: Option<&Value>,
    messages: &[Value],
) -> Value {
    match provider {
        AiProvider::Gemini => {
            let mut contents: Vec<Value> = vec![serde_json::json!({
                "role": "user",
                "parts": [{ "text": prompt }]
            })];
            contents.extend(messages.iter().cloned());
            let mut body = serde_json::json!({
                "contents": contents,
                "generationConfig": {
                    "temperature": 0.3,
                    "maxOutputTokens": MAX_OUTPUT_TOKENS,
                    "topP": 0.8
                }
            });
            if let Some(tools) = tools {
                body["tools"] = serde_json::json!([tools]);
            }
            body
        }
        AiProvider::Anthropic => {
            let mut turns: Vec<Value> = vec![serde_json::json!({
                "role": "user",
                "content": OPENING_USER_TURN
            })];
            turns.extend(messages.iter().cloned());
            let mut body = serde_json::json!({
                "model": model,
                "max_tokens": MAX_OUTPUT_TOKENS,
                // A cache breakpoint after the system prompt means iterations
                // two and three read the (large) diff from cache instead of
                // re-billing it as fresh input on every round trip.
                "system": [{
                    "type": "text",
                    "text": prompt,
                    "cache_control": { "type": "ephemeral" }
                }],
                "messages": turns,
            });
            if let Some(tools) = tools {
                body["tools"] = tools.clone();
            }
            body
        }
        _ => {
            let mut turns: Vec<Value> = vec![
                serde_json::json!({ "role": "system", "content": prompt }),
                serde_json::json!({ "role": "user", "content": OPENING_USER_TURN }),
            ];
            turns.extend(messages.iter().cloned());
            let mut body = serde_json::json!({
                "model": model,
                "messages": turns,
                "temperature": 0.3,
                "max_tokens": MAX_OUTPUT_TOKENS,
            });
            if let Some(tools) = tools {
                body["tools"] = tools.clone();
                body["tool_choice"] = serde_json::json!("auto");
            }
            body
        }
    }
}

/// The conversation turns carried by a request body, for tests and for the
/// non-empty-first-turn invariant.
#[cfg(test)]
pub(crate) fn body_turns(provider: AiProvider, body: &Value) -> Vec<Value> {
    let key = match provider {
        AiProvider::Gemini => "contents",
        _ => "messages",
    };
    body[key].as_array().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_tools() -> Value {
        serde_json::json!([{ "name": "get_diff" }])
    }

    // ── the C1 regression test ────────────────────────────────────

    /// Anthropic's Messages API rejects `messages: []` with a 400. The tool
    /// loop shipped exactly that on its first iteration, so the provider had
    /// never worked for anyone. Assert it for every provider, not just the one
    /// that was broken.
    #[test]
    fn every_provider_sends_a_non_empty_first_turn() {
        for provider in AiProvider::ALL {
            let body = build_request_body(
                *provider,
                provider.default_model(),
                "PROMPT",
                Some(&all_tools()),
                &[],
            );
            let turns = body_turns(*provider, &body);
            assert!(
                !turns.is_empty(),
                "{} sends an empty conversation on iteration 1",
                provider.id()
            );
        }
    }

    #[test]
    fn anthropic_carries_the_prompt_in_system_not_in_the_user_turn() {
        let body = build_request_body(AiProvider::Anthropic, "m", "PROMPT", None, &[]);
        assert_eq!(body["system"][0]["text"], "PROMPT");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], OPENING_USER_TURN);
    }

    #[test]
    fn anthropic_sets_a_cache_breakpoint_on_the_prompt() {
        let body = build_request_body(AiProvider::Anthropic, "m", "PROMPT", None, &[]);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn openai_compatible_seeds_a_system_and_a_user_turn() {
        for provider in [
            AiProvider::OpenAi,
            AiProvider::DeepSeek,
            AiProvider::OpenRouter,
        ] {
            let body = build_request_body(provider, "m", "PROMPT", None, &[]);
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["messages"][0]["content"], "PROMPT");
            assert_eq!(body["messages"][1]["role"], "user");
        }
    }

    #[test]
    fn gemini_seeds_contents_with_the_prompt() {
        let body = build_request_body(AiProvider::Gemini, "m", "PROMPT", None, &[]);
        assert_eq!(body["contents"][0]["parts"][0]["text"], "PROMPT");
        assert_eq!(body["contents"][0]["role"], "user");
    }

    #[test]
    fn history_is_appended_after_the_seeded_turns() {
        let history = vec![serde_json::json!({ "role": "assistant", "content": "hi" })];
        let body = build_request_body(AiProvider::OpenAi, "m", "PROMPT", None, &history);
        let turns = body_turns(AiProvider::OpenAi, &body);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[2]["content"], "hi");
    }

    #[test]
    fn tools_are_omitted_entirely_when_not_requested() {
        for provider in AiProvider::ALL {
            let body = build_request_body(*provider, "m", "PROMPT", None, &[]);
            assert!(
                body.get("tools").is_none(),
                "{} sent a tools field with tools disabled",
                provider.id()
            );
        }
    }

    #[test]
    fn openai_compatible_sets_tool_choice_only_alongside_tools() {
        let with = build_request_body(AiProvider::OpenAi, "m", "P", Some(&all_tools()), &[]);
        assert_eq!(with["tool_choice"], "auto");
        let without = build_request_body(AiProvider::OpenAi, "m", "P", None, &[]);
        assert!(without.get("tool_choice").is_none());
    }

    /// The o-series rejects `max_tokens` and a non-default `temperature`, so
    /// those models are no longer offered at all. Every model still offered
    /// must accept the one body shape this function emits.
    #[test]
    fn openai_compatible_uses_max_tokens_not_max_completion_tokens() {
        let body = build_request_body(AiProvider::OpenAi, "gpt-5.6-luna", "P", None, &[]);
        assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
        assert!(body.get("max_completion_tokens").is_none());
    }

    // ── endpoints ─────────────────────────────────────────────────

    #[test]
    fn each_openai_compatible_provider_gets_its_own_url() {
        assert_eq!(
            openai_compat_endpoint(AiProvider::OpenAi, "", true).url,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_compat_endpoint(AiProvider::DeepSeek, "", true).url,
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            openai_compat_endpoint(AiProvider::OpenRouter, "", true).url,
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn attribution_headers_are_openrouter_only_and_opt_out_able() {
        let on = openai_compat_endpoint(AiProvider::OpenRouter, "", true);
        assert_eq!(on.extra_headers.len(), 2);
        assert!(on.extra_headers.iter().any(|(k, _)| *k == "HTTP-Referer"));
        assert!(on.extra_headers.iter().any(|(k, _)| *k == "X-Title"));

        let off = openai_compat_endpoint(AiProvider::OpenRouter, "", false);
        assert!(off.extra_headers.is_empty());

        let other = openai_compat_endpoint(AiProvider::OpenAi, "", true);
        assert!(other.extra_headers.is_empty());
    }

    #[test]
    fn base_url_override_applies_only_to_the_openai_compatible_family() {
        let overridden =
            openai_compat_endpoint(AiProvider::OpenAi, "https://gw.example.com/v1", true);
        assert_eq!(overridden.url, "https://gw.example.com/v1/chat/completions");

        // Gemini and Anthropic never route through here, and asking for the
        // compat endpoint for them must not adopt the override.
        let gemini = openai_compat_endpoint(AiProvider::Gemini, "https://gw.example.com/v1", true);
        assert_eq!(gemini.url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn base_url_override_tolerates_a_pasted_full_endpoint_and_trailing_slash() {
        for input in [
            "https://gw.example.com/v1",
            "https://gw.example.com/v1/",
            "https://gw.example.com/v1/chat/completions",
        ] {
            assert_eq!(
                openai_compat_endpoint(AiProvider::OpenAi, input, true).url,
                "https://gw.example.com/v1/chat/completions",
                "input {input}"
            );
        }
    }

    #[test]
    fn gemini_url_carries_no_key_query_parameter() {
        let url = gemini_endpoint("gemini-3.1-flash-lite");
        assert!(!url.contains("key="));
        assert!(url.ends_with(":generateContent"));
    }

    // ── base url validation ───────────────────────────────────────

    #[test]
    fn an_empty_override_is_valid_and_means_use_the_built_in_url() {
        assert_eq!(validate_base_url(""), Ok(()));
        assert_eq!(validate_base_url("   "), Ok(()));
    }

    #[test]
    fn https_is_required_except_on_loopback() {
        assert_eq!(validate_base_url("https://gw.example.com/v1"), Ok(()));
        assert_eq!(validate_base_url("http://localhost:11434/v1"), Ok(()));
        assert_eq!(validate_base_url("http://127.0.0.1:11434/v1"), Ok(()));
        assert_eq!(
            validate_base_url("http://gw.example.com/v1"),
            Err(BaseUrlError::InsecureScheme)
        );
    }

    #[test]
    fn a_query_string_or_fragment_is_rejected() {
        assert_eq!(
            validate_base_url("https://gw.example.com/v1?key=abc"),
            Err(BaseUrlError::HasQueryOrFragment)
        );
        assert_eq!(
            validate_base_url("https://gw.example.com/v1#x"),
            Err(BaseUrlError::HasQueryOrFragment)
        );
    }

    #[test]
    fn a_bare_host_or_unknown_scheme_is_rejected() {
        assert_eq!(
            validate_base_url("gw.example.com"),
            Err(BaseUrlError::NotAUrl)
        );
        assert_eq!(
            validate_base_url("ftp://gw.example.com"),
            Err(BaseUrlError::NotAUrl)
        );
        assert_eq!(validate_base_url("https://"), Err(BaseUrlError::NotAUrl));
    }

    #[test]
    fn effective_host_names_where_the_key_is_actually_sent() {
        assert_eq!(effective_host(AiProvider::OpenAi, ""), "api.openai.com");
        assert_eq!(
            effective_host(AiProvider::OpenAi, "https://gw.example.com/v1"),
            "gw.example.com"
        );
        // An override cannot redirect a provider that does not honour it, and
        // the warning must not claim otherwise.
        assert_eq!(
            effective_host(AiProvider::Gemini, "https://gw.example.com/v1"),
            "generativelanguage.googleapis.com"
        );
    }
}
