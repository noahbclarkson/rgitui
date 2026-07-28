//! Localization for rgitui.
//!
//! # Using a translated string
//!
//! ```ignore
//! use rgitui_i18n::{t, tp};
//!
//! Label::new(t!("settings.language.label"));
//! Label::new(t!("settings.language.system_detail", language = "English"));
//! Label::new(tp!("settings.language.missing", missing, count = missing));
//! ```
//!
//! [`t!`] returns a [`SharedString`], which every `rgitui_ui` component already
//! accepts, so translating a call site is a one-line change.
//!
//! # How lookup works
//!
//! The catalogue lives in a process-wide store rather than a GPUI global, so
//! `t!` works from background threads — error messages are built off the UI
//! thread and still need translating. Lookup order is active locale, then
//! English, then the key itself. A key rendered verbatim in the UI is a missing
//! entry in `ENGLISH_MESSAGES`.
//!
//! Because the store is not a GPUI global, GPUI does not know that a language
//! change invalidates the frame. Callers of [`set_language`] must follow it
//! with `cx.refresh_windows()`.

mod catalog;
mod en;
mod loader;

use std::sync::{Arc, OnceLock};

use gpui::{App, SharedString};
use parking_lot::RwLock;

pub use catalog::{Locale, PluralCategory, PluralRule};
pub use en::{ENGLISH_ID, ENGLISH_MESSAGES, ENGLISH_NAME};
pub use loader::user_locales_dir;

use catalog::interpolate;
use loader::{load_embedded_locales, load_locales_from_dir, upsert_locale};

/// Sentinel stored in settings when the UI language should follow the OS.
pub const SYSTEM_LANGUAGE: &str = "system";

/// Build the English catalogue from `ENGLISH_MESSAGES`.
pub fn english_locale() -> Locale {
    Locale {
        id: ENGLISH_ID.to_string(),
        english_name: ENGLISH_NAME.to_string(),
        native_name: ENGLISH_NAME.to_string(),
        plural_rule: PluralRule::OneOther,
        messages: ENGLISH_MESSAGES
            .iter()
            .map(|(key, value)| (key.to_string(), SharedString::from(*value)))
            .collect(),
    }
}

struct Store {
    english: Arc<Locale>,
    active: Arc<Locale>,
    available: Vec<Arc<Locale>>,
    /// The user's stored preference: [`SYSTEM_LANGUAGE`] or a locale id. Kept
    /// distinct from `active` so the picker can show "Match system language" as
    /// selected rather than whichever language that resolved to.
    selection: String,
}

impl Store {
    /// English-only store, used before [`init`] runs and by unit tests in other
    /// crates that never boot a GPUI app.
    fn english_only() -> Self {
        let english = Arc::new(english_locale());
        Self {
            active: english.clone(),
            available: vec![english.clone()],
            english,
            selection: ENGLISH_ID.to_string(),
        }
    }
}

fn store() -> &'static RwLock<Store> {
    static STORE: OnceLock<RwLock<Store>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(Store::english_only()))
}

/// Load all available locales and apply `selection`.
///
/// `selection` is the persisted `language` setting: [`SYSTEM_LANGUAGE`] or a
/// locale id. Call once during app init, after the asset source is installed.
pub fn init(cx: &mut App, selection: &str) {
    let english = Arc::new(english_locale());
    let mut available = vec![english.clone()];

    for locale in load_embedded_locales(cx) {
        upsert_locale(&mut available, locale);
    }
    if let Some(dir) = user_locales_dir() {
        for locale in load_locales_from_dir(&dir) {
            upsert_locale(&mut available, locale);
        }
    }

    // English first, then by the name shown in the picker.
    available.sort_by(|a, b| match (a.id == ENGLISH_ID, b.id == ENGLISH_ID) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.english_name.cmp(&b.english_name),
    });

    {
        let mut store = store().write();
        store.english = english.clone();
        store.active = english;
        store.available = available;
    }

    set_language(selection);
}

