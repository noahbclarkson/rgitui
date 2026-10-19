use std::collections::HashMap;

use gpui::SharedString;

/// CLDR plural categories used for message selection.
///
/// Only the categories rgitui's shipped locales need are modelled. Adding a
/// language with richer plural rules (Polish, Arabic, Russian) means adding a
/// [`PluralRule`] variant, not changing call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluralCategory {
    One,
    Other,
}

impl PluralCategory {
    /// Suffix appended to a message key to select this category, e.g. the key
    /// `commit.count` resolves to `commit.count.one` or `commit.count.other`.
    pub fn suffix(self) -> &'static str {
        match self {
            PluralCategory::One => "one",
            PluralCategory::Other => "other",
        }
    }
}

/// How a locale maps a count onto a [`PluralCategory`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluralRule {
    /// English, German, Spanish, Dutch: exactly 1 is singular.
    #[default]
    OneOther,
    /// French, Brazilian Portuguese: 0 and 1 both take the singular form.
    ZeroOneOther,
    /// Chinese, Japanese, Korean, Vietnamese, Thai: no plural inflection.
    OtherOnly,
}

impl PluralRule {
    pub fn select(self, count: i64) -> PluralCategory {
        match self {
            PluralRule::OneOther => {
                if count == 1 {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }
            PluralRule::ZeroOneOther => {
                if count == 0 || count == 1 {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }
            PluralRule::OtherOnly => PluralCategory::Other,
        }
    }

    /// The categories [`PluralRule::select`] can return, which are the plural
    /// forms a locale using this rule has to define.
    pub fn categories(self) -> &'static [PluralCategory] {
        match self {
            PluralRule::OneOther | PluralRule::ZeroOneOther => {
                &[PluralCategory::One, PluralCategory::Other]
            }
            PluralRule::OtherOnly => &[PluralCategory::Other],
        }
    }

    /// Parse the `plural_rule` field of a locale JSON file.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "one_other" => Some(PluralRule::OneOther),
            "zero_one_other" => Some(PluralRule::ZeroOneOther),
            "other_only" => Some(PluralRule::OtherOnly),
            _ => None,
        }
    }
}

/// The plural category a key selects, if it is one form of a plural message:
/// `files.count.one` is [`PluralCategory::One`]. English declares every plural
/// message as a `.one`/`.other` pair, so the suffix is unambiguous.
pub fn plural_category_of(key: &str) -> Option<PluralCategory> {
    if key.ends_with(".one") {
        Some(PluralCategory::One)
    } else if key.ends_with(".other") {
        Some(PluralCategory::Other)
    } else {
        None
    }
}

/// A single language's message catalogue.
#[derive(Debug, Clone)]
pub struct Locale {
    /// BCP-47 identifier, e.g. `en`, `zh-CN`, `pt-BR`.
    pub id: String,
    /// Name of the language in English, for logs and bug reports.
    pub english_name: String,
    /// Name of the language as its own speakers write it — this is what the
    /// language picker shows, so a user who cannot read the current UI language
    /// can still find their own.
    pub native_name: String,
    pub plural_rule: PluralRule,
    pub messages: HashMap<String, SharedString>,
}

impl Locale {
    pub fn get(&self, key: &str) -> Option<&SharedString> {
        self.messages.get(key)
    }

    /// Keys this locale defines that the reference locale does not. These are
    /// almost always typos or messages deleted from the source language, and
    /// are rejected for shipped locales by the `shipped_locales_have_no_unknown_keys`
    /// test and logged for user locales.
    pub fn unknown_keys(&self, reference: &Locale) -> Vec<&str> {
        let mut keys: Vec<&str> = self
            .messages
            .keys()
            .filter(|key| !reference.messages.contains_key(*key))
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }
}

