//! Reads `keymap.json` and turns it, together with the registry defaults, into
//! gpui key bindings.
//!
//! # File format
//!
//! Zed-shaped: an array of sections, each with a context predicate and a map of
//! keystrokes to action names. Comments and trailing commas are allowed
//! (parsed with `serde_json_lenient`, the same crate Zed uses).
//!
//! ```jsonc
//! [
//!   {
//!     "context": "Workspace && !modal",
//!     "bindings": {
//!       // A plain action name.
//!       "ctrl-alt-s": "rgitui::StageAll",
//!       // An action name plus JSON input.
//!       "ctrl-alt-r": ["rgitui::Refresh", {}],
//!       // `null` removes the binding entirely.
//!       "ctrl-s": null
//!     }
//!   }
//! ]
//! ```
//!
//! # Precedence
//!
//! Defaults are collected first, then the user's file, and gpui prefers the
//! binding added last — so the user always wins. Anything ambiguous *within*
//! one source is reported by [`conflict::detect_conflicts`] and dropped rather
//! than silently shadowed.
//!
//! # Error handling
//!
//! Every problem is accumulated: one unparseable context predicate, unknown
//! action name or malformed keystroke does not discard the rest of the file.
//! The collected messages are returned so the workspace can toast them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, KeyBinding, KeyBindingContextPredicate, SharedString};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::conflict::{self, BindingSource, BindingSpec, ConflictReport, NO_ACTION};
use super::display::KeystrokeStyle;
use super::registry::ALL_COMMANDS;
use super::summary::KeymapSummary;

/// A keystroke-to-action map that keeps the order the entries were written in.
///
/// Order is load-bearing: within one section the last binding on a keystroke
/// wins, so the entries must reach gpui in file order.
#[derive(Debug, Default)]
struct OrderedBindings(Vec<(String, serde_json::Value)>);

impl<'de> Deserialize<'de> for OrderedBindings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OrderedBindingsVisitor;

        impl<'de> Visitor<'de> for OrderedBindingsVisitor {
            type Value = OrderedBindings;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a map of keystrokes to action names")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((keystrokes, action)) =
                    map.next_entry::<String, serde_json::Value>()?
                {
                    entries.push((keystrokes, action));
                }
                Ok(OrderedBindings(entries))
            }
        }

        deserializer.deserialize_map(OrderedBindingsVisitor)
    }
}

/// One section of `keymap.json`.
#[derive(Debug, Default, Deserialize)]
struct KeymapSection {
    /// Context predicate. Empty means the bindings apply everywhere.
    #[serde(default)]
    context: String,
    /// Position-equivalent keystrokes for non-QWERTY layouts (macOS only).
    #[serde(default)]
    use_key_equivalents: bool,
    /// Keystroke to action mapping, in file order.
    #[serde(default)]
    bindings: Option<OrderedBindings>,
}

/// A candidate binding plus the action input needed to build it.
#[derive(Debug, Clone)]
struct PendingBinding {
    spec: BindingSpec,
    input: Option<serde_json::Value>,
    use_key_equivalents: bool,
}

/// The result of assembling the default and user keymaps.
#[derive(Debug, Default)]
pub struct LoadedKeymap {
    /// Bindings to hand to [`gpui::App::bind_keys`], in precedence order.
    pub bindings: Vec<KeyBinding>,
    /// Problems the user should fix: parse errors, unknown actions, conflicts.
    pub problems: Vec<String>,
    /// The same bindings, labelled for display. Everything the UI shows about a
    /// shortcut is read from here, so no surface can drift from the keymap.
    pub summary: KeymapSummary,
}

/// Path of the user keymap, alongside `settings.json` in rgitui's config dir.
pub fn keymap_path() -> PathBuf {
    rgitui_settings::keymap_path()
}

