//! Humanised keystroke rendering.
//!
//! Turns the keystroke spelling used in the registry and in `keymap.json`
//! (`secondary-shift-r`, `ctrl-k ctrl-o`) into the text shown to the user:
//! `Ctrl+Shift+R` on Windows and Linux, `⌘⇧R` on macOS.
//!
//! Parsing is [`gpui::Keystroke::parse`], so `secondary` resolves to the
//! platform's primary modifier and `shift-g`/`G` normalise the same way the
//! keymap does — the display can therefore never disagree with what gpui
//! actually matches. Only the final spelling is ours: gpui's own [`Display`]
//! renders `ctrl-shift-R`, which is the keymap syntax rather than a label.
//!
//! [`KeystrokeStyle`] is passed in rather than read from `cfg!`, so both
//! spellings are unit-testable on any platform.
//!
//! [`Display`]: std::fmt::Display

use gpui::{Keystroke, Modifiers};

/// How a keystroke is spelled for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystrokeStyle {
    /// `Ctrl+Shift+R` — Windows and Linux.
    Words,
    /// `⌘⇧R` — macOS, where modifier glyphs are the platform convention.
    Symbols,
}

impl KeystrokeStyle {
    /// The spelling this platform's users expect.
    pub const fn platform() -> Self {
        if cfg!(target_os = "macos") {
            KeystrokeStyle::Symbols
        } else {
            KeystrokeStyle::Words
        }
    }
}

/// Separates the keystrokes of a chord, e.g. `Ctrl+K Ctrl+O`.
const CHORD_SEPARATOR: &str = " ";

/// Separates alternative bindings for one command, e.g. `Ctrl+Shift+S or Ctrl+U`.
pub const BINDING_SEPARATOR: &str = " or ";

/// Shown in place of a keystroke for a command that has no binding.
pub const UNBOUND: &str = "unbound";

/// Renders one keystroke, e.g. `secondary-shift-r` → `Ctrl+Shift+R`.
///
/// Returns `None` when the keystroke does not parse; the loader reports those
/// separately and they are never applied, so there is nothing to display.
pub fn humanize_keystroke(source: &str, style: KeystrokeStyle) -> Option<String> {
    let keystroke = Keystroke::parse(source).ok()?;
    Some(render(&keystroke.modifiers, &keystroke.key, style))
}

/// Renders a whitespace-separated keystroke sequence, e.g. `ctrl-k ctrl-o` →
/// `Ctrl+K Ctrl+O`.
///
/// Returns `None` when the sequence is empty or any keystroke in it is
/// unparseable — a chord is all-or-nothing, since half of one is meaningless.
pub fn humanize_sequence(keystrokes: &str, style: KeystrokeStyle) -> Option<String> {
    let rendered: Option<Vec<String>> = keystrokes
        .split_whitespace()
        .map(|keystroke| humanize_keystroke(keystroke, style))
        .collect();
    let rendered = rendered?;
    (!rendered.is_empty()).then(|| rendered.join(CHORD_SEPARATOR))
}

/// Joins the alternative bindings of one command, e.g. `UnstageAll` →
/// `Ctrl+Shift+S or Ctrl+U`.
///
/// Duplicates are collapsed: one command often binds the same keystroke in two
/// contexts, which is one thing to press and so one thing to show.
pub fn join_bindings<'a>(displays: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut unique: Vec<&str> = Vec::new();
    for display in displays {
        if !unique.contains(&display) {
            unique.push(display);
        }
    }
    (!unique.is_empty()).then(|| unique.join(BINDING_SEPARATOR))
}

