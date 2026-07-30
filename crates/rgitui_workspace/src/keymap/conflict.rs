//! Keybinding conflict detection.
//!
//! gpui resolves ambiguity silently: when two bindings match the same keystroke
//! in overlapping contexts the later one wins and the earlier one simply never
//! fires. That is fine for the *intended* case — a user binding replacing a
//! default — but it hides genuine mistakes, so rgitui detects and reports them
//! before handing anything to gpui.
//!
//! Everything here is pure: [`detect_conflicts`] takes a list of
//! `(keystrokes, context, action)` triples and returns a report, so it is
//! unit-testable without a window or an `App`.
//!
//! # Rules
//!
//! A pair of bindings conflicts when
//!
//! 1. **Overlap** — they resolve to the same keystroke sequence and their
//!    contexts overlap. Contexts overlap when they are equal, when either is
//!    absent (an absent context matches everywhere), or when one predicate is a
//!    superset of the other per [`KeyBindingContextPredicate::is_superset`].
//! 2. **Chord prefix** — one binding's keystroke sequence is a strict prefix of
//!    the other's and their contexts overlap, e.g. `ctrl-k` alongside
//!    `ctrl-k ctrl-o`. Typing the prefix would resolve immediately and the
//!    chord could never be reached.
//!
//! It is *not* a conflict when
//!
//! * a [`BindingSource::User`] binding overlaps a [`BindingSource::Default`]
//!   one — that is the whole point of a user keymap; or
//! * either binding is an unbind (`null` in `keymap.json`, gpui's `NoAction`),
//!   which is a deliberate instruction to remove a binding.
//!
//! # Resolution
//!
//! For an overlap the later entry wins and the earlier one is dropped, matching
//! gpui's own precedence. For a chord prefix the *prefix* is dropped whichever
//! order it appears in, so the longer chord stays reachable. Either way the
//! losing entry is reported rather than silently applied.

use std::fmt;

use gpui::{KeyBindingContextPredicate, Keystroke};

/// Where a binding came from. Defaults are always ordered before user bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingSource {
    /// Declared by `commands!` in the registry.
    Default,
    /// Read from the user's `keymap.json`.
    User,
}

impl fmt::Display for BindingSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingSource::Default => f.write_str("default"),
            BindingSource::User => f.write_str("keymap.json"),
        }
    }
}

/// The gpui action name that removes a binding. Bound to `null` in `keymap.json`.
pub const NO_ACTION: &str = "zed::NoAction";

/// One candidate key binding, before it is handed to gpui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSpec {
    /// Whitespace-separated keystrokes, exactly as written.
    pub keystrokes: String,
    /// Context predicate source, or `None` for "matches everywhere".
    pub context: Option<String>,
    /// Full gpui action name, or [`NO_ACTION`] for an unbind.
    pub action: String,
    /// Where this binding came from.
    pub source: BindingSource,
}

impl BindingSpec {
    /// A binding declared by the registry.
    pub fn default_binding(keystrokes: &str, context: &str, action: &str) -> Self {
        Self {
            keystrokes: keystrokes.to_owned(),
            context: (!context.is_empty()).then(|| context.to_owned()),
            action: action.to_owned(),
            source: BindingSource::Default,
        }
    }

    /// A binding read from the user's `keymap.json`.
    pub fn user_binding(keystrokes: &str, context: Option<&str>, action: &str) -> Self {
        Self {
            keystrokes: keystrokes.to_owned(),
            context: context.filter(|c| !c.is_empty()).map(str::to_owned),
            action: action.to_owned(),
            source: BindingSource::User,
        }
    }

    /// Whether this entry removes a binding rather than adding one.
    pub fn is_unbind(&self) -> bool {
        self.action == NO_ACTION
    }

    fn context_label(&self) -> &str {
        self.context.as_deref().unwrap_or("<any>")
    }
}

/// Why two bindings conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Same keystroke sequence, overlapping contexts.
    Overlap,
    /// One keystroke sequence is a strict prefix of the other's chord.
    ChordPrefix,
}

