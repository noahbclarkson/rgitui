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
//! one is layered over it, so a translator can fix a few strings against a
//! release build without copying the whole file — the same arrangement custom
//! themes use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, SharedString};
use serde::Deserialize;

use crate::catalog::{Locale, PluralRule};

/// Largest user locale file read. The shipped Chinese catalogue is a few tens
/// of kilobytes; anything near this is not a translation, and reading it would
/// stall startup.
const MAX_USER_LOCALE_BYTES: u64 = 1024 * 1024;

/// On-disk shape of a locale file. Optional fields stay optional here so a
/// user file layered over a shipped locale only replaces what it states.
#[derive(Debug, Deserialize)]
pub(crate) struct LocaleFile {
    id: String,
    english_name: Option<String>,
    native_name: Option<String>,
    plural_rule: Option<String>,
    #[serde(default)]
    messages: HashMap<String, String>,
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl LocaleFile {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Blank values are treated as untranslated rather than as an empty
    /// label, so a half-finished file shows English instead of gaps.
    fn translated_messages(
        messages: HashMap<String, String>,
    ) -> impl Iterator<Item = (String, SharedString)> {
        messages
            .into_iter()
            .filter(|(_, value)| !value.trim().is_empty())
            .map(|(key, value)| (key, SharedString::from(value)))
    }

    fn plural_rule(&self) -> Option<PluralRule> {
        self.plural_rule
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .and_then(PluralRule::from_name)
    }

    /// A standalone locale, with names defaulting to the id.
    pub(crate) fn into_locale(self) -> Locale {
        let plural_rule = self.plural_rule().unwrap_or_default();
        let english_name = non_blank(self.english_name).unwrap_or_else(|| self.id.clone());
        let native_name = non_blank(self.native_name).unwrap_or_else(|| english_name.clone());
        Locale {
            id: self.id,
            english_name,
            native_name,
            plural_rule,
            messages: Self::translated_messages(self.messages).collect(),
        }
    }

    /// This file layered over `base`: what the file states wins, everything
    /// else is kept from `base`.
    pub(crate) fn overlay(self, base: &Locale) -> Locale {
        let plural_rule = self.plural_rule().unwrap_or(base.plural_rule);
        let mut messages = base.messages.clone();
        messages.extend(Self::translated_messages(self.messages));
        Locale {
            id: base.id.clone(),
            english_name: non_blank(self.english_name).unwrap_or_else(|| base.english_name.clone()),
            native_name: non_blank(self.native_name).unwrap_or_else(|| base.native_name.clone()),
            plural_rule,
            messages,
        }
    }
}

/// Parse a locale file. The error is a sentence suitable for a log line.
pub(crate) fn parse_locale_file(json: &str) -> Result<LocaleFile, String> {
    let mut file: LocaleFile =
        serde_json::from_str(json).map_err(|e| format!("invalid JSON: {}", e))?;

    file.id = file.id.trim().to_string();
    if file.id.is_empty() {
        return Err("locale file has an empty \"id\" field".to_string());
    }
    if let Some(name) = file.plural_rule.as_deref().map(str::trim) {
        if !name.is_empty() && PluralRule::from_name(name).is_none() {
            return Err(format!(
                "unknown plural_rule \"{name}\"; expected one_other, zero_one_other or other_only"
            ));
        }
    }
    Ok(file)
}

/// Parse a standalone locale file.
pub fn parse_locale(json: &str) -> Result<Locale, String> {
    parse_locale_file(json).map(LocaleFile::into_locale)
}

/// Insert `locale` into `available`, replacing any entry with the same id.
pub fn upsert_locale(available: &mut Vec<Arc<Locale>>, locale: Arc<Locale>) {
    match available.iter_mut().find(|l| l.id == locale.id) {
        Some(existing) => *existing = locale,
        None => available.push(locale),
    }
}

/// Add a user locale file to `available`, layering it over a locale with the
/// same id if there is one. Returns the locale as it now stands.
pub(crate) fn add_user_locale(available: &mut Vec<Arc<Locale>>, file: LocaleFile) -> Arc<Locale> {
    let locale = match available.iter().find(|l| l.id == file.id()) {
        Some(base) => Arc::new(file.overlay(base)),
        None => Arc::new(file.into_locale()),
    };
    upsert_locale(available, locale.clone());
    locale
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

/// Read one user locale file, refusing anything that is not a regular file of
/// a plausible size. A FIFO named `*.json` would otherwise block startup.
fn read_user_locale(path: &Path) -> Result<LocaleFile, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("not a regular file".to_string());
    }
    if metadata.len() > MAX_USER_LOCALE_BYTES {
        return Err(format!(
            "{} bytes is larger than the {} byte limit",
            metadata.len(),
            MAX_USER_LOCALE_BYTES
        ));
    }
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_locale_file(&json)
}

