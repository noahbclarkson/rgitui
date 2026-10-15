//! Localization for rgitui.
//!
//! # Using a translated string
//!
//! ```ignore
//! use rgitui_i18n::{t, tp};
//!
//! Label::new(t!("settings.language.label"));
//! Label::new(t!("settings.language.system_detail", language = "English"));
//! Label::new(tp!("settings.language.missing", missing));
//! ```
//!
//! [`t!`] returns a [`SharedString`], which every `rgitui_ui` component already
//! accepts, so translating a call site is a one-line change. Text produced away
//! from the UI — a git error built on a background thread — is a [`Message`]
//! from [`msg!`], translated where it is finally shown. Keys are checked
//! at compile time: a key missing from `en.rs`, or named arguments that do not
//! match the English message's placeholders, fail the build. [`key!`] applies
//! the same check to a key stored as data and translated later with
//! [`translate`].
//!
//! # How lookup works
//!
//! The catalogue lives in a process-wide store rather than a GPUI global, so
//! `t!` works from background threads. Lookup order is active locale, then
//! English.
//!
//! [`set_language`] refreshes every window, so anything translated in `render`
//! updates on its own, as do input placeholders given a key with
//! `TextInput::set_placeholder_message`. Text cached in an entity has to be
//! rebuilt when the language changes; observe the [`ActiveLanguage`] global for
//! that.

mod catalog;
mod en;
mod keys;
mod loader;
mod message;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use gpui::{App, Global, SharedString};
use parking_lot::RwLock;

pub use catalog::{Locale, PluralCategory, PluralRule};
pub use en::{ENGLISH_ID, ENGLISH_MESSAGES, ENGLISH_NAME};
#[doc(hidden)]
pub use keys::{args_match, is_known_key, is_known_plural, plural_args_match};
pub use loader::user_locales_dir;
pub use message::{IntoMessageArg, Message, MessageArg, MessageError};

use catalog::{interpolate, plural_category_of};
use loader::{add_user_locale, load_embedded_locales, read_user_locales, upsert_locale};

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

/// The locale rendering the UI, replaced as a GPUI global on every
/// [`set_language`]. Observe it to rebuild translated text that is stored
/// outside `render`.
pub struct ActiveLanguage(Arc<Locale>);

impl ActiveLanguage {
    pub fn locale(&self) -> &Arc<Locale> {
        &self.0
    }
}

impl Global for ActiveLanguage {}

struct Store {
    english: Arc<Locale>,
    active: Arc<Locale>,
    available: Vec<Arc<Locale>>,
    /// The user's stored preference: [`SYSTEM_LANGUAGE`] or a locale id. Kept
    /// distinct from `active` so the picker can show "Match system language" as
    /// selected rather than whichever language that resolved to.
    selection: String,
    /// The OS language, read once at [`init`].
    system: Option<String>,
    /// Untranslated message counts per locale id, computed once at [`init`].
    missing: HashMap<String, usize>,
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
            system: None,
            missing: HashMap::new(),
        }
    }

    /// The locale `selection` resolves to: the best match for the OS language
    /// when following the system, otherwise for the stored id, and English
    /// when nothing matches.
    fn resolve(&self, selection: &str) -> Arc<Locale> {
        if selection == SYSTEM_LANGUAGE {
            return self
                .system
                .as_deref()
                .and_then(|id| best_match(&self.available, id))
                .unwrap_or_else(|| self.english.clone());
        }
        best_match(&self.available, selection).unwrap_or_else(|| {
            log::warn!("unknown language {selection:?} in settings — falling back to English");
            self.english.clone()
        })
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
        for (path, file) in read_user_locales(&dir) {
            let locale = add_user_locale(&mut available, file);
            log::info!("loaded custom locale {} from {}", locale.id, path.display());
            let unknown = locale.unknown_keys(&english);
            if !unknown.is_empty() {
                log::warn!(
                    "{} defines keys English does not, which are ignored: {}",
                    path.display(),
                    unknown.join(", ")
                );
            }
        }
    }

    // English first, then by the name shown in the picker.
    available.sort_by(|a, b| match (a.id == ENGLISH_ID, b.id == ENGLISH_ID) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.english_name.cmp(&b.english_name),
    });

    let missing = available
        .iter()
        .map(|locale| (locale.id.clone(), count_missing(locale, &english)))
        .collect();

    {
        let mut store = store().write();
        store.english = english.clone();
        store.active = english;
        store.available = available;
        store.system = sys_locale::get_locale().filter(|id| !id.trim().is_empty());
        store.missing = missing;
    }

    set_language(cx, selection);
}