/// Substitute `{name}` placeholders in `template` with the supplied arguments.
///
/// An unmatched placeholder is left verbatim rather than blanked, so a bad
/// translation shows `{count}` in the UI — visibly wrong and reportable —
/// instead of silently losing the number.
pub fn interpolate(template: &str, args: &[(&str, String)]) -> String {
    if !template.contains('{') {
        return template.to_string();
    }

    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];

        let Some(close) = after_open.find('}') else {
            // Unterminated brace: emit the remainder untouched.
            out.push_str(&rest[open..]);
            return out;
        };

        let name = &after_open[..close];
        match args.iter().find(|(arg_name, _)| *arg_name == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after_open[close + 1..];
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale_with(id: &str, pairs: &[(&str, &str)]) -> Locale {
        Locale {
            id: id.to_string(),
            english_name: id.to_string(),
            native_name: id.to_string(),
            plural_rule: PluralRule::OneOther,
            messages: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), SharedString::from(v.to_string())))
                .collect(),
        }
    }

    #[test]
    fn one_other_rule_treats_only_one_as_singular() {
        assert_eq!(PluralRule::OneOther.select(0), PluralCategory::Other);
        assert_eq!(PluralRule::OneOther.select(1), PluralCategory::One);
        assert_eq!(PluralRule::OneOther.select(2), PluralCategory::Other);
    }

    #[test]
    fn zero_one_other_rule_treats_zero_as_singular() {
        assert_eq!(PluralRule::ZeroOneOther.select(0), PluralCategory::One);
        assert_eq!(PluralRule::ZeroOneOther.select(1), PluralCategory::One);
        assert_eq!(PluralRule::ZeroOneOther.select(2), PluralCategory::Other);
    }

    #[test]
    fn other_only_rule_never_selects_singular() {
        for count in [0, 1, 2, 100] {
            assert_eq!(PluralRule::OtherOnly.select(count), PluralCategory::Other);
        }
    }

    #[test]
    fn plural_rule_names_parse_and_unknown_ones_are_rejected() {
        assert_eq!(
            PluralRule::from_name("one_other"),
            Some(PluralRule::OneOther)
        );
        assert_eq!(
            PluralRule::from_name("zero_one_other"),
            Some(PluralRule::ZeroOneOther)
        );
        assert_eq!(
            PluralRule::from_name("other_only"),
            Some(PluralRule::OtherOnly)
        );
        assert_eq!(PluralRule::from_name("nonsense"), None);
    }

    #[test]
    fn other_only_needs_no_singular_form() {
        assert_eq!(PluralRule::OtherOnly.categories(), &[PluralCategory::Other]);
        assert_eq!(
            PluralRule::OneOther.categories(),
            &[PluralCategory::One, PluralCategory::Other]
        );
    }

    #[test]
    fn plural_keys_are_recognised_by_suffix() {
        assert_eq!(plural_category_of("a.b.one"), Some(PluralCategory::One));
        assert_eq!(plural_category_of("a.b.other"), Some(PluralCategory::Other));
        assert_eq!(plural_category_of("a.b.none"), None);
    }

    #[test]
    fn interpolate_substitutes_named_placeholders() {
        let args = [("count", "3".to_string()), ("name", "main".to_string())];
        assert_eq!(
            interpolate("{count} commits on {name}", &args),
            "3 commits on main"
        );
    }

    #[test]
    fn interpolate_leaves_unknown_placeholders_visible() {
        assert_eq!(
            interpolate("{count} of {total}", &[("count", "3".to_string())]),
            "3 of {total}"
        );
    }

    #[test]
    fn interpolate_handles_unterminated_brace() {
        assert_eq!(interpolate("50% {oops", &[]), "50% {oops");
    }

    #[test]
    fn interpolate_returns_template_when_no_placeholders() {
        assert_eq!(interpolate("Appearance", &[]), "Appearance");
    }

    #[test]
    fn unknown_keys_reports_keys_missing_from_reference() {
        let reference = locale_with("en", &[("a", "A")]);
        let typo = locale_with("de", &[("a", "A"), ("typo", "T")]);
        assert_eq!(typo.unknown_keys(&reference), vec!["typo"]);
        assert!(reference.unknown_keys(&reference).is_empty());
    }
}
