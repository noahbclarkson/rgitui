//! What the keymap load actually produced, in a form the UI can render.
//!
//! Every shortcut rgitui shows the user comes from here: the shortcut
//! reference, the command palette hints, the settings quick reference, the
//! toolbar tooltips and the home screen. One source means the surfaces cannot
//! disagree, and it means they follow the user's `keymap.json` rather than
//! advertising a default the user has rebound.
//!
//! # Why not ask gpui
//!
//! [`gpui::Keymap::bindings_for_action`] returns the bindings gpui holds for an
//! action, which is close but not enough:
//!
//! * it cannot say whether a binding came from the registry or from
//!   `keymap.json`, which is exactly what the user wants flagged;
//! * it only filters bindings an explicit `null` removed, so after
//!   `"ctrl-s": "rgitui::Commit"` it would still report `ctrl-s` for
//!   `StageAll`, a keystroke that can no longer reach it; and
//! * conflicts are rgitui's own analysis ([`super::conflict`]) and gpui knows
//!   nothing about them.
//!
//! So [`KeymapSummary::build`] is handed the very list of [`BindingSpec`]s that
//! is turned into `gpui::KeyBinding`s, plus the indices that survived, and
//! derives the display from that. It agrees with the live keymap by
//! construction, and it is pure — no `App`, no window, unit-testable.
//!
//! # Shadowing
//!
//! A binding is *shadowed* when a later applied binding claims the same
//! keystroke in an overlapping context. gpui resolves that silently in the
//! later binding's favour, so the earlier one is dropped from the display and
//! the affected command gets a warning saying where its keystroke went. That is
//! how a rebind shows up as "this command lost its shortcut" instead of as a
//! shortcut that does nothing.
//!
//! # Severity
//!
//! Not everything worth telling the user is a problem, so each note carries a
//! [`NoteSeverity`]. A [`NoteSeverity::Warning`] means something was dropped or a
//! command lost a keystroke it will not get back. A [`NoteSeverity::Info`] is the
//! nested-context shadowing [`super::shadow`] finds: both bindings are applied
//! and the scoping is very likely deliberate — the shipped defaults produce
//! several — so it must never become a toast or a startup warning. Keeping both
//! on one list means the shortcuts panel shows them in the same place, styled by
//! severity, with no second channel to keep in step.

use std::sync::Arc;

use super::conflict::{
    self, BindingSource, BindingSpec, ConflictReport, NormalizedKeystroke, NO_ACTION,
};
use super::display::{self, KeystrokeStyle};
use super::registry::{command_for_action, CommandId, ALL_COMMANDS};
use super::shadow;

/// One binding of one command, as it will actually fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveBinding {
    /// Keystrokes as written, e.g. `secondary-shift-r`.
    pub keystrokes: String,
    /// Humanised label, e.g. `Ctrl+Shift+R`.
    pub display: String,
    /// Key context the binding is scoped to, or `None` for "everywhere".
    pub context: Option<String>,
    /// Whether the registry or the user's `keymap.json` supplied it.
    pub source: BindingSource,
}

impl EffectiveBinding {
    /// Whether this binding came from the user's `keymap.json`.
    pub fn is_user_defined(&self) -> bool {
        self.source == BindingSource::User
    }
}

/// How much a note matters. See the [module docs](self#severity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSeverity {
    /// Something was dropped, or the command lost a keystroke for good.
    Warning,
    /// A deeper binding wins the keystroke somewhere. Nothing was dropped.
    Info,
}

/// One thing worth telling the user about a command's bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapNote {
    /// How much it matters.
    pub severity: NoteSeverity,
    /// The explanation, ready to render.
    pub message: String,
}

impl KeymapNote {
    /// Whether this note reports an actual problem.
    pub fn is_warning(&self) -> bool {
        self.severity == NoteSeverity::Warning
    }
}

/// Everything the UI needs to show about one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBindings {
    /// The command.
    pub command: CommandId,
    /// Bindings that will fire, in precedence order (last wins).
    pub bindings: Vec<EffectiveBinding>,
    /// What the user should know: a binding of theirs that was dropped, a
    /// keystroke this command lost to another, or where a panel wins one of its
    /// keystrokes. Warnings first, then info, each in detection order.
    pub notes: Vec<KeymapNote>,
}