/// Apply a language selection and refresh every window, returning the locale
/// that was actually activated.
pub fn set_language(cx: &mut App, selection: &str) -> Arc<Locale> {
    let resolved = {
        let mut store = store().write();
        let resolved = store.resolve(selection);
        store.active = resolved.clone();
        store.selection = selection.to_string();
        resolved
    };
    log::info!("UI language: {} (selection {:?})", resolved.id, selection);
    cx.set_global(ActiveLanguage(resolved.clone()));
    cx.refresh_windows();
    resolved
}

/// A language tag broken into the parts matching cares about. Accepts BCP-47
/// (`zh-Hant-TW`) and POSIX (`zh_TW.UTF-8@pinyin`) spellings.
struct Tag {
    language: String,
    script: Option<String>,
    region: Option<String>,
}

impl Tag {
    fn parse(tag: &str) -> Option<Self> {
        let tag = tag.trim();
        let tag = tag.split(['.', '@']).next().unwrap_or(tag);
        let mut parts = tag.split(['-', '_']).filter(|p| !p.is_empty());
        let language = parts.next()?.to_ascii_lowercase();
        let mut script = None;
        let mut region = None;
        for part in parts {
            if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                script.get_or_insert_with(|| part.to_ascii_lowercase());
            } else if (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
                || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
            {
                region.get_or_insert_with(|| part.to_ascii_uppercase());
            }
        }
        Some(Self {
            language,
            script,
            region,
        })
    }

    /// The script, stated or implied. Chinese is the case that matters: a
    /// Taiwanese, Hong Kong or Macau system means Traditional characters, and
    /// a Simplified catalogue is no substitute for them.
    fn script(&self) -> Option<&str> {
        if let Some(script) = self.script.as_deref() {
            return Some(script);
        }
        if self.language == "zh" {
            return Some(match self.region.as_deref() {
                Some("TW" | "HK" | "MO") => "hant",
                _ => "hans",
            });
        }
        None
    }

    fn same_tag(&self, other: &Tag) -> bool {
        self.language == other.language
            && self.script == other.script
            && self.region == other.region
    }
}

/// Find the locale that best matches `wanted`: the same tag if there is one,
/// otherwise one with the same language written in the same script — so a
/// system locale of `zh-Hans-SG` selects the shipped `zh-CN`, while `zh-TW`
/// does not.
fn best_match(available: &[Arc<Locale>], wanted: &str) -> Option<Arc<Locale>> {
    let wanted = Tag::parse(wanted)?;
    let tags: Vec<(Option<Tag>, &Arc<Locale>)> = available
        .iter()
        .map(|locale| (Tag::parse(&locale.id), locale))
        .collect();

    if let Some((_, exact)) = tags
        .iter()
        .find(|(tag, _)| tag.as_ref().is_some_and(|tag| tag.same_tag(&wanted)))
    {
        return Some((*exact).clone());
    }

    tags.iter()
        .find(|(tag, _)| {
            tag.as_ref().is_some_and(|tag| {
                tag.language == wanted.language && tag.script() == wanted.script()
            })
        })
        .map(|(_, locale)| (*locale).clone())
}

/// The script a locale id is written in, stated or implied, in lowercase:
/// `zh-CN` is `hans`, `zh-TW` is `hant`, `sr-Latn` is `latn`. `None` when the
/// tag neither states nor implies one.
pub fn locale_script(id: &str) -> Option<String> {
    Tag::parse(id).and_then(|tag| tag.script().map(str::to_string))
}

/// The language the OS reports, e.g. `en-US`. `None` before [`init`] or if it
/// could not be detected.
pub fn system_locale_id() -> Option<String> {
    store().read().system.clone()
}