/// Apply a language selection, returning the locale that was actually
/// activated. Callers on the UI thread must follow this with
/// `cx.refresh_windows()` so already-rendered frames pick up the new strings.
pub fn set_language(selection: &str) -> Arc<Locale> {
    let mut store = store().write();

    let resolved = if selection == SYSTEM_LANGUAGE {
        system_locale_id()
            .and_then(|id| best_match(&store.available, &id))
            .unwrap_or_else(|| store.english.clone())
    } else {
        match best_match(&store.available, selection) {
            Some(locale) => locale,
            None => {
                log::warn!("unknown language {selection:?} in settings — falling back to English");
                store.english.clone()
            }
        }
    };

    log::info!("UI language: {} (selection {:?})", resolved.id, selection);
    store.active = resolved.clone();
    store.selection = selection.to_string();
    resolved
}

/// Find the locale that best matches `wanted`: an exact id match if there is
/// one, otherwise a locale sharing the primary subtag — so a system locale of
/// `zh-Hans-CN` still selects the shipped `zh-CN`.
fn best_match(available: &[Arc<Locale>], wanted: &str) -> Option<Arc<Locale>> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }

    if let Some(exact) = available
        .iter()
        .find(|locale| locale.id.eq_ignore_ascii_case(wanted))
    {
        return Some(exact.clone());
    }

    let wanted_primary = primary_subtag(wanted);
    available
        .iter()
        .find(|locale| primary_subtag(&locale.id).eq_ignore_ascii_case(wanted_primary))
        .cloned()
}

fn primary_subtag(id: &str) -> &str {
    id.split(['-', '_']).next().unwrap_or(id)
}

/// The language the OS reports, e.g. `en-US`. `None` if it cannot be detected.
pub fn system_locale_id() -> Option<String> {
    sys_locale::get_locale().filter(|id| !id.trim().is_empty())
}

/// The locale currently rendering the UI.
pub fn active_locale() -> Arc<Locale> {
    store().read().active.clone()
}

/// The stored language preference: [`SYSTEM_LANGUAGE`] or a locale id.
pub fn language_selection() -> String {
    store().read().selection.clone()
}

/// Every locale the user can pick, English first.
pub fn available_locales() -> Vec<Arc<Locale>> {
    store().read().available.clone()
}

/// Number of English messages `locale_id` does not translate.
pub fn missing_count_of(locale_id: &str) -> usize {
    let store = store().read();
    match store.available.iter().find(|l| l.id == locale_id) {
        Some(locale) => store
            .english
            .messages
            .keys()
            .filter(|key| locale.get(key).is_none())
            .count(),
        None => 0,
    }
}

/// Look up `key`, falling back to English and then to the key itself.
pub fn translate(key: &str) -> SharedString {
    let store = store().read();
    if let Some(message) = store.active.get(key) {
        return message.clone();
    }
    match store.english.get(key) {
        Some(message) => message.clone(),
        None => {
            log::debug!("missing translation key: {key}");
            SharedString::from(key.to_string())
        }
    }
}

/// Look up `key` and substitute its `{name}` placeholders.
pub fn translate_args(key: &str, args: &[(&str, String)]) -> SharedString {
    SharedString::from(interpolate(translate(key).as_ref(), args))
}

/// Look up the plural form of `key` for `count` under the active locale's
/// plural rule, then substitute placeholders.
///
/// `key` is the stem: `settings.language.missing` resolves to
/// `settings.language.missing.one` or `…other`. `count` is not substituted
/// automatically — pass it as an argument if the message displays it.
pub fn translate_plural(key: &str, count: i64, args: &[(&str, String)]) -> SharedString {
    let store = store().read();
    let category = store.active.plural_rule.select(count);

    let active_key = format!("{key}.{}", category.suffix());
    if let Some(message) = store.active.get(&active_key) {
        return SharedString::from(interpolate(message.as_ref(), args));
    }

    // The English fallback re-selects under English's own rule: a Chinese
    // catalogue that only defines `.other` must not send us looking for an
    // English `.other` when the count is 1.
    let english_category = store.english.plural_rule.select(count);
    let english_key = format!("{key}.{}", english_category.suffix());
    match store.english.get(&english_key) {
        Some(message) => SharedString::from(interpolate(message.as_ref(), args)),
        None => {
            log::debug!("missing plural translation key: {english_key}");
            SharedString::from(english_key)
        }
    }
}