impl CommandBindings {
    /// The humanised keystrokes, or `None` when the command is unbound.
    pub fn display(&self) -> Option<String> {
        display::join_bindings(self.bindings.iter().map(|binding| binding.display.as_str()))
    }

    /// Whether any of this command's bindings came from `keymap.json`.
    pub fn is_user_defined(&self) -> bool {
        self.bindings.iter().any(EffectiveBinding::is_user_defined)
    }

    /// The messages of this command's notes at one severity.
    pub fn messages(&self, severity: NoteSeverity) -> Vec<&str> {
        self.notes
            .iter()
            .filter(|note| note.severity == severity)
            .map(|note| note.message.as_str())
            .collect()
    }
}

/// A group of commands sharing a `commands!` view block, which is also the key
/// context they are scoped to. Drives the layout of the shortcut reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandGroup {
    /// The view name from the registry, e.g. `DiffViewer`.
    pub view: &'static str,
    /// The distinct key contexts the group's bindings use.
    pub contexts: Vec<&'static str>,
    /// The group's commands: bound ones first, each in registry order.
    pub commands: Vec<CommandBindings>,
}

impl CommandGroup {
    /// A sentence describing when the group's shortcuts are live, derived from
    /// the key contexts rather than written by hand.
    pub fn description(&self) -> String {
        match self.contexts.as_slice() {
            [] => "Reachable from the command palette only.".to_owned(),
            contexts => format!("Active while `{}` matches.", contexts.join("`, `")),
        }
    }
}

/// The bindings in force, grouped and labelled for display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapSummary {
    /// Every command, in registry order.
    commands: Vec<CommandBindings>,
    /// Warnings that name no registry command — a `keymap.json` binding for an
    /// action rgitui does not know, for instance. Surfaced so nothing is lost.
    pub unattributed_warnings: Vec<String>,
}

impl KeymapSummary {
    /// Derives the summary from the specs handed to gpui.
    ///
    /// `specs` must be in the order they are applied (defaults then user
    /// bindings) and `applied` the indices that survived conflict detection and
    /// validation, exactly as the loader computed them.
    pub fn build(
        specs: &[BindingSpec],
        applied: &[usize],
        report: &ConflictReport,
        style: KeystrokeStyle,
    ) -> Self {
        let mut commands: Vec<CommandBindings> = ALL_COMMANDS
            .iter()
            .map(|meta| CommandBindings {
                command: meta.id,
                bindings: Vec::new(),
                notes: Vec::new(),
            })
            .collect();
        let mut unattributed_warnings = Vec::new();

        for conflict in &report.conflicts {
            let ignored = &specs[conflict.ignored];
            note(
                &mut commands,
                &mut unattributed_warnings,
                &ignored.action,
                NoteSeverity::Warning,
                conflict.message.clone(),
            );
        }

        // Only applied specs can fire, and only in the order they were applied.
        let applied_specs: Vec<&BindingSpec> = applied.iter().map(|&index| &specs[index]).collect();
        let sequences: Vec<Option<Vec<NormalizedKeystroke>>> = applied_specs
            .iter()
            .map(|spec| conflict::normalize_sequence(&spec.keystrokes))
            .collect();

        for (index, spec) in applied_specs.iter().enumerate() {
            if spec.is_unbind() {
                continue;
            }
            let Some(display) = display::humanize_sequence(&spec.keystrokes, style) else {
                continue;
            };

            if let Some(shadow) = shadow_of(index, &applied_specs, &sequences) {
                note(
                    &mut commands,
                    &mut unattributed_warnings,
                    &spec.action,
                    NoteSeverity::Warning,
                    shadow_message(&display, applied_specs[shadow]),
                );
                continue;
            }

            let Some(id) = command_for_action(&spec.action) else {
                continue;
            };
            let Some(entry) = commands.iter_mut().find(|entry| entry.command == id) else {
                continue;
            };
            entry.bindings.push(EffectiveBinding {
                keystrokes: spec.keystrokes.clone(),
                display,
                context: spec.context.clone(),
                source: spec.source,
            });
        }

        // Last, so every command's warnings come before its info notes: a panel
        // binding that wins a keystroke from a global one. Both bindings are
        // applied — the finding is filed against the command that cannot reach
        // its keystroke while that panel has focus.
        for found in shadow::detect_shadowing(specs) {
            if applied.binary_search(&found.inner).is_err()
                || applied.binary_search(&found.outer).is_err()
            {
                continue;
            }
            note(
                &mut commands,
                &mut unattributed_warnings,
                &specs[found.outer].action,
                NoteSeverity::Info,
                found.message,
            );
        }

        Self {
            commands,
            unattributed_warnings,
        }
    }