/// Every `.json` locale file the user has placed in `dir`, in path order so
/// that two files with the same id always layer the same way. A missing
/// directory is the normal case and is not reported.
pub(crate) fn read_user_locales(dir: &Path) -> Vec<(PathBuf, LocaleFile)> {
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

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();

    let mut files = Vec::new();
    for path in paths {
        match read_user_locale(&path) {
            Ok(file) => files.push((path, file)),
            Err(e) => log::warn!("skipping locale {}: {e}", path.display()),
        }
    }
    files
}

/// Directory the user can drop custom locale files into.
pub fn user_locales_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rgitui").join("locales"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::PluralRule;

    fn locale(id: &str, name: &str, messages: &[(&str, &str)]) -> Locale {
        Locale {
            id: id.to_string(),
            english_name: name.to_string(),
            native_name: name.to_string(),
            plural_rule: PluralRule::OtherOnly,
            messages: messages
                .iter()
                .map(|(k, v)| (k.to_string(), SharedString::from(v.to_string())))
                .collect(),
        }
    }

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
    fn rejects_an_unknown_plural_rule() {
        let err = parse_locale(r#"{ "id": "pl", "plural_rule": "one_few_many" }"#)
            .expect_err("should reject");
        assert!(err.contains("one_few_many"), "unexpected error: {err}");
    }

    #[test]
    fn upsert_replaces_a_locale_with_the_same_id() {
        let mut available = vec![
            Arc::new(locale("en", "English", &[])),
            Arc::new(locale("de", "German", &[])),
        ];
        upsert_locale(&mut available, Arc::new(locale("de", "Deutsch", &[])));
        assert_eq!(available.len(), 2);
        assert_eq!(available[1].english_name, "Deutsch");

        upsert_locale(&mut available, Arc::new(locale("fr", "French", &[])));
        assert_eq!(available.len(), 3);
    }

    #[test]
    fn a_user_file_layers_over_the_shipped_locale() {
        let mut available = vec![Arc::new(locale(
            "zh-CN",
            "简体中文",
            &[("a", "甲"), ("b", "乙")],
        ))];
        let file = parse_locale_file(
            r#"{ "id": "zh-CN", "messages": { "b": "改", "c": "丙", "a": " " } }"#,
        )
        .expect("file should parse");

        let merged = add_user_locale(&mut available, file);
        assert_eq!(available.len(), 1);
        assert_eq!(merged.native_name, "简体中文");
        assert_eq!(merged.plural_rule, PluralRule::OtherOnly);
        assert_eq!(merged.get("a").map(|s| s.as_ref()), Some("甲"));
        assert_eq!(merged.get("b").map(|s| s.as_ref()), Some("改"));
        assert_eq!(merged.get("c").map(|s| s.as_ref()), Some("丙"));
    }

    #[test]
    fn a_user_file_for_a_new_language_stands_alone() {
        let mut available = vec![Arc::new(locale("en", "English", &[]))];
        let file = parse_locale_file(
            r#"{ "id": "de", "english_name": "German", "plural_rule": "one_other" }"#,
        )
        .expect("file should parse");
        let added = add_user_locale(&mut available, file);
        assert_eq!(available.len(), 2);
        assert_eq!(added.english_name, "German");
        assert_eq!(added.plural_rule, PluralRule::OneOther);
    }

    #[test]
    fn loading_a_missing_directory_yields_nothing() {
        let missing = Path::new("this/directory/does/not/exist");
        assert!(read_user_locales(missing).is_empty());
    }

    #[test]
    fn user_files_are_read_in_path_order_and_oversized_or_odd_entries_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("b.json"), r#"{ "id": "b" }"#).unwrap();
        std::fs::write(dir.path().join("a.json"), r#"{ "id": "a" }"#).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a locale").unwrap();
        std::fs::create_dir(dir.path().join("dir.json")).unwrap();
        let oversized = vec![b' '; MAX_USER_LOCALE_BYTES as usize + 1];
        std::fs::write(dir.path().join("big.json"), oversized).unwrap();

        let ids: Vec<String> = read_user_locales(dir.path())
            .into_iter()
            .map(|(_, file)| file.id().to_string())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
