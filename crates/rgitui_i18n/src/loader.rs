//! Parsing and discovery of translation files.
//!
//! A translation is a single JSON document:
//!
//! ```json
//! {
//!   "id": "zh-CN",
//!   "english_name": "Chinese (Simplified)",
//!   "native_name": "简体中文",
//!   "plural_rule": "other_only",
//!   "messages": {
//!     "settings.nav.appearance": "外观"
//!   }
//! }
//! ```
//!
//! Translations are loaded from two places, in order: the locale files shipped
//! inside the binary (`assets/locales/`), then any the user has dropped in
//! `<config_dir>/rgitui/locales/`. A user file with the same `id` as a shipped
//! one replaces it, which is what lets a translator iterate on a language
//! without rebuilding — the same arrangement custom themes use.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui::{App, SharedString};
use serde::Deserialize;

use crate::catalog::{Locale, PluralRule};

/// On-disk shape of a locale file.
#[derive(Debug, Deserialize)]
struct LocaleFile {
    id: String,
    #[serde(default)]
    english_name: String,
    #[serde(default)]
    native_name: String,
    #[serde(default)]
    plural_rule: String,
    #[serde(default)]
    messages: HashMap<String, String>,
}

/// Parse a locale file. The error is a sentence suitable for a log line.
pub fn parse_locale(json: &str) -> Result<Locale, String> {
    let file: LocaleFile =
        serde_json::from_str(json).map_err(|e| format!("invalid JSON: {}", e))?;

    if file.id.trim().is_empty() {
        return Err("locale file has an empty \"id\" field".to_string());
    }

    let id = file.id.trim().to_string();
    let english_name = if file.english_name.trim().is_empty() {
        id.clone()
    } else {
        file.english_name.trim().to_string()
    };
    let native_name = if file.native_name.trim().is_empty() {
        english_name.clone()
    } else {
        file.native_name.trim().to_string()
    };

    // Blank values are treated as untranslated rather than as an empty label,
    // so a half-finished file shows English instead of gaps in the UI.
    let messages = file
        .messages
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| (key, SharedString::from(value)))
        .collect();

    Ok(Locale {
        id,
        english_name,
        native_name,
        plural_rule: PluralRule::from_name(&file.plural_rule),
        messages,
    })
}

/// Insert `locale` into `available`, replacing any entry with the same id.
pub fn upsert_locale(available: &mut Vec<Arc<Locale>>, locale: Arc<Locale>) {
    match available.iter_mut().find(|l| l.id == locale.id) {
        Some(existing) => *existing = locale,
        None => available.push(locale),
    }
}

/// Load every `.json` locale bundled into the binary under `assets/locales/`.
pub fn load_embedded_locales(cx: &App) -> Vec<Arc<Locale>> {
    let asset_source = cx.asset_source();
    let paths = match asset_source.list("locales") {
        Ok(paths) => paths,
        Err(e) => {
            log::warn!("failed to list embedded locales: {e}");
            return Vec::new();
        }
    };

    let mut locales = Vec::new();
    for path in paths {
        if !path.ends_with(".json") {
            continue;
        }
        match asset_source.load(&path) {
            Ok(Some(bytes)) => match std::str::from_utf8(&bytes) {
                Ok(json) => match parse_locale(json) {
                    Ok(locale) => {
                        log::info!(
                            "loaded bundled locale {} ({})",
                            locale.id,
                            locale.english_name
                        );
                        locales.push(Arc::new(locale));
                    }
                    Err(e) => log::warn!("failed to parse locale {path}: {e}"),
                },
                Err(e) => log::warn!("locale {path} is not valid UTF-8: {e}"),
            },
            Ok(None) => log::warn!("locale path listed but load returned None: {path}"),
            Err(e) => log::warn!("failed to load locale {path}: {e}"),
        }
    }
    locales
}

/// Load every `.json` locale the user has placed in `dir`. A missing directory
/// is the normal case and is not reported.
pub fn load_locales_from_dir(dir: &Path) -> Vec<Arc<Locale>> {
    if !dir.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("failed to read locales directory {}: {e}", dir.display());
            return Vec::new();
        }
    };

    let mut locales = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match parse_locale(&json) {
                Ok(locale) => {
                    log::info!("loaded custom locale {} from {}", locale.id, path.display());
                    locales.push(Arc::new(locale));
                }
                Err(e) => log::warn!("failed to parse locale {}: {e}", path.display()),
            },
            Err(e) => log::warn!("failed to read locale {}: {e}", path.display()),
        }
    }
    locales
}

/// Directory the user can drop custom locale files into.
pub fn user_locales_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rgitui").join("locales"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::PluralRule;

    #[test]
    fn parses_a_complete_locale_file() {
        let json = r#"{
            "id": "zh-CN",
            "english_name": "Chinese (Simplified)",
            "native_name": "简体中文",
            "plural_rule": "other_only",
            "messages": { "settings.nav.appearance": "外观" }
        }"#;

        let locale = parse_locale(json).expect("locale should parse");
        assert_eq!(locale.id, "zh-CN");
        assert_eq!(locale.english_name, "Chinese (Simplified)");
        assert_eq!(locale.native_name, "简体中文");
        assert_eq!(locale.plural_rule, PluralRule::OtherOnly);
        assert_eq!(
            locale.get("settings.nav.appearance").map(|s| s.as_ref()),
            Some("外观")
        );
    }

    #[test]
    fn missing_names_default_to_the_locale_id() {
        let locale = parse_locale(r#"{ "id": "fr" }"#).expect("locale should parse");
        assert_eq!(locale.english_name, "fr");
        assert_eq!(locale.native_name, "fr");
        assert!(locale.messages.is_empty());
    }

    #[test]
    fn native_name_defaults_to_english_name() {
        let locale = parse_locale(r#"{ "id": "fr", "english_name": "French" }"#)
            .expect("locale should parse");
        assert_eq!(locale.native_name, "French");
    }

    #[test]
    fn blank_messages_are_treated_as_untranslated() {
        let json = r#"{ "id": "de", "messages": { "a": "Ja", "b": "", "c": "   " } }"#;
        let locale = parse_locale(json).expect("locale should parse");
        assert!(locale.get("a").is_some());
        assert!(locale.get("b").is_none());
        assert!(locale.get("c").is_none());
    }

    #[test]
    fn rejects_a_file_without_an_id() {
        assert!(parse_locale(r#"{ "english_name": "French" }"#).is_err());
        assert!(parse_locale(r#"{ "id": "  " }"#).is_err());
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_locale("{ not json").expect_err("should reject");
        assert!(err.contains("invalid JSON"), "unexpected error: {err}");
    }

    #[test]
    fn upsert_replaces_a_locale_with_the_same_id() {
        let make = |id: &str, name: &str| {
            Arc::new(Locale {
                id: id.to_string(),
                english_name: name.to_string(),
                native_name: name.to_string(),
                plural_rule: PluralRule::OneOther,
                messages: HashMap::new(),
            })
        };

        let mut available = vec![make("en", "English"), make("de", "German")];
        upsert_locale(&mut available, make("de", "Deutsch"));
        assert_eq!(available.len(), 2);
        assert_eq!(available[1].english_name, "Deutsch");

        upsert_locale(&mut available, make("fr", "French"));
        assert_eq!(available.len(), 3);
    }

    #[test]
    fn loading_a_missing_directory_yields_nothing() {
        let missing = Path::new("this/directory/does/not/exist");
        assert!(load_locales_from_dir(missing).is_empty());
    }
}