/// A detected conflict between two candidate bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Why the two bindings conflict.
    pub kind: ConflictKind,
    /// Index of the binding that is kept.
    pub winner: usize,
    /// Index of the binding that is dropped.
    pub ignored: usize,
    /// Human-readable explanation, shown to the user.
    pub message: String,
}

/// The outcome of [`detect_conflicts`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictReport {
    /// Every conflict found, in the order the losing binding appears.
    pub conflicts: Vec<Conflict>,
    /// Indices of bindings that must not be applied. Sorted and deduplicated.
    pub dropped: Vec<usize>,
}

impl ConflictReport {
    /// Whether the binding at `index` survived conflict resolution.
    pub fn is_kept(&self, index: usize) -> bool {
        !self.dropped.contains(&index)
    }

    /// One message per conflict, ready to surface as a toast.
    pub fn messages(&self) -> Vec<String> {
        self.conflicts
            .iter()
            .map(|conflict| conflict.message.clone())
            .collect()
    }
}

/// A keystroke reduced to the parts gpui matches on, so that different spellings
/// of the same keystroke (`ctrl-s` and `secondary-s` off macOS) compare equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedKeystroke {
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    function: bool,
    key: String,
}

impl From<Keystroke> for NormalizedKeystroke {
    fn from(keystroke: Keystroke) -> Self {
        Self {
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            platform: keystroke.modifiers.platform,
            function: keystroke.modifiers.function,
            key: keystroke.key,
        }
    }
}

/// Parses a whitespace-separated keystroke sequence.
///
/// Returns `None` when any keystroke is unparseable — such entries are reported
/// by the loader and excluded from conflict detection.
pub fn normalize_sequence(keystrokes: &str) -> Option<Vec<NormalizedKeystroke>> {
    let sequence: Vec<NormalizedKeystroke> = keystrokes
        .split_whitespace()
        .map(|source| Keystroke::parse(source).ok().map(NormalizedKeystroke::from))
        .collect::<Option<_>>()?;
    (!sequence.is_empty()).then_some(sequence)
}

/// Whether two context predicates can both be satisfied by the same element.
///
/// An absent predicate matches everywhere, so it overlaps everything. Two
/// present predicates overlap when either is a superset of the other. A
/// predicate that fails to parse is reported by the loader; here it is treated
/// as overlapping nothing so a single bad predicate cannot suppress unrelated
/// bindings.
pub fn contexts_overlap(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, _) | (_, None) => true,
        (Some(left), Some(right)) => {
            if left == right {
                return true;
            }
            let Ok(left) = KeyBindingContextPredicate::parse(left) else {
                return false;
            };
            let Ok(right) = KeyBindingContextPredicate::parse(right) else {
                return false;
            };
            left.is_superset(&right) || right.is_superset(&left)
        }
    }
}

/// Whether `prefix` is a strict prefix of `sequence`.
fn is_strict_prefix(prefix: &[NormalizedKeystroke], sequence: &[NormalizedKeystroke]) -> bool {
    prefix.len() < sequence.len() && sequence.starts_with(prefix)
}