/// The starter `keymap.json` written when the user first opens the file.
///
/// The example bindings name real actions at their real default keystrokes,
/// taken from the registry, so the stub cannot advertise something that does not
/// exist. It is commented out, so writing it changes no binding.
pub fn keymap_stub() -> String {
    let example = ALL_COMMANDS
        .iter()
        .find(|meta| !meta.default_bindings.is_empty())
        .expect("the registry binds at least one command");
    let (keystrokes, context) = example.default_bindings[0];

    format!(
        "// rgitui keybindings. Saving this file reloads them immediately.\n\
         //\n\
         // Every action name is listed in docs/KEYBINDINGS.md. For completion\n\
         // while editing, point your editor at docs/keymap.schema.json for this\n\
         // filename — the file itself is a JSON array, so it has nowhere to put\n\
         // a \"$schema\" key.\n\
         //\n\
         // `secondary` is the platform's primary modifier: cmd on macOS, ctrl\n\
         // elsewhere. A binding you add wins over the default it replaces.\n\
         [\n  \
         {{\n    \
         \"context\": \"{context}\",\n    \
         \"bindings\": {{\n      \
         // Bind an action to a keystroke.\n      \
         // \"ctrl-alt-s\": \"{action}\",\n      \
         // Remove a default binding.\n      \
         // \"{keystrokes}\": null\n    \
         }}\n  \
         }}\n\
         ]\n",
        action = example.action_name,
    )
}

/// Makes sure `keymap.json` exists, creating it with [`keymap_stub`] if not.
///
/// Returns the path either way, so the caller can hand it to an editor.
pub fn ensure_keymap_file() -> std::io::Result<PathBuf> {
    let path = keymap_path();
    if path.exists() {
        return Ok(path);
    }
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(&path, keymap_stub())?;
    Ok(path)
}

/// The default bindings declared by `commands!`, in registry order.
///
/// Pure, so the default set can be validated without an `App`.
pub fn default_specs() -> Vec<BindingSpec> {
    ALL_COMMANDS
        .iter()
        .flat_map(|meta| {
            meta.default_bindings.iter().map(|(keystrokes, context)| {
                BindingSpec::default_binding(keystrokes, context, meta.action_name)
            })
        })
        .collect()
}

/// Parses `keymap.json` content into candidate bindings, accumulating errors.
///
/// A section whose `context` does not parse is skipped, but the rest of the file
/// still loads.
fn parse_user_specs(content: &str) -> (Vec<PendingBinding>, Vec<String>) {
    let mut problems = Vec::new();

    if content.trim().is_empty() {
        return (Vec::new(), problems);
    }

    let sections: Vec<KeymapSection> = match serde_json_lenient::from_str(content) {
        Ok(sections) => sections,
        Err(error) => {
            problems.push(format!(
                "keymap.json is not valid JSON, so no custom keybindings were loaded: {error}. \
                 Fix the syntax error and save the file to reload."
            ));
            return (Vec::new(), problems);
        }
    };

    let mut pending = Vec::new();
    for section in &sections {
        let context = (!section.context.trim().is_empty()).then(|| section.context.clone());

        if let Some(context) = context.as_deref() {
            if let Err(error) = KeyBindingContextPredicate::parse(context) {
                problems.push(format!(
                    "keymap.json: the context `{context}` could not be parsed, so that section \
                     was skipped: {error}."
                ));
                continue;
            }
        }

        for (keystrokes, action) in section.bindings.iter().flat_map(|bindings| &bindings.0) {
            match parse_action(action) {
                Ok((name, input)) => pending.push(PendingBinding {
                    spec: BindingSpec {
                        keystrokes: keystrokes.clone(),
                        context: context.clone(),
                        action: name,
                        source: BindingSource::User,
                    },
                    input,
                    use_key_equivalents: section.use_key_equivalents,
                }),
                Err(error) => problems.push(format!(
                    "keymap.json: the binding for `{keystrokes}` was skipped: {error}"
                )),
            }
        }
    }

    (pending, problems)
}

/// Splits a `keymap.json` action value into an action name and optional input.
///
/// `null` becomes gpui's `NoAction`, which removes the binding.
fn parse_action(action: &serde_json::Value) -> Result<(String, Option<serde_json::Value>), String> {
    match action {
        serde_json::Value::Null => Ok((NO_ACTION.to_owned(), None)),
        serde_json::Value::String(name) => Ok((name.clone(), None)),
        serde_json::Value::Array(items) => {
            let [serde_json::Value::String(name), input] = items.as_slice() else {
                return Err(
                    "expected a two-element array of `[\"namespace::Action\", input]`.".to_owned(),
                );
            };
            Ok((name.clone(), Some(input.clone())))
        }
        other => Err(format!(
            "expected an action name, a two-element `[name, input]` array, or `null`, \
             but found `{other}`."
        )),
    }
}