/// Values usable as a plural count, so call sites can pass the `usize` from
/// `.len()` without casting.
pub trait IntoCount {
    fn into_count(self) -> i64;
}

macro_rules! impl_into_count {
    ($($ty:ty),+) => {
        $(impl IntoCount for $ty {
            fn into_count(self) -> i64 {
                self as i64
            }
        })+
    };
}

impl_into_count!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

/// Translate a message key.
///
/// ```ignore
/// t!("settings.language.label")
/// t!("settings.language.system_detail", language = "English")
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::translate($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate_args(
            $key,
            &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),+],
        )
    };
}

/// Translate a count-dependent message key.
///
/// ```ignore
/// tp!("settings.language.missing", missing, count = missing)
/// ```
#[macro_export]
macro_rules! tp {
    ($key:expr, $count:expr) => {
        $crate::translate_plural($key, $crate::IntoCount::into_count($count), &[])
    };
    ($key:expr, $count:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate_plural(
            $key,
            $crate::IntoCount::into_count($count),
            &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),+],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::parse_locale;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn shipped_locales_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("locales")
    }

    fn shipped_locales() -> Vec<Locale> {
        let dir = shipped_locales_dir();
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()));

        let mut locales = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let json = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            let locale = parse_locale(&json)
                .unwrap_or_else(|e| panic!("{} is not a valid locale: {e}", path.display()));
            locales.push(locale);
        }
        locales
    }

    fn test_locale(id: &str, native_name: &str) -> Locale {
        Locale {
            id: id.to_string(),
            english_name: id.to_string(),
            native_name: native_name.to_string(),
            plural_rule: PluralRule::OtherOnly,
            messages: Default::default(),
        }
    }

    fn placeholders(message: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = message;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            match after.find('}') {
                Some(close) => {
                    found.push(format!("{{{}}}", &after[..close]));
                    rest = &after[close + 1..];
                }
                None => break,
            }
        }
        found
    }

    #[test]
    fn english_catalogue_has_no_duplicate_keys() {
        let mut seen = HashSet::new();
        for (key, _) in ENGLISH_MESSAGES {
            assert!(
                seen.insert(*key),
                "duplicate key in ENGLISH_MESSAGES: {key}"
            );
        }
    }

    #[test]
    fn english_catalogue_is_sorted_by_key() {
        let keys: Vec<&str> = ENGLISH_MESSAGES.iter().map(|(key, _)| *key).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(
            keys, sorted,
            "keep ENGLISH_MESSAGES sorted by key so translation diffs stay readable"
        );
    }

    #[test]
    fn english_catalogue_has_no_empty_messages() {
        for (key, value) in ENGLISH_MESSAGES {
            assert!(!value.trim().is_empty(), "empty English message for {key}");
        }
    }

    #[test]
    fn english_plural_keys_come_in_pairs() {
        let english = english_locale();
        for key in english.messages.keys() {
            if let Some(stem) = key.strip_suffix(".one") {
                assert!(
                    english.get(&format!("{stem}.other")).is_some(),
                    "{key} has no matching .other form"
                );
            }
            if let Some(stem) = key.strip_suffix(".other") {
                assert!(
                    english.get(&format!("{stem}.one")).is_some(),
                    "{key} has no matching .one form"
                );
            }
        }
    }

    /// Shipped translations are allowed to be incomplete — missing keys fall
    /// back to English. They are not allowed to contain keys English does not
    /// define: those are typos or leftovers from a deleted message, and they
    /// would silently never render.
    #[test]
    fn shipped_locales_have_no_unknown_keys() {
        let english = english_locale();
        for locale in shipped_locales() {
            let unknown = locale.unknown_keys(&english);
            assert!(
                unknown.is_empty(),
                "locale {} defines keys that are not in ENGLISH_MESSAGES: {:?}",
                locale.id,
                unknown
            );
        }
    }

    #[test]
    fn shipped_locales_have_distinct_ids_that_are_not_english() {
        let mut seen = HashSet::new();
        for locale in shipped_locales() {
            assert_ne!(
                locale.id, ENGLISH_ID,
                "English is defined in Rust, not as an asset file"
            );
            assert!(seen.insert(locale.id.clone()), "duplicate locale id");
        }
    }

    #[test]
    fn shipped_locales_declare_a_native_name() {
        for locale in shipped_locales() {
            assert!(
                !locale.native_name.trim().is_empty() && locale.native_name != locale.id,
                "locale {} should declare native_name so users can find it",
                locale.id
            );
        }
    }

    #[test]
    fn shipped_locale_placeholders_match_english() {
        let english = english_locale();
        for locale in shipped_locales() {
            for (key, translated) in &locale.messages {
                let Some(source) = english.get(key) else {
                    continue;
                };
                for placeholder in placeholders(source.as_ref()) {
                    assert!(
                        translated.as_ref().contains(&placeholder),
                        "locale {} is missing placeholder {} in key {}",
                        locale.id,
                        placeholder,
                        key
                    );
                }
            }
        }
    }

    #[test]
    fn best_match_prefers_an_exact_id() {
        let available = vec![
            Arc::new(english_locale()),
            Arc::new(test_locale("zh-CN", "简体中文")),
            Arc::new(test_locale("zh-TW", "繁體中文")),
        ];
        let matched = best_match(&available, "zh-TW").expect("exact match");
        assert_eq!(matched.id, "zh-TW");
    }

    #[test]
    fn best_match_falls_back_to_the_primary_subtag() {
        let available = vec![
            Arc::new(english_locale()),
            Arc::new(test_locale("zh-CN", "简体中文")),
        ];
        let matched = best_match(&available, "zh-Hans-SG").expect("primary subtag match");
        assert_eq!(matched.id, "zh-CN");

        let matched = best_match(&available, "en-GB").expect("primary subtag match");
        assert_eq!(matched.id, ENGLISH_ID);
    }

    #[test]
    fn best_match_ignores_case_and_separator_style() {
        let available = vec![Arc::new(test_locale("zh-CN", "简体中文"))];
        assert!(best_match(&available, "ZH-cn").is_some());
        assert!(best_match(&available, "zh_CN").is_some());
    }

    #[test]
    fn best_match_rejects_an_unrelated_language() {
        let available = vec![Arc::new(english_locale())];
        assert!(best_match(&available, "ja").is_none());
        assert!(best_match(&available, "   ").is_none());
    }

    #[test]
    fn translate_falls_back_to_the_key_when_nothing_defines_it() {
        assert_eq!(translate("no.such.key").as_ref(), "no.such.key");
    }

    #[test]
    fn translate_returns_english_for_a_known_key() {
        assert_eq!(translate("settings.nav.general").as_ref(), "General");
    }

    #[test]
    fn translate_args_substitutes_placeholders() {
        let rendered = t!("settings.language.system_detail", language = "Deutsch");
        assert_eq!(rendered.as_ref(), "Detected: Deutsch");
    }

    #[test]
    fn translate_plural_selects_english_forms() {
        assert_eq!(
            tp!("settings.language.missing", 1usize, count = 1).as_ref(),
            "1 string not translated"
        );
        assert_eq!(
            tp!("settings.language.missing", 7usize, count = 7).as_ref(),
            "7 strings not translated"
        );
    }

    #[test]
    fn translate_plural_falls_back_to_the_key_when_undefined() {
        assert_eq!(
            translate_plural("no.such.plural", 2, &[]).as_ref(),
            "no.such.plural.other"
        );
    }

    #[test]
    fn missing_count_of_english_is_zero() {
        assert_eq!(missing_count_of(ENGLISH_ID), 0);
    }
}