    /// The registry defaults on their own, with no user keymap.
    ///
    /// Used as the fallback before the keymap has loaded, and by the tests that
    /// pin the default labels.
    pub fn defaults(style: KeystrokeStyle) -> Self {
        let specs = super::loader::default_specs();
        let report = conflict::detect_conflicts(&specs);
        let applied: Vec<usize> = (0..specs.len()).filter(|i| report.is_kept(*i)).collect();
        Self::build(&specs, &applied, &report, style)
    }

    /// Every command, in registry order.
    pub fn commands(&self) -> &[CommandBindings] {
        &self.commands
    }

    /// What is known about one command.
    pub fn command(&self, id: CommandId) -> Option<&CommandBindings> {
        self.commands.iter().find(|entry| entry.command == id)
    }

    /// The humanised keystrokes for one command, or `None` when it is unbound.
    ///
    /// This is the accessor every shortcut hint in the UI goes through.
    pub fn display(&self, id: CommandId) -> Option<String> {
        self.command(id).and_then(CommandBindings::display)
    }

    /// Whether the user rebound this command in `keymap.json`.
    pub fn is_user_defined(&self, id: CommandId) -> bool {
        self.command(id)
            .is_some_and(CommandBindings::is_user_defined)
    }

    /// Everything worth saying about one command's bindings, warnings first.
    pub fn notes(&self, id: CommandId) -> &[KeymapNote] {
        self.command(id).map_or(&[], |entry| &entry.notes)
    }

    /// Problems with one command: dropped bindings and lost keystrokes.
    pub fn warnings(&self, id: CommandId) -> Vec<&str> {
        self.command(id)
            .map_or_else(Vec::new, |entry| entry.messages(NoteSeverity::Warning))
    }

    /// Informational notes for one command: keystrokes a panel wins from it.
    pub fn infos(&self, id: CommandId) -> Vec<&str> {
        self.command(id)
            .map_or_else(Vec::new, |entry| entry.messages(NoteSeverity::Info))
    }

    /// Number of bindings that came from `keymap.json`.
    pub fn user_binding_count(&self) -> usize {
        self.commands
            .iter()
            .flat_map(|entry| &entry.bindings)
            .filter(|binding| binding.is_user_defined())
            .count()
    }

    /// Number of commands carrying at least one warning.
    ///
    /// Informational notes are excluded on purpose: the shipped defaults produce
    /// several, and counting them would tell every user their keymap has problems.
    pub fn warning_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|entry| entry.notes.iter().any(KeymapNote::is_warning))
            .count()
            + usize::from(!self.unattributed_warnings.is_empty())
    }

    /// Number of commands that have at least one binding.
    pub fn bound_command_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|entry| !entry.bindings.is_empty())
            .count()
    }

    /// The commands grouped by `commands!` view block, bound ones first.
    ///
    /// Bound before unbound because the reference is read to look a keystroke
    /// up; the unbound tail is kept so the palette-only commands are still
    /// discoverable, and so a command whose binding the user removed does not
    /// silently vanish.
    pub fn groups(&self) -> Vec<CommandGroup> {
        let mut groups: Vec<CommandGroup> = Vec::new();

        for meta in ALL_COMMANDS {
            let Some(entry) = self.command(meta.id) else {
                continue;
            };
            let group = match groups.iter_mut().find(|group| group.view == meta.view) {
                Some(group) => group,
                None => {
                    groups.push(CommandGroup {
                        view: meta.view,
                        contexts: Vec::new(),
                        commands: Vec::new(),
                    });
                    groups.last_mut().expect("just pushed")
                }
            };
            for (_, context) in meta.default_bindings {
                if !group.contexts.contains(context) {
                    group.contexts.push(context);
                }
            }
            group.commands.push(entry.clone());
        }

        for group in &mut groups {
            group
                .commands
                .sort_by_key(|entry| entry.bindings.is_empty());
        }
        groups
    }
}