/// The locale "Match system language" resolves to, if the OS language has a
/// translation (English counts).
pub fn system_locale_match() -> Option<Arc<Locale>> {
    let store = store().read();
    store
        .system
        .as_deref()
        .and_then(|id| best_match(&store.available, id))
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

/// Number of English messages `locale_id` leaves untranslated.
pub fn missing_count_of(locale_id: &str) -> usize {
    store().read().missing.get(locale_id).copied().unwrap_or(0)
}

/// English messages `locale` does not translate. Plural forms the locale's
/// rule never selects are not counted: a Chinese catalogue has no `.one`
/// forms, and that is complete, not one string short.
fn count_missing(locale: &Locale, english: &Locale) -> usize {
    english
        .messages
        .keys()
        .filter(|key| {
            let needed = plural_category_of(key)
                .is_none_or(|category| locale.plural_rule.categories().contains(&category));
            needed && locale.get(key).is_none()
        })
        .count()
}

/// Look up `key`, falling back to English and then to the key itself. Prefer
/// [`t!`], which checks the key at compile time; call this directly only for a
/// key held as data, ideally produced by [`key!`].
pub fn translate(key: &str) -> SharedString {
    let store = store().read();
    if let Some(message) = store.active.get(key) {
        return message.clone();
    }
    match store.english.get(key) {
        Some(message) => message.clone(),
        None => {
            log::warn!("missing translation key: {key}");
            SharedString::from(key.to_string())
        }
    }
}

/// Look up `key` and substitute its `{name}` placeholders.
pub fn translate_args(key: &str, args: &[(&str, String)]) -> SharedString {
    let message = translate(key);
    if args.is_empty() {
        // The common case, every `t!` without arguments: the catalogue's own
        // string, which for English is a `&'static str` and costs no copy.
        return message;
    }
    SharedString::from(interpolate(message.as_ref(), args))
}

/// Look up the plural form of `key` for `count` under the active locale's
/// plural rule, then substitute placeholders.
///
/// `key` is the stem: `settings.language.missing` resolves to
/// `settings.language.missing.one` or `…other`. `count` is substituted for
/// `{count}`; `args` supplies any other placeholders.
pub fn translate_plural(key: &str, count: i64, args: &[(&str, String)]) -> SharedString {
    let mut all_args = Vec::with_capacity(args.len() + 1);
    all_args.push(("count", count.to_string()));
    all_args.extend(args.iter().cloned());

    let store = store().read();
    let category = store.active.plural_rule.select(count);

    let active_key = format!("{key}.{}", category.suffix());
    if let Some(message) = store.active.get(&active_key) {
        return SharedString::from(interpolate(message.as_ref(), &all_args));
    }

    // The English fallback re-selects under English's own rule: a Chinese
    // catalogue that only defines `.other` must not send us looking for an
    // English `.other` when the count is 1.
    let english_category = store.english.plural_rule.select(count);
    let english_key = format!("{key}.{}", english_category.suffix());
    match store.english.get(&english_key) {
        Some(message) => SharedString::from(interpolate(message.as_ref(), &all_args)),
        None => {
            log::warn!("missing plural translation key: {english_key}");
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

/// Compile-time checks shared by the translation macros. Each arm expands to a
/// `const` assertion evaluated where the macro is used.
#[doc(hidden)]
#[macro_export]
macro_rules! __i18n_check {
    (key $key:literal) => {
        const _: () = ::core::assert!(
            $crate::is_known_key($key),
            ::core::concat!("unknown translation key \"", $key, "\": add it to crates/rgitui_i18n/src/en.rs"),
        );
    };
    (args $key:literal; $($name:ident),*) => {
        const _: () = ::core::assert!(
            $crate::args_match($key, &[$(::core::stringify!($name)),*]),
            ::core::concat!("the arguments for \"", $key, "\" do not match the placeholders in its English message"),
        );
    };
    (plural $key:literal) => {
        const _: () = ::core::assert!(
            $crate::is_known_plural(::core::concat!($key, ".one"), ::core::concat!($key, ".other")),
            ::core::concat!("unknown plural translation key \"", $key, "\": add \"", $key, ".one\" and \"", $key, ".other\" to crates/rgitui_i18n/src/en.rs"),
        );
    };
    (plural_args $key:literal; $($name:ident),*) => {
        const _: () = ::core::assert!(
            $crate::plural_args_match(
                ::core::concat!($key, ".one"),
                ::core::concat!($key, ".other"),
                &[$(::core::stringify!($name)),*],
            ),
            ::core::concat!("the arguments for \"", $key, "\" do not match the placeholders in its English messages"),
        );
    };
}

/// Translate a message key. The key and the argument names are checked
/// against `en.rs` at compile time.
///
/// ```ignore
/// t!("settings.language.label")
/// t!("settings.language.system_detail", language = "English")
/// ```
#[macro_export]
macro_rules! t {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        $crate::__i18n_check!(key $key);
        $crate::__i18n_check!(args $key; $($name),*);
        $crate::translate_args(
            $key,
            &[$((::core::stringify!($name), ::std::string::ToString::to_string(&$value))),*],
        )
    }};
}

/// Translate a count-dependent message. `$key` is the stem of a `.one`/`.other`
/// pair; the count fills `{count}`, and any other placeholders are passed by
/// name. Checked against `en.rs` at compile time.
///
/// ```ignore
/// tp!("settings.language.missing", missing)
/// ```
#[macro_export]
macro_rules! tp {
    ($key:literal, $count:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        $crate::__i18n_check!(plural $key);
        $crate::__i18n_check!(plural_args $key; $($name),*);
        $crate::translate_plural(
            $key,
            $crate::IntoCount::into_count($count),
            &[$((::core::stringify!($name), ::std::string::ToString::to_string(&$value))),*],
        )
    }};
}

/// A [`Message`] to translate later, where it is shown. Checked like [`t!`];
/// a value may itself be a [`Message`], which is translated along with it.
///
/// ```ignore
/// msg!("git.push.failed", remote = remote_name)
/// ```
#[macro_export]
macro_rules! msg {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        $crate::__i18n_check!(key $key);
        $crate::__i18n_check!(args $key; $($name),*);
        $crate::Message::keyed(
            $key,
            ::std::vec![$((::core::stringify!($name), $crate::IntoMessageArg::into_message_arg($value))),*],
        )
    }};
}

