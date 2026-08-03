//! Nested-context shadowing: a panel binding that masks a global one.
//!
//! [`super::conflict`] compares two bindings' context predicates for textual or
//! [`is_superset`] overlap, which cannot see that `GraphView && !TextInput` sits
//! *inside* `Workspace && !modal` at dispatch time. So a binding scoped to a
//! panel taking a keystroke away from a global binding used to go completely
//! unreported.
//!
//! It is reported here, and deliberately at a lower severity than a conflict:
//! deeper-wins scoping is usually the whole point — the shipped defaults rely on
//! it, which is why `Esc` means "back to the diff" in the blame view and
//! "dismiss" everywhere else. Nothing is dropped, both bindings stay applied, and
//! the finding never becomes a toast or a startup warning. It shows up in the
//! shortcuts panel and in `docs/KEYBINDINGS.md`, next to the command that lost
//! the keystroke.
//!
//! # How it is decided
//!
//! [`super::registry::CONTEXT_TREE`] says which element each key context is set
//! on and what encloses it, so every focus path the app can produce can be
//! reconstructed. Both predicates are then evaluated against that path with
//! gpui's own [`depth_of`] — the same function the keymap uses to pick a winner —
//! so the verdict cannot drift from what actually happens when the key is
//! pressed. A binding that matches at a greater depth shadows one that matches at
//! a shallower depth; two that match at the same depth are a
//! [`super::conflict`], not a shadow; and two on unrelated paths never meet.
//!
//! Everything here is pure: [`detect_shadowing`] takes the same
//! `(keystrokes, context, action)` specs the loader hands to gpui.
//!
//! [`is_superset`]: gpui::KeyBindingContextPredicate::is_superset
//! [`depth_of`]: gpui::KeyBindingContextPredicate::depth_of

use gpui::{KeyBindingContextPredicate, KeyContext};

use super::conflict::{normalize_sequence, BindingSpec, NormalizedKeystroke};
use super::registry::{context_node, KeyContextNode, CONTEXT_TREE};

/// A binding that another, deeper binding takes a keystroke away from.
///
/// Both bindings stay applied — this is an observation about which one wins
/// where, not a rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed {
    /// Index of the deeper binding, which wins while its element has focus.
    pub inner: usize,
    /// Index of the binding that cannot be reached there.
    pub outer: usize,
    /// The element that has to have focus for this to happen.
    pub context: &'static str,
    /// Informational explanation, filed against the [`Self::outer`] command.
    pub message: String,
}

/// The focus path ending at `node`, root first, as gpui would see it.
///
/// The root carries `modal` when anything on the path is an overlay, because that
/// is what the workspace root does while one is open — which is in turn what makes
/// a `Workspace && !modal` binding stand down rather than be shadowed.
fn focus_path(node: &'static KeyContextNode) -> Vec<KeyContext> {
    let mut chain: Vec<&'static KeyContextNode> = vec![node];
    while let Some(parent) = chain.last().and_then(|node| node.parent) {
        let Some(parent) = context_node(parent) else {
            break;
        };
        chain.push(parent);
        // `context_tree_is_well_formed` rules cycles out; belt and braces so a
        // malformed table cannot hang the keymap load.
        if chain.len() > CONTEXT_TREE.len() {
            break;
        }
    }
    chain.reverse();

    let modal = chain.iter().any(|node| node.modal);
    chain
        .iter()
        .enumerate()
        .map(|(depth, node)| {
            let mut context = KeyContext::default();
            for identifier in node.identifiers() {
                context.add(identifier);
            }
            if modal && depth == 0 {
                context.add("modal");
            }
            context
        })
        .collect()
}

/// A binding reduced to what shadowing cares about.
struct Candidate {
    /// Index into the caller's binding list.
    index: usize,
    /// Parsed context predicate. `None` means "matches everywhere".
    predicate: Option<KeyBindingContextPredicate>,
    /// Normalised keystroke sequence.
    keys: Vec<NormalizedKeystroke>,
}