/// Finds every conflict in `bindings`, which must be ordered by precedence:
/// earlier entries lose to later ones. See the [module docs](self) for the rules.
pub fn detect_conflicts(bindings: &[BindingSpec]) -> ConflictReport {
    let sequences: Vec<Option<Vec<NormalizedKeystroke>>> = bindings
        .iter()
        .map(|binding| normalize_sequence(&binding.keystrokes))
        .collect();

    let mut conflicts = Vec::new();
    let mut dropped = Vec::new();

    for later in 0..bindings.len() {
        for earlier in 0..later {
            let (Some(earlier_keys), Some(later_keys)) = (&sequences[earlier], &sequences[later])
            else {
                continue;
            };
            let (earlier_binding, later_binding) = (&bindings[earlier], &bindings[later]);

            // Unbinds are deliberate removals, not mistakes.
            if earlier_binding.is_unbind() || later_binding.is_unbind() {
                continue;
            }
            if !contexts_overlap(
                earlier_binding.context.as_deref(),
                later_binding.context.as_deref(),
            ) {
                continue;
            }

            if earlier_keys == later_keys {
                // A user binding replacing a default is the feature, not a bug.
                if earlier_binding.source == BindingSource::Default
                    && later_binding.source == BindingSource::User
                {
                    continue;
                }
                conflicts.push(Conflict {
                    kind: ConflictKind::Overlap,
                    winner: later,
                    ignored: earlier,
                    message: format!(
                        "`{}` is bound twice in overlapping contexts: `{}` ({}, context `{}`) \
                         is ignored in favour of `{}` ({}, context `{}`).",
                        later_binding.keystrokes,
                        earlier_binding.action,
                        earlier_binding.source,
                        earlier_binding.context_label(),
                        later_binding.action,
                        later_binding.source,
                        later_binding.context_label(),
                    ),
                });
                dropped.push(earlier);
            } else if is_strict_prefix(earlier_keys, later_keys) {
                conflicts.push(prefix_conflict(earlier, later, bindings));
                dropped.push(earlier);
            } else if is_strict_prefix(later_keys, earlier_keys) {
                conflicts.push(prefix_conflict(later, earlier, bindings));
                dropped.push(later);
            }
        }
    }

    dropped.sort_unstable();
    dropped.dedup();
    ConflictReport { conflicts, dropped }
}