/// Checks whether an action name (with its optional JSON input) can be built.
///
/// Injected so the validation pass stays pure and testable without an `App`.
type ActionResolver<'a> = &'a dyn Fn(&str, Option<&serde_json::Value>) -> Result<(), String>;

/// Which candidate bindings survive validation, and what went wrong with the rest.
#[derive(Debug, Default, PartialEq, Eq)]
struct BindingPlan {
    /// Indices into the candidate list, in precedence order.
    keep: Vec<usize>,
    /// One message per rejected binding.
    problems: Vec<String>,
}

/// Validates every candidate binding that survived conflict resolution.
///
/// Pure apart from `resolve_action`. Each rejection is recorded and the
/// remaining candidates are still planned, so one bad entry never aborts the
/// load.
fn plan_bindings(
    pending: &[PendingBinding],
    report: &ConflictReport,
    resolve_action: ActionResolver<'_>,
) -> BindingPlan {
    let mut plan = BindingPlan::default();

    for (index, entry) in pending.iter().enumerate() {
        if !report.is_kept(index) {
            continue;
        }

        if let Err(error) = resolve_action(&entry.spec.action, entry.input.as_ref()) {
            plan.problems.push(format!(
                "{}: the binding for `{}` was skipped: {error} \
                 See docs/KEYBINDINGS.md for the list of action names.",
                entry.spec.source, entry.spec.keystrokes
            ));
            continue;
        }

        if let Some(context) = entry.spec.context.as_deref() {
            if let Err(error) = KeyBindingContextPredicate::parse(context) {
                plan.problems.push(format!(
                    "{}: the context `{context}` could not be parsed, so the binding for `{}` \
                     was skipped: {error}.",
                    entry.spec.source, entry.spec.keystrokes
                ));
                continue;
            }
        }

        if let Err(error) = parse_keystrokes(&entry.spec.keystrokes) {
            plan.problems.push(format!(
                "{}: the binding for `{}` was skipped: {error}",
                entry.spec.source, entry.spec.keystrokes
            ));
            continue;
        }

        plan.keep.push(index);
    }

    plan
}

