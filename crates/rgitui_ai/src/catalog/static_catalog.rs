//! The bundled model table.
//!
//! Successor to the hardcoded `Vec<&str>` that used to live inside a render
//! function in the settings view. It serves two jobs:
//!
//! 1. The last-resort catalogue when there is no cache and no network.
//! 2. Metadata enrichment for OpenAI and DeepSeek, whose `/models` endpoints
//!    report neither a context window nor tool support, so a live fetch alone
//!    would render a list with every column blank.
//!
//! It goes stale between releases. That is acceptable for a fallback — the
//! picker labels the source, so "shipped with the app" never masquerades as
//! "current" — but it is the reason the live catalogue exists.

use super::{ModelInfo, ToolSupport};
use rgitui_settings::AiProvider;

pub(crate) struct StaticModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub context_length: u32,
    pub max_output_tokens: u32,
    pub tool_support: ToolSupport,
}

const fn model(
    id: &'static str,
    display_name: &'static str,
    context_length: u32,
    max_output_tokens: u32,
    tool_support: ToolSupport,
) -> StaticModel {
    StaticModel {
        id,
        display_name,
        context_length,
        max_output_tokens,
        tool_support,
    }
}

const GEMINI: &[StaticModel] = &[
    model(
        "gemini-3.1-flash-lite",
        "Gemini 3.1 Flash Lite",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
    model(
        "gemini-3.1-pro-preview",
        "Gemini 3.1 Pro (preview)",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
    model(
        "gemini-3-flash-preview",
        "Gemini 3 Flash (preview)",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
    model(
        "gemini-2.5-flash",
        "Gemini 2.5 Flash",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
    model(
        "gemini-2.5-pro",
        "Gemini 2.5 Pro",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
];

// `o3` and `o4-mini` are deliberately absent: the o-series rejects both
// `max_tokens` and a non-default `temperature`, so every request this app
// builds 400s against them, and they are being retired regardless.
const OPENAI: &[StaticModel] = &[
    model(
        "gpt-5.6-luna",
        "GPT-5.6 Luna",
        1_047_576,
        32_768,
        ToolSupport::Supported,
    ),
    model(
        "gpt-5.4",
        "GPT-5.4",
        400_000,
        128_000,
        ToolSupport::Supported,
    ),
    model("gpt-5", "GPT-5", 400_000, 128_000, ToolSupport::Supported),
    model(
        "gpt-5-mini",
        "GPT-5 mini",
        400_000,
        128_000,
        ToolSupport::Supported,
    ),
    model(
        "gpt-5-nano",
        "GPT-5 nano",
        400_000,
        128_000,
        ToolSupport::Supported,
    ),
];

// `claude-sonnet-4-5-20241022` is deliberately absent: `20241022` is the Claude
// 3.5 snapshot date attached to a 4.5 name, so it was a guaranteed 404.
const ANTHROPIC: &[StaticModel] = &[
    model(
        "claude-haiku-4-5",
        "Claude Haiku 4.5",
        200_000,
        64_000,
        ToolSupport::Supported,
    ),
    model(
        "claude-sonnet-4-6",
        "Claude Sonnet 4.6",
        200_000,
        64_000,
        ToolSupport::Supported,
    ),
    model(
        "claude-opus-4-6",
        "Claude Opus 4.6",
        200_000,
        64_000,
        ToolSupport::Supported,
    ),
];

const DEEPSEEK: &[StaticModel] = &[
    model(
        "deepseek-v4-flash",
        "DeepSeek V4 Flash",
        128_000,
        8_192,
        ToolSupport::Supported,
    ),
    model(
        "deepseek-v4-pro",
        "DeepSeek V4 Pro",
        128_000,
        8_192,
        ToolSupport::Supported,
    ),
];

// OpenRouter's real catalogue is fetched live and needs no key, so the bundled
// slice only has to cover the offline case with a handful of safe defaults.
const OPENROUTER: &[StaticModel] = &[
    model(
        "google/gemini-3.1-flash-lite",
        "Gemini 3.1 Flash Lite",
        1_048_576,
        65_536,
        ToolSupport::Supported,
    ),
    model(
        "openai/gpt-5.6-luna",
        "GPT-5.6 Luna",
        1_047_576,
        32_768,
        ToolSupport::Supported,
    ),
    model(
        "anthropic/claude-haiku-4.5",
        "Claude Haiku 4.5",
        200_000,
        64_000,
        ToolSupport::Supported,
    ),
    model(
        "deepseek/deepseek-v4-flash",
        "DeepSeek V4 Flash",
        128_000,
        8_192,
        ToolSupport::Supported,
    ),
];

pub(crate) fn static_models(provider: AiProvider) -> &'static [StaticModel] {
    match provider {
        AiProvider::Gemini => GEMINI,
        AiProvider::OpenAi => OPENAI,
        AiProvider::Anthropic => ANTHROPIC,
        AiProvider::DeepSeek => DEEPSEEK,
        AiProvider::OpenRouter => OPENROUTER,
    }
}

/// The bundled catalogue for a provider, as `ModelInfo` rows.
pub fn bundled_catalog(provider: AiProvider) -> Vec<ModelInfo> {
    static_models(provider)
        .iter()
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            display_name: m.display_name.to_string(),
            context_length: Some(m.context_length),
            max_output_tokens: Some(m.max_output_tokens),
            prompt_price_per_mtok: None,
            completion_price_per_mtok: None,
            tool_support: m.tool_support,
            emits_text: true,
            is_variant: false,
            created: None,
        })
        .collect()
}

/// Fill in what a provider's live `/models` endpoint does not report.
///
/// OpenAI and DeepSeek return an id and nothing else, so without this every
/// row in the picker would show a blank context window and no tool badge even
/// when the fetch succeeded.
pub fn enrich_from_static(provider: AiProvider, models: &mut [ModelInfo]) {
    let table = static_models(provider);
    for info in models.iter_mut() {
        let Some(known) = table.iter().find(|m| m.id == info.id) else {
            continue;
        };
        if info.display_name.is_empty() || info.display_name == info.id {
            info.display_name = known.display_name.to_string();
        }
        info.context_length = info.context_length.or(Some(known.context_length));
        info.max_output_tokens = info.max_output_tokens.or(Some(known.max_output_tokens));
        if info.tool_support == ToolSupport::Unknown {
            info.tool_support = known.tool_support;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_a_bundled_catalogue_containing_its_default() {
        for provider in AiProvider::ALL {
            let models = bundled_catalog(*provider);
            assert!(
                !models.is_empty(),
                "{} has no bundled models",
                provider.id()
            );
            assert!(
                models.iter().any(|m| m.id == provider.default_model()),
                "{}'s default model {} is not in its own list — the exact bug that \
                 rendered the Model row with nothing selected",
                provider.id(),
                provider.default_model()
            );
        }
    }

    #[test]
    fn the_broken_model_ids_are_gone() {
        let dead = [
            "claude-sonnet-4-5-20241022",
            "o3",
            "o4-mini",
            "gemini-2.0-flash",
        ];
        for provider in AiProvider::ALL {
            for model in bundled_catalog(*provider) {
                assert!(
                    !dead.contains(&model.id.as_str()),
                    "{} still offers the retired id {}",
                    provider.id(),
                    model.id
                );
            }
        }
    }

    #[test]
    fn bundled_ids_are_unique_within_a_provider() {
        for provider in AiProvider::ALL {
            let mut ids: Vec<String> = bundled_catalog(*provider)
                .into_iter()
                .map(|m| m.id)
                .collect();
            let count = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), count, "{} has duplicate ids", provider.id());
        }
    }

    #[test]
    fn enrichment_fills_gaps_without_overwriting_live_data() {
        let mut models = vec![
            ModelInfo {
                id: "gpt-5.4".into(),
                display_name: "gpt-5.4".into(),
                context_length: None,
                max_output_tokens: None,
                prompt_price_per_mtok: None,
                completion_price_per_mtok: None,
                tool_support: ToolSupport::Unknown,
                emits_text: true,
                is_variant: false,
                created: None,
            },
            ModelInfo {
                id: "gpt-5".into(),
                display_name: "Live Name".into(),
                context_length: Some(1),
                max_output_tokens: Some(2),
                prompt_price_per_mtok: Some(3.0),
                completion_price_per_mtok: None,
                tool_support: ToolSupport::Unsupported,
                emits_text: true,
                is_variant: false,
                created: None,
            },
        ];
        enrich_from_static(AiProvider::OpenAi, &mut models);

        assert_eq!(models[0].display_name, "GPT-5.4");
        assert_eq!(models[0].context_length, Some(400_000));
        assert_eq!(models[0].tool_support, ToolSupport::Supported);

        // Live values win.
        assert_eq!(models[1].display_name, "Live Name");
        assert_eq!(models[1].context_length, Some(1));
        assert_eq!(models[1].tool_support, ToolSupport::Unsupported);
    }

    #[test]
    fn an_unknown_id_passes_through_enrichment_untouched() {
        let mut models = vec![ModelInfo {
            id: "some-future-model".into(),
            display_name: "some-future-model".into(),
            context_length: None,
            max_output_tokens: None,
            prompt_price_per_mtok: None,
            completion_price_per_mtok: None,
            tool_support: ToolSupport::Unknown,
            emits_text: true,
            is_variant: false,
            created: None,
        }];
        enrich_from_static(AiProvider::OpenAi, &mut models);
        assert_eq!(models[0].context_length, None);
        assert_eq!(models[0].tool_support, ToolSupport::Unknown);
    }
}