/// Builds the report entry for a chord-prefix conflict. The prefix loses so the
/// longer chord stays reachable.
fn prefix_conflict(prefix: usize, chord: usize, bindings: &[BindingSpec]) -> Conflict {
    let (prefix_binding, chord_binding) = (&bindings[prefix], &bindings[chord]);
    Conflict {
        kind: ConflictKind::ChordPrefix,
        winner: chord,
        ignored: prefix,
        message: format!(
            "`{}` ({}, context `{}`) shadows the chord `{}` ({}, context `{}`); \
             the shorter binding is ignored so the chord stays reachable.",
            prefix_binding.keystrokes,
            prefix_binding.source,
            prefix_binding.context_label(),
            chord_binding.keystrokes,
            chord_binding.source,
            chord_binding.context_label(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_binding(keystrokes: &str, context: &str, action: &str) -> BindingSpec {
        BindingSpec::default_binding(keystrokes, context, action)
    }

    fn user_binding(keystrokes: &str, context: Option<&str>, action: &str) -> BindingSpec {
        BindingSpec::user_binding(keystrokes, context, action)
    }

    #[test]
    fn distinct_keystrokes_do_not_conflict() {
        let report = detect_conflicts(&[
            default_binding("ctrl-s", "Workspace", "rgitui::StageAll"),
            default_binding("ctrl-u", "Workspace", "rgitui::UnstageAll"),
        ]);
        assert_eq!(report, ConflictReport::default());
    }

    #[test]
    fn disjoint_contexts_do_not_conflict() {
        let report = detect_conflicts(&[
            default_binding("y", "GraphView", "graph::CopySha"),
            default_binding("y", "DiffViewer", "diff::CopyLine"),
        ]);
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    }

    #[test]
    fn same_context_duplicate_is_detected_and_the_later_wins() {
        let bindings = [
            default_binding("ctrl-s", "Workspace", "rgitui::StageAll"),
            default_binding("ctrl-s", "Workspace", "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, ConflictKind::Overlap);
        assert_eq!(report.conflicts[0].winner, 1);
        assert_eq!(report.conflicts[0].ignored, 0);
        assert_eq!(report.dropped, vec![0]);
        assert!(!report.is_kept(0));
        assert!(report.is_kept(1));
    }

    #[test]
    fn superset_context_overlap_is_detected() {
        // `Workspace` matches everything `Workspace && !modal` matches.
        let bindings = [
            default_binding("ctrl-s", "Workspace && !modal", "rgitui::StageAll"),
            default_binding("ctrl-s", "Workspace", "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, ConflictKind::Overlap);
        assert_eq!(report.dropped, vec![0]);
    }

    #[test]
    fn a_missing_context_overlaps_every_context() {
        let bindings = [
            user_binding("ctrl-s", None, "rgitui::StageAll"),
            user_binding("ctrl-s", Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.dropped, vec![0]);
    }

    #[test]
    fn chord_prefix_shadowing_drops_the_prefix() {
        let bindings = [
            user_binding("ctrl-k ctrl-o", Some("Workspace"), "rgitui::OpenRepo"),
            user_binding("ctrl-k", Some("Workspace"), "rgitui::StageAll"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, ConflictKind::ChordPrefix);
        // The prefix loses even though it comes later, so the chord stays reachable.
        assert_eq!(report.conflicts[0].ignored, 1);
        assert_eq!(report.conflicts[0].winner, 0);
        assert_eq!(report.dropped, vec![1]);
    }

    #[test]
    fn chord_prefix_shadowing_is_order_independent() {
        let bindings = [
            user_binding("ctrl-k", Some("Workspace"), "rgitui::StageAll"),
            user_binding("ctrl-k ctrl-o", Some("Workspace"), "rgitui::OpenRepo"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, ConflictKind::ChordPrefix);
        assert_eq!(report.dropped, vec![0]);
    }

    #[test]
    fn chord_prefix_in_a_disjoint_context_is_fine() {
        let bindings = [
            user_binding("ctrl-k ctrl-o", Some("GraphView"), "graph::CopySha"),
            user_binding("ctrl-k", Some("DiffViewer"), "diff::CopyLine"),
        ];
        let report = detect_conflicts(&bindings);
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    }

    #[test]
    fn a_user_binding_may_replace_a_default() {
        let bindings = [
            default_binding("ctrl-s", "Workspace", "rgitui::StageAll"),
            user_binding("ctrl-s", Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report, ConflictReport::default());
    }

    #[test]
    fn two_user_bindings_on_one_keystroke_do_conflict() {
        let bindings = [
            user_binding("ctrl-s", Some("Workspace"), "rgitui::StageAll"),
            user_binding("ctrl-s", Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.dropped, vec![0]);
    }

    #[test]
    fn unbinds_are_never_conflicts() {
        let bindings = [
            default_binding("ctrl-s", "Workspace", "rgitui::StageAll"),
            user_binding("ctrl-s", Some("Workspace"), NO_ACTION),
            user_binding("ctrl-k", Some("Workspace"), NO_ACTION),
            user_binding("ctrl-k ctrl-o", Some("Workspace"), "rgitui::OpenRepo"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report, ConflictReport::default());
    }

    #[test]
    fn equivalent_spellings_of_one_keystroke_are_compared_equal() {
        let native = if cfg!(target_os = "macos") {
            "cmd-s"
        } else {
            "ctrl-s"
        };
        let bindings = [
            user_binding("secondary-s", Some("Workspace"), "rgitui::StageAll"),
            user_binding(native, Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert_eq!(report.conflicts.len(), 1, "{:?}", report.conflicts);
    }

    #[test]
    fn unparseable_keystrokes_are_skipped() {
        let bindings = [
            user_binding("ctrl-a-b", Some("Workspace"), "rgitui::StageAll"),
            user_binding("ctrl-a-b", Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn messages_name_both_bindings() {
        let bindings = [
            user_binding("ctrl-s", Some("Workspace"), "rgitui::StageAll"),
            user_binding("ctrl-s", Some("Workspace"), "rgitui::Commit"),
        ];
        let report = detect_conflicts(&bindings);
        let message = &report.messages()[0];
        assert!(message.contains("rgitui::StageAll"), "{message}");
        assert!(message.contains("rgitui::Commit"), "{message}");
        assert!(message.contains("ctrl-s"), "{message}");
    }
}
