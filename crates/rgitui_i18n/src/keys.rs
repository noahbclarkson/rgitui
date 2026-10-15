//! Compile-time checks behind [`crate::t!`], [`crate::tp!`] and [`crate::key!`].
//!
//! Every function here is `const` so the macros can evaluate it in a `const`
//! item at each call site: a key missing from [`ENGLISH_MESSAGES`], or named
//! arguments that do not match the English message's `{placeholders}`, fail
//! the build at the line that is wrong rather than rendering the raw key.
//!
//! Lookup is a binary search, which [`assert_sorted`] makes safe by rejecting
//! an unsorted or duplicated table when `en.rs` is compiled.

use std::cmp::Ordering;

use crate::en::ENGLISH_MESSAGES;

const fn compare(a: &[u8], b: &[u8]) -> Ordering {
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a[i] < b[i] {
            return Ordering::Less;
        }
        if a[i] > b[i] {
            return Ordering::Greater;
        }
        i += 1;
    }
    if a.len() < b.len() {
        Ordering::Less
    } else if a.len() > b.len() {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Whether `a[start..end]` equals `b`.
const fn range_equals(a: &[u8], start: usize, end: usize, b: &[u8]) -> bool {
    if end - start != b.len() {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if a[start + i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn find(key: &[u8]) -> Option<&'static str> {
    let mut low = 0;
    let mut high = ENGLISH_MESSAGES.len();
    while low < high {
        let mid = low + (high - low) / 2;
        match compare(ENGLISH_MESSAGES[mid].0.as_bytes(), key) {
            Ordering::Less => low = mid + 1,
            Ordering::Greater => high = mid,
            Ordering::Equal => return Some(ENGLISH_MESSAGES[mid].1),
        }
    }
    None
}

/// Whether `key` is defined in [`ENGLISH_MESSAGES`].
pub const fn is_known_key(key: &str) -> bool {
    find(key.as_bytes()).is_some()
}

/// Whether `key` has both plural forms, `key.one` and `key.other`.
pub const fn is_known_plural(one: &str, other: &str) -> bool {
    is_known_key(one) && is_known_key(other)
}

/// Whether `message` contains `{name}`.
const fn has_placeholder(message: &[u8], name: &[u8]) -> bool {
    let mut i = 0;
    while i < message.len() {
        if message[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < message.len() && message[end] != b'}' {
                end += 1;
            }
            if end == message.len() {
                return false;
            }
            if range_equals(message, start, end, name) {
                return true;
            }
            i = end;
        }
        i += 1;
    }
    false
}

/// Whether every `{placeholder}` in `message` is one of `names`, or is
/// `{count}` when the caller supplies the count implicitly.
const fn placeholders_covered(message: &[u8], names: &[&str], count_supplied: bool) -> bool {
    let mut i = 0;
    while i < message.len() {
        if message[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < message.len() && message[end] != b'}' {
                end += 1;
            }
            if end == message.len() {
                return true;
            }
            let mut found = count_supplied && range_equals(message, start, end, b"count");
            let mut n = 0;
            while n < names.len() {
                if range_equals(message, start, end, names[n].as_bytes()) {
                    found = true;
                }
                n += 1;
            }
            if !found {
                return false;
            }
            i = end;
        }
        i += 1;
    }
    true
}

/// Whether `names` are exactly the placeholders of `key`'s English message:
/// each name is used, and each placeholder is supplied. An unknown key passes,
/// since [`is_known_key`] already reports it.
pub const fn args_match(key: &str, names: &[&str]) -> bool {
    let Some(message) = find(key.as_bytes()) else {
        return true;
    };
    let message = message.as_bytes();
    let mut n = 0;
    while n < names.len() {
        if !has_placeholder(message, names[n].as_bytes()) {
            return false;
        }
        n += 1;
    }
    placeholders_covered(message, names, false)
}

/// The plural counterpart of [`args_match`]. `count` is always supplied by
/// [`crate::tp!`], so `names` excludes it. Each name must appear in at least
/// one form, and each placeholder in either form must be supplied. Unknown
/// keys pass, since [`is_known_plural`] already reports them.
pub const fn plural_args_match(one: &str, other: &str, names: &[&str]) -> bool {
    let (Some(one), Some(other)) = (find(one.as_bytes()), find(other.as_bytes())) else {
        return true;
    };
    let (one, other) = (one.as_bytes(), other.as_bytes());
    let mut n = 0;
    while n < names.len() {
        let name = names[n].as_bytes();
        if range_equals(name, 0, name.len(), b"count") {
            return false;
        }
        if !has_placeholder(one, name) && !has_placeholder(other, name) {
            return false;
        }
        n += 1;
    }
    placeholders_covered(one, names, true) && placeholders_covered(other, names, true)
}

/// Rejects a table that is out of order or repeats a key, naming the first
/// offending key. Binary search in [`find`] depends on it.
pub(crate) const fn assert_sorted(table: &[(&str, &str)]) {
    let mut i = 1;
    while i < table.len() {
        if !matches!(
            compare(table[i - 1].0.as_bytes(), table[i].0.as_bytes()),
            Ordering::Less
        ) {
            panic!("{}", table[i].0);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_are_found_and_unknown_ones_are_not() {
        assert!(is_known_key("settings.nav.general"));
        assert!(!is_known_key("settings.nav.genral"));
        assert!(!is_known_key(""));
    }

    #[test]
    fn every_english_key_is_found_by_the_binary_search() {
        for (key, _) in ENGLISH_MESSAGES {
            assert!(is_known_key(key), "{key} not found");
        }
    }

    #[test]
    fn placeholder_detection_matches_whole_names() {
        assert!(has_placeholder(b"Detected: {language}", b"language"));
        assert!(!has_placeholder(b"Detected: {language}", b"lang"));
        assert!(!has_placeholder(b"Detected: {language", b"language"));
    }

    #[test]
    fn args_must_cover_and_only_cover_the_placeholders() {
        assert!(args_match("settings.language.system_detail", &["language"]));
        assert!(!args_match("settings.language.system_detail", &[]));
        assert!(!args_match(
            "settings.language.system_detail",
            &["language", "extra"]
        ));
        assert!(!args_match("settings.language.system_detail", &["lang"]));
        assert!(args_match("settings.nav.general", &[]));
        assert!(args_match("no.such.key", &["left", "to", "is_known_key"]));
    }

    #[test]
    fn plural_args_get_count_for_free() {
        assert!(plural_args_match(
            "settings.language.missing.one",
            "settings.language.missing.other",
            &[]
        ));
        assert!(!plural_args_match(
            "settings.language.missing.one",
            "settings.language.missing.other",
            &["count"]
        ));
        assert!(!plural_args_match(
            "settings.language.missing.one",
            "settings.language.missing.other",
            &["unused"]
        ));
    }
}