/// Validates a whitespace-separated keystroke sequence.
fn parse_keystrokes(keystrokes: &str) -> Result<(), String> {
    if keystrokes.split_whitespace().next().is_none() {
        return Err("the keystroke is empty.".to_owned());
    }
    for keystroke in keystrokes.split_whitespace() {
        gpui::Keystroke::parse(keystroke).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Assembles the default bindings and the given user keymap content.
///
/// Exposed separately from [`load`] so tests can drive it with inline content.
pub fn load_from_content(content: &str, cx: &App) -> LoadedKeymap {
    let (user_pending, mut problems) = parse_user_specs(content);

    let mut pending: Vec<PendingBinding> = default_specs()
        .into_iter()
        .map(|spec| PendingBinding {
            spec,
            input: None,
            use_key_equivalents: false,
        })
        .collect();
    pending.extend(user_pending);

    let specs: Vec<BindingSpec> = pending.iter().map(|entry| entry.spec.clone()).collect();
    let report = conflict::detect_conflicts(&specs);
    problems.extend(report.messages());

    let plan = plan_bindings(&pending, &report, &|name, input| {
        cx.build_action(name, input.cloned())
            .map(|_| ())
            .map_err(|error| format!("{error}."))
    });
    problems.extend(plan.problems);

    // Derived from the specs that are about to be bound, so the labels the UI
    // shows and the bindings gpui matches cannot disagree.
    let summary = KeymapSummary::build(&specs, &plan.keep, &report, KeystrokeStyle::platform());

    let bindings = build_bindings(&pending, &plan.keep, &mut problems, cx);
    LoadedKeymap {
        bindings,
        problems,
        summary,
    }
}

/// Constructs the gpui bindings for the planned candidates.
fn build_bindings(
    pending: &[PendingBinding],
    keep: &[usize],
    problems: &mut Vec<String>,
    cx: &App,
) -> Vec<KeyBinding> {
    let mut bindings = Vec::with_capacity(keep.len());

    for entry in keep.iter().map(|&index| &pending[index]) {
        // `plan_bindings` already proved the name, context and keystrokes are good.
        let Ok(action) = cx.build_action(&entry.spec.action, entry.input.clone()) else {
            continue;
        };
        let context = match entry.spec.context.as_deref() {
            Some(context) => KeyBindingContextPredicate::parse(context).ok().map(Rc::new),
            None => None,
        };
        let input = entry
            .input
            .as_ref()
            .map(|input| SharedString::from(input.to_string()));

        match KeyBinding::load(
            &entry.spec.keystrokes,
            action,
            context,
            entry.use_key_equivalents,
            input,
            cx.keyboard_mapper().as_ref(),
        ) {
            Ok(binding) => bindings.push(binding),
            Err(error) => problems.push(format!(
                "{}: the binding for `{}` was skipped: {error}",
                entry.spec.source, entry.spec.keystrokes
            )),
        }
    }

    bindings
}

/// Reads the user keymap from disk, returning empty content when absent.
fn read_user_keymap(path: &Path) -> (String, Vec<String>) {
    match std::fs::read_to_string(path) {
        Ok(content) => (content, Vec::new()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (String::new(), Vec::new()),
        Err(error) => (
            String::new(),
            vec![format!(
                "Could not read {}: {error}. Custom keybindings were not applied.",
                path.display()
            )],
        ),
    }
}

/// Loads the defaults plus the user's `keymap.json` from disk.
pub fn load(cx: &App) -> LoadedKeymap {
    let (content, mut problems) = read_user_keymap(&keymap_path());
    let mut loaded = load_from_content(&content, cx);
    problems.extend(loaded.problems);
    loaded.problems = problems;
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_keystroke_parses() {
        for spec in default_specs() {
            for keystroke in spec.keystrokes.split_whitespace() {
                assert!(
                    gpui::Keystroke::parse(keystroke).is_ok(),
                    "default keystroke `{keystroke}` for {} does not parse",
                    spec.action
                );
            }
        }
    }

    #[test]
    fn every_default_context_parses() {
        for spec in default_specs() {
            let context = spec.context.as_deref().expect("defaults declare a context");
            assert!(
                KeyBindingContextPredicate::parse(context).is_ok(),
                "default context `{context}` for {} does not parse",
                spec.action
            );
        }
    }

    /// The stub is written into `keymap.json`, so it must load cleanly — an
    /// example that does not parse would greet the user with an error toast.
    #[test]
    fn the_starter_keymap_parses_and_binds_nothing() {
        let stub = keymap_stub();
        let (pending, problems) = parse_user_specs(&stub);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(
            pending.is_empty(),
            "the starter keymap must leave every binding at its default: {pending:?}"
        );
        // The commented examples name a real action and a real default keystroke.
        let example = ALL_COMMANDS
            .iter()
            .find(|meta| !meta.default_bindings.is_empty())
            .expect("the registry binds at least one command");
        assert!(stub.contains(example.action_name), "{stub}");
        assert!(stub.contains(example.default_bindings[0].0), "{stub}");
    }

    #[test]
    fn the_default_keymap_has_no_conflicts() {
        let report = conflict::detect_conflicts(&default_specs());
        assert!(
            report.conflicts.is_empty(),
            "default keymap has conflicts:\n{}",
            report.messages().join("\n")
        );
    }

    #[test]
    fn parses_jsonc_with_comments_and_trailing_commas() {
        let content = r#"
        [
          // A section.
          {
            "context": "Workspace",
            "bindings": {
              "ctrl-alt-s": "rgitui::StageAll", /* inline */
            },
          },
        ]
        "#;
        let (pending, problems) = parse_user_specs(content);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].spec.action, "rgitui::StageAll");
        assert_eq!(pending[0].spec.context.as_deref(), Some("Workspace"));
    }

    #[test]
    fn bindings_keep_their_file_order() {
        let content = r#"[{ "bindings": {
            "ctrl-1": "rgitui::Fetch",
            "ctrl-2": "rgitui::Pull",
            "ctrl-3": "rgitui::Push"
        }}]"#;
        let (pending, _) = parse_user_specs(content);
        let keystrokes: Vec<&str> = pending
            .iter()
            .map(|entry| entry.spec.keystrokes.as_str())
            .collect();
        assert_eq!(keystrokes, ["ctrl-1", "ctrl-2", "ctrl-3"]);
    }

    #[test]
    fn null_becomes_an_unbind() {
        let content = r#"[{ "context": "Workspace", "bindings": { "ctrl-s": null } }]"#;
        let (pending, problems) = parse_user_specs(content);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].spec.action, NO_ACTION);
    }

    #[test]
    fn an_action_may_carry_json_input() {
        let content = r#"[{ "bindings": { "ctrl-s": ["rgitui::StageAll", { "all": true }] } }]"#;
        let (pending, problems) = parse_user_specs(content);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(pending[0].spec.action, "rgitui::StageAll");
        assert!(pending[0].input.is_some());
    }

    #[test]
    fn a_user_binding_overrides_a_default_and_is_ordered_last() {
        let content = r#"[{
            "context": "Workspace && !modal",
            "bindings": { "secondary-s": "rgitui::Commit" }
        }]"#;
        let (user, problems) = parse_user_specs(content);
        assert!(problems.is_empty(), "{problems:?}");

        let mut specs = default_specs();
        let default_index = specs
            .iter()
            .position(|spec| spec.action == "rgitui::StageAll")
            .expect("StageAll is bound by default");
        specs.extend(user.iter().map(|entry| entry.spec.clone()));
        let user_index = specs.len() - 1;

        assert!(
            default_index < user_index,
            "the user binding must come last"
        );
        let report = conflict::detect_conflicts(&specs);
        assert!(
            report.conflicts.is_empty(),
            "overriding a default must not be reported: {:?}",
            report.messages()
        );
        assert!(report.is_kept(default_index));
        assert!(report.is_kept(user_index));
    }

    #[test]
    fn a_bad_section_context_does_not_discard_the_rest_of_the_file() {
        let content = r#"[
            { "context": "Workspace &&", "bindings": { "ctrl-1": "rgitui::Fetch" } },
            { "context": "Workspace",    "bindings": { "ctrl-2": "rgitui::Pull" } }
        ]"#;
        let (pending, problems) = parse_user_specs(content);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("could not be parsed"), "{problems:?}");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].spec.action, "rgitui::Pull");
    }

    #[test]
    fn a_malformed_action_value_does_not_discard_the_rest_of_the_section() {
        let content = r#"[{ "bindings": {
            "ctrl-1": 42,
            "ctrl-2": "rgitui::Pull"
        }}]"#;
        let (pending, problems) = parse_user_specs(content);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("ctrl-1"), "{problems:?}");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].spec.action, "rgitui::Pull");
    }

    #[test]
    fn a_syntax_error_is_reported_once() {
        let (pending, problems) = parse_user_specs("[{ \"context\": }]");
        assert!(pending.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("not valid JSON"), "{problems:?}");
    }

    #[test]
    fn an_empty_file_loads_the_defaults_only() {
        let (pending, problems) = parse_user_specs("   \n  ");
        assert!(pending.is_empty());
        assert!(problems.is_empty());
    }

    /// Stands in for `App::build_action`: every registry action name resolves,
    /// plus `zed::NoAction`.
    fn resolve_known_actions(name: &str, _input: Option<&serde_json::Value>) -> Result<(), String> {
        let known = name == NO_ACTION || ALL_COMMANDS.iter().any(|meta| meta.action_name == name);
        if known {
            Ok(())
        } else {
            Err(format!("didn't find an action named `{name}`."))
        }
    }

    fn plan(content: &str) -> BindingPlan {
        let (user, _) = parse_user_specs(content);
        let mut pending: Vec<PendingBinding> = default_specs()
            .into_iter()
            .map(|spec| PendingBinding {
                spec,
                input: None,
                use_key_equivalents: false,
            })
            .collect();
        pending.extend(user);
        let specs: Vec<BindingSpec> = pending.iter().map(|entry| entry.spec.clone()).collect();
        let report = conflict::detect_conflicts(&specs);
        plan_bindings(&pending, &report, &resolve_known_actions)
    }

    #[test]
    fn every_default_binding_survives_planning() {
        let plan = plan("");
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(plan.keep.len(), default_specs().len());
    }

    /// The planning tests above substitute [`resolve_known_actions`] for
    /// `App::build_action`, so they would still pass if a generated action name
    /// did not match what gpui actually registered — and every binding would
    /// then fail to load at runtime, leaving the app with no shortcuts and a
    /// toast per command. This drives the real resolver instead.
    #[gpui::test]
    fn the_defaults_load_against_the_real_action_registry(cx: &mut gpui::TestAppContext) {
        let loaded = cx.update(|cx| load_from_content("", cx));

        assert!(
            loaded.problems.is_empty(),
            "the shipped defaults do not load cleanly: {:?}",
            loaded.problems
        );
        assert_eq!(
            loaded.bindings.len(),
            default_specs().len(),
            "some default bindings were dropped on the way to gpui"
        );
    }

    /// A user binding naming an action gpui does not know must be reported and
    /// skipped, not silently dropped along with the rest of the file.
    #[gpui::test]
    fn an_unknown_user_action_is_reported_by_the_real_resolver(cx: &mut gpui::TestAppContext) {
        let loaded = cx.update(|cx| {
            load_from_content(
                r#"[{ "context": "Workspace", "bindings": {
                    "ctrl-alt-1": "rgitui::NoSuchCommand"
                }}]"#,
                cx,
            )
        });

        assert_eq!(loaded.problems.len(), 1, "{:?}", loaded.problems);
        assert!(
            loaded.problems[0].contains("rgitui::NoSuchCommand"),
            "{:?}",
            loaded.problems
        );
        assert_eq!(
            loaded.bindings.len(),
            default_specs().len(),
            "the defaults must survive one bad user binding"
        );
    }

    #[test]
    fn an_unknown_action_name_is_reported_and_the_load_continues() {
        let plan = plan(
            r#"[{ "context": "Workspace", "bindings": {
                "ctrl-alt-1": "rgitui::NoSuchCommand",
                "ctrl-alt-2": "rgitui::Pull"
            }}]"#,
        );
        assert_eq!(plan.problems.len(), 1, "{:?}", plan.problems);
        assert!(
            plan.problems[0].contains("rgitui::NoSuchCommand"),
            "{:?}",
            plan.problems
        );
        // The defaults plus the one good user binding.
        assert_eq!(plan.keep.len(), default_specs().len() + 1);
    }

    #[test]
    fn an_unparseable_keystroke_is_reported_and_the_load_continues() {
        let plan = plan(
            r#"[{ "context": "Workspace", "bindings": {
                "ctrl-a-b": "rgitui::Fetch",
                "ctrl-alt-2": "rgitui::Pull"
            }}]"#,
        );
        assert_eq!(plan.problems.len(), 1, "{:?}", plan.problems);
        assert!(plan.problems[0].contains("ctrl-a-b"), "{:?}", plan.problems);
        assert_eq!(plan.keep.len(), default_specs().len() + 1);
    }

    #[test]
    fn an_unbind_is_planned_so_gpui_can_apply_it() {
        let plan =
            plan(r#"[{ "context": "Workspace && !modal", "bindings": { "secondary-s": null } }]"#);
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(plan.keep.len(), default_specs().len() + 1);
    }

    #[test]
    fn a_conflicting_pair_of_user_bindings_drops_the_earlier_one() {
        let plan = plan(
            r#"[{ "context": "Workspace", "bindings": {
                "ctrl-alt-9": "rgitui::Fetch"
            }}, { "context": "Workspace", "bindings": {
                "ctrl-alt-9": "rgitui::Pull"
            }}]"#,
        );
        // One of the two user bindings was dropped, so only one was added.
        assert_eq!(plan.keep.len(), default_specs().len() + 1);
        let last = *plan.keep.last().expect("at least one binding");
        assert_eq!(
            last,
            default_specs().len() + 1,
            "the later binding must win"
        );
    }
}