/// Reports every binding a deeper one masks. See the [module docs](self).
///
/// `bindings` is in application order and the returned indices point back into
/// it, matching [`super::conflict::ConflictReport`]. Unbinds and bindings whose
/// keystrokes or context do not parse are skipped — the loader reports those, and
/// they never reach gpui.
pub fn detect_shadowing(bindings: &[BindingSpec]) -> Vec<Shadowed> {
    let candidates: Vec<Candidate> = bindings
        .iter()
        .enumerate()
        .filter(|(_, binding)| !binding.is_unbind())
        .filter_map(|(index, binding)| {
            let predicate = match binding.context.as_deref() {
                Some(context) => Some(KeyBindingContextPredicate::parse(context).ok()?),
                None => None,
            };
            Some(Candidate {
                index,
                predicate,
                keys: normalize_sequence(&binding.keystrokes)?,
            })
        })
        .collect();

    let mut found: Vec<Shadowed> = Vec::new();

    for node in CONTEXT_TREE {
        let path = focus_path(node);
        // A binding with no context is treated as if it were on the focused
        // element, exactly as gpui's `binding_enabled` does.
        let live: Vec<(&Candidate, usize)> = candidates
            .iter()
            .filter_map(|candidate| {
                let depth = match &candidate.predicate {
                    Some(predicate) => predicate.depth_of(&path)?,
                    None => path.len(),
                };
                Some((candidate, depth))
            })
            .collect();

        for (inner, inner_depth) in &live {
            for (outer, outer_depth) in &live {
                if outer_depth >= inner_depth
                    || inner.keys != outer.keys
                    || bindings[inner.index].action == bindings[outer.index].action
                {
                    continue;
                }
                // One pair can be shadowed on several paths — a `List` binding
                // masked in every panel that joins the group, say. The first
                // path is enough to explain it.
                if found
                    .iter()
                    .any(|found| found.inner == inner.index && found.outer == outer.index)
                {
                    continue;
                }
                found.push(Shadowed {
                    inner: inner.index,
                    outer: outer.index,
                    context: node.name(),
                    message: message(&bindings[inner.index], &bindings[outer.index], node),
                });
            }
        }
    }

    found
}

