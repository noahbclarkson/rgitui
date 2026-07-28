//! The English message catalogue — the source of truth for every translatable
//! string in rgitui.
//!
//! English lives in Rust rather than JSON for the same reason the built-in
//! themes do (see `rgitui_theme::builtin_themes`): it is always available, it
//! cannot drift from an on-disk asset, and a missing translation in any other
//! language can always fall back to it. Translations ship as JSON under
//! `assets/locales/`; see that directory's README for how to add one.
//!
//! Keys are `area.component.element`, sorted alphabetically. Adding a string to
//! the UI means adding it here first — `assets/locales/*.json` files are
//! checked against this table by the tests in `lib.rs`.
//!
//! Messages may contain `{name}` placeholders, filled in by [`crate::t!`].
//! Count-dependent messages are declared as a `.one`/`.other` pair and selected
//! by [`crate::tp!`] using the active locale's plural rule.

pub const ENGLISH_ID: &str = "en";
pub const ENGLISH_NAME: &str = "English";

pub const ENGLISH_MESSAGES: &[(&str, &str)] = &[
    (
        "settings.appearance.mode.auto",
        "Auto",
    ),
    (
        "settings.appearance.mode.dark",
        "Dark",
    ),
    (
        "settings.appearance.mode.description",
        "Choose whether to show light or dark themes, or auto-detect.",
    ),
    (
        "settings.appearance.mode.label",
        "Appearance Mode",
    ),
    (
        "settings.appearance.mode.light",
        "Light",
    ),
    (
        "settings.appearance.subtitle",
        "Customize the look and feel of the application.",
    ),
    (
        "settings.appearance.theme.active",
        "Active",
    ),
    (
        "settings.appearance.theme.description",
        "Select a theme to change all interface colors.",
    ),
    (
        "settings.appearance.theme.label",
        "Color Theme",
    ),
    (
        "settings.appearance.theme_editor.button",
        "Edit Theme",
    ),
    (
        "settings.appearance.theme_editor.description",
        "Edit colors, create custom themes, and export as JSON.",
    ),
    (
        "settings.appearance.theme_editor.label",
        "Custom Theme Editor",
    ),
    (
        "settings.appearance.title",
        "Appearance",
    ),
    (
        "settings.language.complete",
        "Fully translated",
    ),
    (
        "settings.language.description",
        "Choose the language used throughout the interface. Untranslated text falls back to English.",
    ),
    (
        "settings.language.label",
        "Language",
    ),
    (
        "settings.language.missing.one",
        "{count} string not translated",
    ),
    (
        "settings.language.missing.other",
        "{count} strings not translated",
    ),
    (
        "settings.language.system",
        "Match system language",
    ),
    (
        "settings.language.system_detail",
        "Detected: {language}",
    ),
    (
        "settings.language.system_unknown",
        "No system language detected — using English.",
    ),
    (
        "settings.nav.ai",
        "AI",
    ),
    (
        "settings.nav.appearance",
        "Appearance",
    ),
    (
        "settings.nav.auth",
        "Auth",
    ),
    (
        "settings.nav.general",
        "General",
    ),
    (
        "settings.nav.preferences",
        "Preferences",
    ),
];