/// The plural counterpart of [`msg!`], checked like [`tp!`].
///
/// ```ignore
/// msgp!("git.stage.files.done", paths.len())
/// ```
#[macro_export]
macro_rules! msgp {
    ($key:literal, $count:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        $crate::__i18n_check!(plural $key);
        $crate::__i18n_check!(plural_args $key; $($name),*);
        $crate::Message::plural(
            $key,
            $crate::IntoCount::into_count($count),
            ::std::vec![$((::core::stringify!($name), $crate::IntoMessageArg::into_message_arg($value))),*],
        )
    }};
}

/// A message key checked at compile time, for keys stored as data and
/// translated later with [`translate`] or [`translate_args`].
///
/// ```ignore
/// const FILTERS: [(&str, Filter); 2] = [(key!("filter.open"), Filter::Open), (key!("filter.closed"), Filter::Closed)];
/// ```
#[macro_export]
macro_rules! key {
    ($key:literal) => {{
        $crate::__i18n_check!(key $key);
        $key
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::parse_locale;
    use std::collections::{BTreeSet, HashSet};
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

    fn placeholders(message: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = message;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            match after.find('}') {
                Some(close) => {
                    found.insert(after[..close].to_string());
                    rest = &after[close + 1..];
                }
                None => break,
            }
        }
        found
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

    /// Every English key is used by some `t!`, `tp!` or `key!` in the
    /// workspace. A key nothing uses is a message deleted from the UI whose
    /// translations would otherwise be maintained forever.
    #[test]
    fn every_english_key_is_used_somewhere() {
        let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut sources = String::new();
        let mut stack = vec![crates_dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)
                .expect("readable crates dir")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_none_or(|name| name != "target") {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("rgitui_i18n/src/en.rs")
                {
                    sources.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
                }
            }
        }

        let unused: Vec<&str> = ENGLISH_MESSAGES
            .iter()
            .map(|(key, _)| *key)
            .filter(|key| {
                let stem = key
                    .strip_suffix(".one")
                    .or_else(|| key.strip_suffix(".other"))
                    .unwrap_or(key);
                !sources.contains(&format!("\"{key}\""))
                    && !sources.contains(&format!("\"{stem}\""))
            })
            .collect();
        assert!(unused.is_empty(), "English keys no code uses: {unused:?}");
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

    /// A translation must use exactly the English placeholders: dropping one
    /// loses a value, and adding or misspelling one renders it literally.
    #[test]
    fn shipped_locale_placeholders_match_english() {
        let english = english_locale();
        for locale in shipped_locales() {
            for (key, translated) in &locale.messages {
                let Some(source) = english.get(key) else {
                    continue;
                };
                assert_eq!(
                    placeholders(translated.as_ref()),
                    placeholders(source.as_ref()),
                    "locale {} has different placeholders from English in key {}",
                    locale.id,
                    key
                );
            }
        }
    }

    #[test]
    fn shipped_locales_only_define_plural_forms_their_rule_selects() {
        for locale in shipped_locales() {
            for key in locale.messages.keys() {
                if let Some(category) = plural_category_of(key) {
                    assert!(
                        locale.plural_rule.categories().contains(&category),
                        "locale {} defines {key}, which its plural rule never selects",
                        locale.id
                    );
                }
            }
        }
    }

    #[test]
    fn best_match_prefers_an_exact_tag() {
        let available = vec![
            Arc::new(english_locale()),
            Arc::new(test_locale("zh-CN", "简体中文")),
            Arc::new(test_locale("zh-TW", "繁體中文")),
        ];
        let matched = best_match(&available, "zh-TW").expect("exact match");
        assert_eq!(matched.id, "zh-TW");
    }

    #[test]
    fn best_match_accepts_the_same_language_in_the_same_script() {
        let available = vec![
            Arc::new(english_locale()),
            Arc::new(test_locale("zh-CN", "简体中文")),
        ];
        for wanted in ["zh-Hans-SG", "zh-SG", "zh", "zh_CN.UTF-8"] {
            let matched = best_match(&available, wanted).expect(wanted);
            assert_eq!(matched.id, "zh-CN", "{wanted}");
        }

        let matched = best_match(&available, "en-GB").expect("same language");
        assert_eq!(matched.id, ENGLISH_ID);
    }

    #[test]
    fn traditional_chinese_does_not_get_simplified() {
        let available = vec![
            Arc::new(english_locale()),
            Arc::new(test_locale("zh-CN", "简体中文")),
        ];
        for wanted in [
            "zh-TW",
            "zh-HK",
            "zh-MO",
            "zh-Hant",
            "zh-Hant-CN",
            "zh_TW.UTF-8",
        ] {
            assert!(best_match(&available, wanted).is_none(), "{wanted}");
        }
    }

    #[test]
    fn locale_script_is_stated_or_implied() {
        assert_eq!(locale_script("zh-CN").as_deref(), Some("hans"));
        assert_eq!(locale_script("zh-TW").as_deref(), Some("hant"));
        assert_eq!(locale_script("sr-Latn-RS").as_deref(), Some("latn"));
        assert_eq!(locale_script("en"), None);
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
    fn a_fully_translated_other_only_locale_has_nothing_missing() {
        let english = english_locale();
        let mut chinese = test_locale("zh-CN", "简体中文");
        for key in english.messages.keys() {
            if plural_category_of(key) != Some(PluralCategory::One) {
                chinese
                    .messages
                    .insert(key.clone(), SharedString::from("译"));
            }
        }
        assert_eq!(count_missing(&chinese, &english), 0);

        chinese.messages.remove("settings.language.label");
        assert_eq!(count_missing(&chinese, &english), 1);
    }

    #[test]
    fn a_one_other_locale_needs_both_plural_forms() {
        let english = english_locale();
        let mut german = test_locale("de", "Deutsch");
        german.plural_rule = PluralRule::OneOther;
        for key in english.messages.keys() {
            german.messages.insert(key.clone(), SharedString::from("x"));
        }
        german.messages.remove("settings.language.missing.one");
        assert_eq!(count_missing(&german, &english), 1);
    }

    #[test]
    fn translate_falls_back_to_the_key_when_nothing_defines_it() {
        assert_eq!(translate("no.such.key").as_ref(), "no.such.key");
    }

    #[test]
    fn translate_returns_english_for_a_known_key() {
        assert_eq!(t!("settings.nav.general").as_ref(), "General");
    }

    #[test]
    fn translate_args_substitutes_placeholders() {
        let rendered = t!("settings.language.system_detail", language = "Deutsch");
        assert_eq!(rendered.as_ref(), "Detected: Deutsch");
    }

    #[test]
    fn translate_plural_selects_english_forms_and_fills_the_count() {
        assert_eq!(
            tp!("settings.language.missing", 1usize).as_ref(),
            "1 string not translated"
        );
        assert_eq!(
            tp!("settings.language.missing", 7usize).as_ref(),
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
    fn key_macro_yields_the_key() {
        const KEY: &str = key!("settings.nav.general");
        assert_eq!(KEY, "settings.nav.general");
        assert_eq!(translate(KEY).as_ref(), "General");
    }
}