/// Explains which binding wins where, in the keystroke spelling `keymap.json`
/// uses so the sentence reads the same on every platform.
fn message(inner: &BindingSpec, outer: &BindingSpec, node: &KeyContextNode) -> String {
    format!(
        "`{}` runs `{}` while {} is focused, so `{}` is not reachable there. \
         Both bindings stay active.",
        inner.keystrokes, inner.action, node.label, outer.action,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::conflict::{detect_conflicts, BindingSpec, NO_ACTION};
    use crate::keymap::loader::default_specs;

    fn default_binding(keystrokes: &str, context: &str, action: &str) -> BindingSpec {
        BindingSpec::default_binding(keystrokes, context, action)
    }

    /// `(keystrokes, inner action, outer action)` for every finding, which is what
    /// the assertions below care about.
    fn findings(bindings: &[BindingSpec]) -> Vec<(&str, &str, &str)> {
        detect_shadowing(bindings)
            .into_iter()
            .map(|shadow| {
                (
                    bindings[shadow.inner].keystrokes.as_str(),
                    bindings[shadow.inner].action.as_str(),
                    bindings[shadow.outer].action.as_str(),
                )
            })
            .collect()
    }

    /// The case that motivated this: `SquashSelected` used to default to
    /// `secondary-shift-s`, which the graph dispatched ahead of the workspace's
    /// `UnstageAll` — invisible to conflict detection, because neither predicate
    /// is a superset of the other.
    #[test]
    fn a_panel_binding_masking_a_global_one_is_reported() {
        let bindings = [
            default_binding(
                "secondary-shift-s",
                "Workspace && !modal",
                "rgitui::UnstageAll",
            ),
            default_binding(
                "secondary-shift-s",
                "GraphView && !TextInput",
                "graph::SquashSelected",
            ),
        ];
        let found = detect_shadowing(&bindings);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].inner, 1);
        assert_eq!(found[0].outer, 0);
        assert_eq!(found[0].context, "GraphView");
        assert_eq!(
            found[0].message,
            "`secondary-shift-s` runs `graph::SquashSelected` while the commit graph is \
             focused, so `rgitui::UnstageAll` is not reachable there. Both bindings stay active."
        );

        // And conflict detection still sees nothing, which is the gap being filled.
        assert!(detect_conflicts(&bindings).conflicts.is_empty());
    }

    /// Two bindings on the same element are a conflict, not a shadow: one of them
    /// is genuinely dead and gets dropped.
    #[test]
    fn a_same_context_duplicate_is_left_to_conflict_detection() {
        let bindings = [
            default_binding("secondary-s", "Workspace && !modal", "rgitui::StageAll"),
            default_binding("secondary-s", "Workspace && !modal", "rgitui::Commit"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
        assert_eq!(detect_conflicts(&bindings).conflicts.len(), 1);
    }

    /// Nor is a superset overlap a shadow — the same depth, so `conflict` owns it.
    #[test]
    fn a_superset_overlap_at_one_depth_is_not_a_shadow() {
        let bindings = [
            default_binding("secondary-s", "Workspace && !modal", "rgitui::StageAll"),
            default_binding("secondary-s", "Workspace", "rgitui::Commit"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
        assert!(!detect_conflicts(&bindings).conflicts.is_empty());
    }

    /// Two panels can own the same letter without either losing anything, which is
    /// exactly what the per-view contexts are for.
    #[test]
    fn sibling_panels_sharing_a_keystroke_are_not_reported() {
        let bindings = [
            default_binding("s", "GraphView && !TextInput", "graph::SquashSelected"),
            default_binding("s", "Sidebar && !TextInput", "sidebar::ToggleStageRow"),
            default_binding("s", "DiffViewer && !TextInput", "diff::StageSelection"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
        assert!(detect_conflicts(&bindings).conflicts.is_empty());
    }

    /// The two window roots never see each other's bindings.
    #[test]
    fn bindings_in_separate_windows_are_not_reported() {
        let bindings = [
            default_binding("escape", "Workspace", "menu::Cancel"),
            default_binding("escape", "SettingsWindow", "menu::Cancel"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
    }

    /// A global binding a modal already suppresses is not shadowed by the modal's
    /// own binding: `!modal` turned it off, nothing took it away.
    #[test]
    fn a_modal_does_not_shadow_what_the_modal_gate_already_disabled() {
        let bindings = [
            default_binding(
                "tab",
                "Workspace && !modal && !TextInput",
                "rgitui::FocusNextPanel",
            ),
            default_binding("tab", "ThemeEditor", "theme::ThemeEditorNextField"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());

        // Without the `!modal` gate the same pair is a shadow, so it is the gate
        // doing the work rather than the overlay being skipped.
        let ungated = [
            default_binding("tab", "Workspace", "rgitui::FocusNextPanel"),
            default_binding("tab", "ThemeEditor", "theme::ThemeEditorNextField"),
        ];
        assert_eq!(detect_shadowing(&ungated).len(), 1);
    }

    #[test]
    fn a_user_binding_can_shadow_a_default() {
        let mut bindings = vec![default_binding(
            "ctrl-alt-9",
            "Workspace && !modal",
            "rgitui::Fetch",
        )];
        bindings.push(BindingSpec::user_binding(
            "ctrl-alt-9",
            Some("DiffViewer"),
            "diff::NextHunk",
        ));
        let found = detect_shadowing(&bindings);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("the diff viewer"), "{found:?}");
    }

    /// One command bound in two contexts is one behaviour, not a shadow of itself.
    #[test]
    fn a_command_does_not_shadow_itself() {
        let bindings = [
            default_binding("/", "Workspace && !modal && !TextInput", "rgitui::Search"),
            default_binding("/", "GraphView && !TextInput", "rgitui::Search"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
    }

    #[test]
    fn an_unbind_is_not_a_shadow() {
        let bindings = [
            default_binding("secondary-s", "Workspace && !modal", "rgitui::StageAll"),
            BindingSpec::user_binding("secondary-s", Some("GraphView"), NO_ACTION),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
    }

    #[test]
    fn an_unparseable_binding_is_skipped_rather_than_reported() {
        let bindings = [
            default_binding("ctrl-a-b", "Workspace", "rgitui::StageAll"),
            default_binding("ctrl-a-b", "GraphView", "graph::CopyCommitSha"),
            default_binding("ctrl-alt-1", "Workspace &&", "rgitui::Fetch"),
            default_binding("ctrl-alt-1", "GraphView", "graph::CopyCommitMessage"),
        ];
        assert!(detect_shadowing(&bindings).is_empty());
    }

    /// The shipped defaults must not conflict — that is already pinned in the
    /// loader — and the shadowing they do have is deliberate, so it is spelled out
    /// here rather than merely counted.
    #[test]
    fn the_shipped_defaults_shadow_only_these() {
        let specs = default_specs();
        assert!(
            detect_conflicts(&specs).conflicts.is_empty(),
            "the defaults must never conflict"
        );

        // Esc means "back to the diff" or "close the search" in the three views
        // below and dismisses the topmost overlay everywhere else; `/` and Ctrl+F
        // filter the focused list instead of searching the commit graph.
        let mut expected = vec![
            ("escape", "blame::BlameShowDiff", "menu::Cancel"),
            ("escape", "graph::GraphCancel", "menu::Cancel"),
            ("escape", "history::HistoryShowDiff", "menu::Cancel"),
            ("/", "detail::FileSearch", "rgitui::Search"),
            ("/", "sidebar::FilterBranches", "rgitui::Search"),
            ("secondary-f", "detail::FileSearch", "rgitui::Search"),
            ("secondary-f", "sidebar::FilterBranches", "rgitui::Search"),
        ];
        expected.sort_unstable();

        let mut found = findings(&specs);
        found.sort_unstable();
        assert_eq!(
            found, expected,
            "the defaults' shadowing changed — check it is still intentional"
        );
    }

    /// Bare `s` for squash has to be reachable in the graph, which means nothing
    /// shallower may claim it and nothing deeper may take it away.
    #[test]
    fn bare_s_in_the_graph_collides_with_nothing() {
        let specs = default_specs();
        let squash: Vec<&BindingSpec> = specs
            .iter()
            .filter(|spec| spec.action == "graph::SquashSelected")
            .collect();
        assert_eq!(
            squash
                .iter()
                .map(|spec| (spec.keystrokes.as_str(), spec.context.as_deref()))
                .collect::<Vec<_>>(),
            [("s", Some("GraphView && !modal && !TextInput"))]
        );

        for shadow in detect_shadowing(&specs) {
            assert_ne!(
                specs[shadow.outer].action, "graph::SquashSelected",
                "something takes `s` away from squash: {}",
                shadow.message
            );
            assert_ne!(
                specs[shadow.inner].action, "graph::SquashSelected",
                "squash takes a keystroke away from another command: {}",
                shadow.message
            );
        }
    }
}