/// Spells out modifiers followed by the key.
fn render(modifiers: &Modifiers, key: &str, style: KeystrokeStyle) -> String {
    let mut out = String::new();
    match style {
        // Word order follows `Keystroke::unparse`, so the label reads in the
        // same order as the keymap entry it came from.
        KeystrokeStyle::Words => {
            if modifiers.function {
                out.push_str("Fn+");
            }
            if modifiers.control {
                out.push_str("Ctrl+");
            }
            if modifiers.alt {
                out.push_str("Alt+");
            }
            if modifiers.platform {
                out.push_str(if cfg!(target_os = "windows") {
                    "Win+"
                } else {
                    "Super+"
                });
            }
            if modifiers.shift {
                out.push_str("Shift+");
            }
        }
        // The glyphs and their order are gpui's, so a label matches what the
        // rest of the platform shows in its menus.
        KeystrokeStyle::Symbols => {
            if modifiers.function {
                out.push_str("fn");
            }
            if modifiers.control {
                out.push('^');
            }
            if modifiers.alt {
                out.push('⌥');
            }
            if modifiers.platform {
                out.push('⌘');
            }
            if modifiers.shift {
                out.push('⇧');
            }
        }
    }
    out.push_str(&render_key(key, style));
    out
}

/// Spells out the key itself.
///
/// A single character is upper-cased — the keymap stores `r` for what the user
/// sees printed on the key as `R`, with shift tracked as a modifier.
fn render_key(key: &str, style: KeystrokeStyle) -> String {
    if let Some(named) = named_key(key, style) {
        return named.to_owned();
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(single), None) => single.to_uppercase().collect(),
        // `f5`, or a key name this build does not know: show it verbatim rather
        // than inventing a spelling.
        _ => title_case(key),
    }
}

/// The display name of a key gpui spells with a word.
///
/// macOS shows glyphs for these; elsewhere they get conventional capitalisation.
fn named_key(key: &str, style: KeystrokeStyle) -> Option<&'static str> {
    let symbols = matches!(style, KeystrokeStyle::Symbols);
    Some(match key {
        "enter" => {
            if symbols {
                "↩"
            } else {
                "Enter"
            }
        }
        "escape" => {
            if symbols {
                "⎋"
            } else {
                "Esc"
            }
        }
        "tab" => {
            if symbols {
                "⇥"
            } else {
                "Tab"
            }
        }
        "space" => {
            if symbols {
                "␣"
            } else {
                "Space"
            }
        }
        "backspace" => {
            if symbols {
                "⌫"
            } else {
                "Backspace"
            }
        }
        "delete" => {
            if symbols {
                "⌦"
            } else {
                "Delete"
            }
        }
        "up" => {
            if symbols {
                "↑"
            } else {
                "Up"
            }
        }
        "down" => {
            if symbols {
                "↓"
            } else {
                "Down"
            }
        }
        "left" => {
            if symbols {
                "←"
            } else {
                "Left"
            }
        }
        "right" => {
            if symbols {
                "→"
            } else {
                "Right"
            }
        }
        "home" => {
            if symbols {
                "↖"
            } else {
                "Home"
            }
        }
        "end" => {
            if symbols {
                "↘"
            } else {
                "End"
            }
        }
        "pageup" => {
            if symbols {
                "⇞"
            } else {
                "PageUp"
            }
        }
        "pagedown" => {
            if symbols {
                "⇟"
            } else {
                "PageDown"
            }
        }
        "insert" => "Insert",
        // A modifier bound as the key in its own right.
        "shift" => {
            if symbols {
                "⇧"
            } else {
                "Shift"
            }
        }
        "control" => {
            if symbols {
                "^"
            } else {
                "Ctrl"
            }
        }
        "alt" => {
            if symbols {
                "⌥"
            } else {
                "Alt"
            }
        }
        "platform" => {
            if symbols {
                "⌘"
            } else if cfg!(target_os = "windows") {
                "Win"
            } else {
                "Super"
            }
        }
        "function" => {
            if symbols {
                "fn"
            } else {
                "Fn"
            }
        }
        _ => return None,
    })
}