/// Files a note against the command that owns `action`.
///
/// A *warning* about an action no registry command owns goes to `unattributed`
/// instead, so nothing the user has to fix is lost. An *informational* note about
/// one is dropped: there is no row for it to sit next to and nothing to fix.
fn note(
    commands: &mut [CommandBindings],
    unattributed: &mut Vec<String>,
    action: &str,
    severity: NoteSeverity,
    message: String,
) {
    match command_for_action(action)
        .and_then(|id| commands.iter_mut().find(|entry| entry.command == id))
    {
        Some(entry) => entry.notes.push(KeymapNote { severity, message }),
        None if severity == NoteSeverity::Warning => unattributed.push(message),
        None => {}
    }
}

/// Index of the applied binding that takes `index`'s keystroke away, if any.
///
/// A later binding shadows an earlier one when the keystroke sequences match and
/// the contexts overlap — including an unbind, which is a deliberate removal.
fn shadow_of(
    index: usize,
    applied: &[&BindingSpec],
    sequences: &[Option<Vec<NormalizedKeystroke>>],
) -> Option<usize> {
    let keys = sequences[index].as_ref()?;
    ((index + 1)..applied.len()).rev().find(|&later| {
        sequences[later].as_ref() == Some(keys)
            && applied[later].action != applied[index].action
            && conflict::contexts_overlap(
                applied[index].context.as_deref(),
                applied[later].context.as_deref(),
            )
    })
}

/// Explains where a command's keystroke went.
fn shadow_message(display: &str, winner: &BindingSpec) -> String {
    if winner.action == NO_ACTION {
        return format!("`{display}` was removed by keymap.json.");
    }
    let winner_label = command_for_action(&winner.action)
        .map(|id| format!("`{}`", id.description()))
        .unwrap_or_else(|| format!("`{}`", winner.action));
    format!(
        "`{display}` now runs {winner_label} ({}), so it no longer reaches this command.",
        winner.source
    )
}

/// The summary shown before the keymap has loaded, built once.
pub fn fallback() -> Arc<KeymapSummary> {
    use std::sync::OnceLock;
    static FALLBACK: OnceLock<Arc<KeymapSummary>> = OnceLock::new();
    FALLBACK
        .get_or_init(|| Arc::new(KeymapSummary::defaults(KeystrokeStyle::platform())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults in the word style, which is what the assertions below spell.
    fn defaults() -> KeymapSummary {
        KeymapSummary::defaults(KeystrokeStyle::Words)
    }

    /// Builds a summary from the defaults plus the given user bindings, applying
    /// the same conflict resolution the loader does.
    fn with_user(user: &[BindingSpec]) -> KeymapSummary {
        let mut specs = super::super::loader::default_specs();
        specs.extend(user.iter().cloned());
        let report = conflict::detect_conflicts(&specs);
        let applied: Vec<usize> = (0..specs.len()).filter(|i| report.is_kept(*i)).collect();
        KeymapSummary::build(&specs, &applied, &report, KeystrokeStyle::Words)
    }

    fn user(keystrokes: &str, context: &str, action: &str) -> BindingSpec {
        BindingSpec::user_binding(keystrokes, Some(context), action)
    }

    /// The drift that motivated this work: the help used to advertise
    /// `Ctrl+Shift+F` for Fetch while the registry bound `Ctrl+Shift+R`.
    #[test]
    fn a_default_label_comes_from_the_registry_keystroke() {
        let summary = defaults();
        assert_eq!(
            summary.display(CommandId::Fetch).as_deref(),
            Some("Ctrl+Shift+R")
        );
        assert_eq!(
            summary.display(CommandId::Fetch),
            display::humanize_sequence(
                CommandId::Fetch.default_bindings()[0].0,
                KeystrokeStyle::Words
            )
        );
        assert!(!summary.is_user_defined(CommandId::Fetch));
    }

    #[test]
    fn every_default_binding_is_labelled() {
        let summary = defaults();
        for meta in ALL_COMMANDS {
            let entry = summary
                .command(meta.id)
                .expect("every command is summarised");
            assert_eq!(
                entry.bindings.len(),
                meta.default_bindings.len(),
                "{} lost a binding: {entry:?}",
                meta.action_name
            );
            for binding in &entry.bindings {
                assert!(
                    !binding.display.is_empty(),
                    "{} has an empty label",
                    meta.action_name
                );
                assert_eq!(binding.source, BindingSource::Default);
            }
            assert!(
                entry.messages(NoteSeverity::Warning).is_empty(),
                "{} warns about the defaults: {:?}",
                meta.action_name,
                entry.notes
            );
        }
        assert!(summary.unattributed_warnings.is_empty());
        assert_eq!(summary.user_binding_count(), 0);
        assert_eq!(summary.warning_count(), 0);
    }

    /// The defaults rely on deeper-wins scoping, so they legitimately produce
    /// informational notes — and none of them may be a warning, because that is
    /// what the load-time toast is driven from.
    #[test]
    fn the_defaults_note_where_a_panel_wins_a_keystroke() {
        let summary = defaults();
        let infos = summary.infos(CommandId::Search);
        assert!(
            infos.iter().any(|message| {
                message.contains("sidebar::FilterBranches") && message.contains("the sidebar")
            }),
            "{infos:?}"
        );
        assert!(summary
            .infos(CommandId::Cancel)
            .iter()
            .any(|message| message.contains("graph::GraphCancel")));

        // Info notes must not inflate the "you have a problem" count.
        assert_eq!(summary.warning_count(), 0);
        assert!(
            summary
                .commands()
                .iter()
                .any(|entry| !entry.messages(NoteSeverity::Info).is_empty()),
            "the defaults should have at least one info note"
        );
    }

    #[test]
    fn a_command_with_two_keystrokes_lists_both() {
        assert_eq!(
            defaults().display(CommandId::UnstageAll).as_deref(),
            Some("Ctrl+Shift+S or Ctrl+U")
        );
    }

    /// One command binding one keystroke in two contexts is one label.
    #[test]
    fn keystrokes_bound_twice_in_different_contexts_show_once() {
        assert_eq!(
            defaults().display(CommandId::SelectNext).as_deref(),
            Some("Down or J")
        );
    }

    #[test]
    fn an_unbound_command_has_no_label() {
        let summary = defaults();
        assert_eq!(summary.display(CommandId::Pull), None);
        assert!(summary
            .command(CommandId::Pull)
            .expect("Pull is summarised")
            .bindings
            .is_empty());
    }

    #[test]
    fn a_user_binding_is_labelled_and_marked() {
        let summary = with_user(&[user("ctrl-alt-p", "Workspace", "rgitui::Pull")]);
        assert_eq!(
            summary.display(CommandId::Pull).as_deref(),
            Some("Ctrl+Alt+P")
        );
        assert!(summary.is_user_defined(CommandId::Pull));
        assert_eq!(summary.user_binding_count(), 1);
        // The defaults it did not touch are untouched.
        assert!(!summary.is_user_defined(CommandId::Fetch));
    }

    /// Rebinding a default keystroke moves the label to the new command and
    /// tells the old one where its keystroke went.
    #[test]
    fn rebinding_a_keystroke_moves_the_label() {
        let summary = with_user(&[user("secondary-s", "Workspace && !modal", "rgitui::Commit")]);

        let commit = summary
            .command(CommandId::Commit)
            .expect("Commit is summarised");
        assert!(commit
            .bindings
            .iter()
            .any(|binding| binding.display == "Ctrl+S" && binding.is_user_defined()));

        assert_eq!(summary.display(CommandId::StageAll), None);
        let warnings = summary.warnings(CommandId::StageAll);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Ctrl+S"), "{warnings:?}");
        assert!(warnings[0].contains("keymap.json"), "{warnings:?}");
    }

    #[test]
    fn an_unbind_removes_the_label_and_says_so() {
        let summary = with_user(&[user("secondary-s", "Workspace && !modal", NO_ACTION)]);
        assert_eq!(summary.display(CommandId::StageAll), None);
        let warnings = summary.warnings(CommandId::StageAll);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("was removed by keymap.json"),
            "{warnings:?}"
        );
    }

    /// The headline: the user must be able to see which of their bindings was
    /// ignored and why, without reading a log.
    #[test]
    fn a_conflict_is_reported_against_the_command_that_lost() {
        let summary = with_user(&[
            user("ctrl-alt-9", "Workspace", "rgitui::Fetch"),
            user("ctrl-alt-9", "Workspace", "rgitui::Pull"),
        ]);

        let warnings = summary.warnings(CommandId::Fetch);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("ctrl-alt-9"), "{warnings:?}");
        assert!(warnings[0].contains("rgitui::Fetch"), "{warnings:?}");
        assert!(warnings[0].contains("is ignored"), "{warnings:?}");
        // Fetch keeps its own default; only the extra binding was dropped.
        assert_eq!(
            summary.display(CommandId::Fetch).as_deref(),
            Some("Ctrl+Shift+R")
        );
        assert_eq!(
            summary.display(CommandId::Pull).as_deref(),
            Some("Ctrl+Alt+9")
        );
        assert_eq!(summary.warning_count(), 1);
    }

    #[test]
    fn a_chord_shadowed_by_its_prefix_is_reported() {
        let summary = with_user(&[
            user("ctrl-alt-k ctrl-alt-o", "Workspace", "rgitui::OpenRepo"),
            user("ctrl-alt-k", "Workspace", "rgitui::Pull"),
        ]);
        let warnings = summary.warnings(CommandId::Pull);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("shadows the chord"), "{warnings:?}");
        assert_eq!(summary.display(CommandId::Pull), None);
        // The default comes first: defaults are applied before user bindings.
        assert_eq!(
            summary.display(CommandId::OpenRepo).as_deref(),
            Some("Ctrl+O or Ctrl+Alt+K Ctrl+Alt+O")
        );
    }

    #[test]
    fn a_binding_for_an_unknown_action_is_not_silently_dropped() {
        let summary = with_user(&[
            user("ctrl-alt-9", "Workspace", "rgitui::Nonexistent"),
            user("ctrl-alt-9", "Workspace", "rgitui::Pull"),
        ]);
        assert_eq!(summary.unattributed_warnings.len(), 1, "{summary:?}");
        assert!(summary.unattributed_warnings[0].contains("rgitui::Nonexistent"));
        assert!(summary.warning_count() > 0);
    }

    #[test]
    fn groups_follow_the_registry_views_and_put_bound_commands_first() {
        let groups = defaults().groups();
        let views: Vec<&str> = groups.iter().map(|group| group.view).collect();
        assert_eq!(views.first(), Some(&"Workspace"));
        assert!(views.contains(&"DiffViewer"), "{views:?}");

        let total: usize = groups.iter().map(|group| group.commands.len()).sum();
        assert_eq!(
            total,
            ALL_COMMANDS.len(),
            "every command is in exactly one group"
        );

        for group in &groups {
            assert!(!group.description().is_empty());
            let first_unbound = group
                .commands
                .iter()
                .position(|entry| entry.bindings.is_empty());
            if let Some(first_unbound) = first_unbound {
                assert!(
                    group.commands[first_unbound..]
                        .iter()
                        .all(|entry| entry.bindings.is_empty()),
                    "{}: a bound command follows an unbound one",
                    group.view
                );
            }
        }
    }

    #[test]
    fn a_group_description_names_its_key_contexts() {
        let groups = defaults().groups();
        let diff = groups
            .iter()
            .find(|group| group.view == "DiffViewer")
            .expect("the diff viewer is a group");
        assert!(
            diff.description()
                .contains("DiffViewer && !modal && !TextInput"),
            "{}",
            diff.description()
        );
    }
}