/// Upper-cases the first character, leaving the rest — `f5` → `F5`.
fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_primary_modifier_follows_the_platform() {
        assert_eq!(
            humanize_keystroke("secondary-shift-r", KeystrokeStyle::Words).as_deref(),
            Some("Ctrl+Shift+R")
        );
        assert_eq!(
            humanize_keystroke("secondary-shift-r", KeystrokeStyle::Symbols).as_deref(),
            // `secondary` is `ctrl` off macOS, which is `^` in the glyph style.
            Some(if cfg!(target_os = "macos") {
                "⌘⇧R"
            } else {
                "^⇧R"
            })
        );
    }

    #[test]
    fn an_explicit_platform_modifier_renders_the_same_either_way() {
        assert_eq!(
            humanize_keystroke("cmd-shift-r", KeystrokeStyle::Symbols).as_deref(),
            Some("⌘⇧R")
        );
    }

    #[test]
    fn named_keys_get_conventional_names() {
        for (source, words) in [
            ("secondary-enter", "Ctrl+Enter"),
            ("escape", "Esc"),
            ("f5", "F5"),
            ("shift-tab", "Shift+Tab"),
            ("secondary-up", "Ctrl+Up"),
            ("alt-5", "Alt+5"),
            ("space", "Space"),
            ("delete", "Delete"),
            ("secondary-,", "Ctrl+,"),
            ("secondary-[", "Ctrl+["),
        ] {
            assert_eq!(
                humanize_keystroke(source, KeystrokeStyle::Words).as_deref(),
                Some(words),
                "{source}"
            );
        }
    }

    /// `?` and `G` reach gpui as shift plus a base key; the label has to show
    /// the character the user actually types.
    #[test]
    fn shifted_characters_keep_the_shift_modifier_visible() {
        assert_eq!(
            humanize_keystroke("?", KeystrokeStyle::Words).as_deref(),
            Some("?")
        );
        assert_eq!(
            humanize_keystroke("shift-g", KeystrokeStyle::Words).as_deref(),
            Some("Shift+G")
        );
        // `G` parses to shift-g, so both spellings render identically.
        assert_eq!(
            humanize_keystroke("G", KeystrokeStyle::Words),
            humanize_keystroke("shift-g", KeystrokeStyle::Words)
        );
    }

    #[test]
    fn a_bare_letter_is_upper_cased() {
        assert_eq!(
            humanize_keystroke("j", KeystrokeStyle::Words).as_deref(),
            Some("J")
        );
        assert_eq!(
            humanize_keystroke("j", KeystrokeStyle::Symbols).as_deref(),
            Some("J")
        );
    }

    #[test]
    fn a_chord_renders_every_keystroke() {
        assert_eq!(
            humanize_sequence("ctrl-k ctrl-o", KeystrokeStyle::Words).as_deref(),
            Some("Ctrl+K Ctrl+O")
        );
        assert_eq!(
            humanize_sequence("ctrl-k ctrl-o", KeystrokeStyle::Symbols).as_deref(),
            Some("^K ^O")
        );
    }

    #[test]
    fn an_unparseable_keystroke_has_no_label() {
        assert_eq!(humanize_keystroke("ctrl-a-b", KeystrokeStyle::Words), None);
        assert_eq!(humanize_sequence("", KeystrokeStyle::Words), None);
        // One bad keystroke discards the whole chord.
        assert_eq!(
            humanize_sequence("ctrl-k ctrl-a-b", KeystrokeStyle::Words),
            None
        );
    }

    #[test]
    fn alternative_bindings_are_joined_and_deduplicated() {
        assert_eq!(
            join_bindings(["Ctrl+Shift+S", "Ctrl+U"]).as_deref(),
            Some("Ctrl+Shift+S or Ctrl+U")
        );
        // `down` and `j` in two contexts is still one thing to press.
        assert_eq!(join_bindings(["Down", "Down"]).as_deref(), Some("Down"));
        assert_eq!(join_bindings(std::iter::empty()), None);
    }

    /// The source files that must show shortcuts the keymap decided, not
    /// shortcuts a developer typed. Each is scanned by
    /// [`no_surface_hardcodes_a_chord`]; add a file here when you route a new
    /// shortcut display through [`super::shortcut`].
    const SURFACES: &[(&str, &str)] = &[
        ("shortcuts_help.rs", include_str!("../shortcuts_help.rs")),
        ("command_palette.rs", include_str!("../command_palette.rs")),
        (
            "settings_window/view.rs",
            include_str!("../settings_window/view.rs"),
        ),
        ("toolbar.rs", include_str!("../toolbar.rs")),
        ("title_bar.rs", include_str!("../title_bar.rs")),
        (
            "workspace/layout.rs",
            include_str!("../workspace/layout.rs"),
        ),
        (
            "workspace/commands.rs",
            include_str!("../workspace/commands.rs"),
        ),
    ];

    /// Spellings that only ever appear in a hand-written shortcut label.
    ///
    /// A modifier joined to something with `+` or `-` cannot occur in Rust code
    /// outside a string, so this needs no quote handling. Built at runtime so
    /// this test's own source does not trip it.
    ///
    /// It does not catch a bare `"?"` or `"j / k"` — a single character is
    /// indistinguishable from ordinary text — so it is a floor, not a proof.
    fn chord_needles() -> Vec<String> {
        ["Ctrl", "Cmd", "Alt", "Shift", "Win", "Super"]
            .into_iter()
            .flat_map(|modifier| ["+", "-"].map(move |joiner| format!("{modifier}{joiner}")))
            .chain(["⌘".to_owned(), "⌥".to_owned(), "⇧".to_owned()])
            .collect()
    }

    /// The `Ctrl+Shift+F`-for-Fetch drift happened because a shortcut label was a
    /// literal in a list nobody re-checked. Deleting the literals fixed it once;
    /// this test is what stops the next one being written, by failing the build if
    /// a user-facing surface spells a chord out again instead of asking the
    /// keymap.
    ///
    /// Doc comments and the test modules are skipped — prose may name a chord,
    /// and an assertion has to spell out what it expects.
    #[test]
    fn no_surface_hardcodes_a_chord() {
        let needles = chord_needles();
        for (name, source) in SURFACES {
            let code = source
                .split_once("\n#[cfg(test)]")
                .map_or(*source, |(before, _)| before);
            for (number, line) in code.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for needle in &needles {
                    assert!(
                        !line.contains(needle.as_str()),
                        "{name}:{} spells a keystroke out instead of reading it from the \
                         keymap — use `keymap::shortcut` or `keymap::command_tooltip`:\n  {}",
                        number + 1,
                        line.trim()
                    );
                }
            }
        }
    }

    /// The scanner has to actually catch the string the old code contained,
    /// otherwise it is a test that can never fail.
    #[test]
    fn the_chord_scanner_catches_the_label_that_drifted() {
        let needles = chord_needles();
        for offender in [
            // The literal the shortcut help used to carry for Fetch.
            r#"("Ctrl+Shift+F", "Fetch"),"#,
            // The palette's hint field.
            r#"Some("Ctrl+Shift+F"),"#,
            // A chord tucked inside a longer sentence.
            r#"tooltip_text: "Fetch from remote (Ctrl+Shift+R)","#,
            // The macOS spelling.
            r#"Label::new("⌘⇧R")"#,
        ] {
            assert!(
                needles
                    .iter()
                    .any(|needle| offender.contains(needle.as_str())),
                "the scanner would not have caught {offender}"
            );
        }
    }

    /// The glyph style exists to match what macOS shows elsewhere, so on macOS
    /// it must agree with gpui's own rendering for the cases gpui spells with
    /// glyphs. rgitui extends the set to keys gpui leaves as bare words
    /// (`enter`, `space`, …), which is why the comparison is limited.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_glyph_style_matches_gpui_for_the_keys_gpui_spells_with_glyphs() {
        for source in [
            "cmd-shift-r",
            "cmd-s",
            "ctrl-alt-cmd-shift-a",
            "escape",
            "cmd-up",
            "shift-tab",
            "j",
        ] {
            let keystroke = Keystroke::parse(source).expect("the test keystrokes parse");
            assert_eq!(
                humanize_keystroke(source, KeystrokeStyle::Symbols).as_deref(),
                Some(keystroke.to_string().as_str()),
                "{source} drifted from gpui's rendering"
            );
        }
    }
}
