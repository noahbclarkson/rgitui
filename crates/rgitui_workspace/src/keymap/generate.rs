//! Generates the committed keybinding artifacts from the registry.
//!
//! Two files are derived from `commands!` and checked in:
//!
//! * `docs/KEYBINDINGS.md` — the user-facing reference, including the
//!   nested-context overlaps [`super::shadow`] finds in the defaults,
//! * `docs/keymap.schema.json` — a JSON Schema enumerating every action name so
//!   editors can complete `keymap.json`.
//!
//! Both are golden-tested: the tests below regenerate the content and compare it
//! with the committed file, failing with a line diff when it is stale. Run
//! `cargo test -p rgitui_workspace keymap::generate` after changing `commands!`,
//! then copy the regenerated content in (the failure message tells you which
//! file drifted).

use std::fmt::Write as _;

use super::registry::{CommandMeta, ALL_COMMANDS};

/// Header written into both generated files so nobody hand-edits them.
const GENERATED_NOTICE: &str = "Generated from the `commands!` declaration in \
     `crates/rgitui_workspace/src/keymap/registry.rs`. Do not edit by hand.";

/// The distinct view names in the registry, in declaration order.
fn views() -> Vec<&'static str> {
    let mut views: Vec<&'static str> = Vec::new();
    for meta in ALL_COMMANDS {
        if !views.contains(&meta.view) {
            views.push(meta.view);
        }
    }
    views
}

/// Escapes a value for a Markdown table cell.
///
/// GFM splits table rows on `|` even inside a code span, so an `||` context
/// predicate would silently break the row it sits in.
fn table_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Renders a command's default keystrokes for a Markdown table cell.
fn keystroke_cell(meta: &CommandMeta) -> String {
    if meta.default_bindings.is_empty() {
        return "_unbound_".to_owned();
    }
    meta.default_bindings
        .iter()
        .map(|(keystroke, _)| format!("`{}`", table_cell(keystroke)))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Renders the key contexts a command's default bindings are scoped to.
///
/// Keystrokes of one command may differ here — a vim-style letter usually
/// carries `&& !TextInput` where its arrow-key twin does not — so each distinct
/// context is listed once, in binding order.
fn context_cell(meta: &CommandMeta) -> String {
    let mut contexts: Vec<&'static str> = Vec::new();
    for (_, context) in meta.default_bindings {
        if !contexts.contains(context) {
            contexts.push(context);
        }
    }
    if contexts.is_empty() {
        return "—".to_owned();
    }
    contexts
        .iter()
        .map(|context| format!("`{}`", table_cell(context)))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Renders `docs/KEYBINDINGS.md`.
pub fn keybindings_markdown() -> String {
    let mut out = String::new();

    out.push_str("# Keyboard shortcuts\n\n");
    out.push_str("<!-- ");
    out.push_str(GENERATED_NOTICE);
    out.push_str(" -->\n\n");
    out.push_str(
        "Every shortcut below is rebindable, and press `?` in rgitui to see the ones actually \
         in force — that reference is generated from the same declaration as this page, so it \
         follows your own keybindings rather than the defaults.\n\n",
    );
    out.push_str(
        "`secondary` is the platform's primary modifier: `cmd` on macOS, `ctrl` everywhere \
         else. Commands marked _unbound_ have no default keystroke and are reached from the \
         command palette (`secondary-shift-p`).\n\n",
    );
    out.push_str("## Customising\n\n");
    out.push_str(
        "Keybindings live in `keymap.json`, next to `settings.json` in rgitui's config \
         directory:\n\n\
         | Platform | Path |\n\
         | --- | --- |\n\
         | Linux | `~/.config/rgitui/keymap.json` |\n\
         | macOS | `~/Library/Application Support/rgitui/keymap.json` |\n\
         | Windows | `%APPDATA%\\rgitui\\keymap.json` |\n\n\
         Run the **Open keymap.json** command from the palette, or use the button in the \
         shortcut reference, to create the file with a commented example already in it and \
         open it in your editor.\n\n",
    );
    out.push_str(
        "```jsonc\n\
         [\n  \
         {\n    \
         \"context\": \"Workspace && !modal\",\n    \
         \"bindings\": {\n      \
         // Rebind staging.\n      \
         \"ctrl-alt-s\": \"rgitui::StageAll\",\n      \
         // Remove a default binding.\n      \
         \"secondary-s\": null\n    \
         }\n  \
         }\n\
         ]\n\
         ```\n\n",
    );
    out.push_str(
        "The file is reloaded when you save it. Bindings you add win over the defaults. \
         Two bindings on the same keystroke in overlapping contexts, or a binding that \
         shadows the prefix of a chord, are reported as a toast and the losing binding is \
         dropped rather than silently ignored.\n\n",
    );
    out.push_str(
        "`docs/keymap.schema.json` lists every action name with its description; associate it \
         with `keymap.json` in your editor's JSON schema settings for completion and hovers.\n",
    );

    for view in views() {
        let _ = write!(out, "\n## {view}\n\n");
        out.push_str("| Keystroke | Context | Action | Description |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for meta in ALL_COMMANDS.iter().filter(|meta| meta.view == view) {
            let _ = writeln!(
                out,
                "| {} | {} | `{}` | {} |",
                keystroke_cell(meta),
                context_cell(meta),
                meta.action_name,
                meta.description()
            );
        }
    }

    let _ = write!(
        out,
        "\n## Key contexts\n\n\
         A binding fires when its context matches somewhere on the path from the \
         focused element to the window root, and the deepest matching binding \
         wins. That is what lets one keystroke mean different things in different \
         panels without any `if focused` checks.\n\n\
         | Context | Set on |\n\
         | --- | --- |\n\
         | `Workspace` | the workspace root, so it is always in scope |\n\
         | `SettingsWindow` | the settings window root |\n\
         | `modal` | added to the workspace root while any overlay or dialog is open |\n\
         | `TextInput` | any text field, so single-key shortcuts do not steal typing |\n\
         | `List` | every panel, picker and dialog that owns a row selection |\n\
         | a view name | the panel, overlay or dialog of that name — see the tables above |\n\
         \n\
         Contexts combine with `&&`, `||` and `!`, and `>` matches a descendant. \
         `!TextInput` is false whenever a text field is anywhere on the focus \
         path, which is why the vim-style letters carry it and the arrow keys \
         do not.\n"
    );

    out.push_str(&shadowing_section());

    out
}

/// Renders the "Where a panel wins a keystroke" section.
///
/// Derived from the same analysis the shortcut reference shows on the affected
/// row, so the page cannot claim a keystroke works somewhere it does not.
fn shadowing_section() -> String {
    let specs = super::loader::default_specs();
    let shadows = super::shadow::detect_shadowing(&specs);

    let mut out = String::from("\n## Where a panel wins a keystroke\n\n");
    out.push_str(
        "The deepest match wins, so a few of the shortcuts above cannot be reached while a \
         particular panel has focus. That is intended — the alternative would be a panel \
         unable to give a letter its own meaning. Both bindings stay active; only one of them \
         is what the keystroke does in that panel.\n\n",
    );

    if shadows.is_empty() {
        out.push_str("The defaults currently have no such overlaps.\n");
        return out;
    }

    out.push_str("| Keystroke | Runs | While focused | So this is out of reach |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for shadow in &shadows {
        let node = super::registry::context_node(shadow.context);
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | `{}` |",
            table_cell(&specs[shadow.inner].keystrokes),
            specs[shadow.inner].action,
            node.map_or(shadow.context, |node| node.label),
            specs[shadow.outer].action,
        );
    }
    out
}

/// Renders `docs/keymap.schema.json`.
pub fn keymap_json_schema() -> String {
    let action_names: Vec<serde_json::Value> = std::iter::once(json_action_name(
        super::conflict::NO_ACTION,
        "Remove the binding this keystroke would otherwise have.",
    ))
    .chain(ALL_COMMANDS.iter().map(|meta| {
        json_action_name(
            meta.action_name,
            &format!("{} Command id: `{}`.", meta.description(), meta.id.as_str()),
        )
    }))
    .collect();

    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://raw.githubusercontent.com/noahbclarkson/rgitui/main/docs/keymap.schema.json",
        "title": "rgitui keymap",
        "description": GENERATED_NOTICE,
        "type": "array",
        "items": { "$ref": "#/$defs/section" },
        "$defs": {
            "section": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "context": {
                        "type": "string",
                        "description":
                            "When these bindings are active, e.g. `Workspace && !modal`. \
                             Combine identifiers with `&&`, `||` and `!`; `>` matches a \
                             descendant. Omit to bind everywhere.",
                    },
                    "use_key_equivalents": {
                        "type": "boolean",
                        "description":
                            "Interpret keystrokes by their position on a QWERTY keyboard. \
                             macOS only.",
                        "default": false,
                    },
                    "bindings": {
                        "type": "object",
                        "description":
                            "Keystrokes to actions. A keystroke is modifiers then a key joined \
                             by `-` (`secondary-shift-p`); separate the keystrokes of a chord \
                             with spaces (`ctrl-k ctrl-o`). Later entries win.",
                        "additionalProperties": { "$ref": "#/$defs/action" },
                    },
                },
            },
            "action": {
                "description":
                    "An action name, a two-element `[name, input]` array, or `null` to unbind.",
                "oneOf": [
                    { "$ref": "#/$defs/actionName" },
                    {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 2,
                        "items": [{ "$ref": "#/$defs/actionName" }, true],
                    },
                    { "type": "null", "description": "Remove this binding." },
                ],
            },
            "actionName": {
                "type": "string",
                "anyOf": action_names,
            },
        },
    });

    let mut out = serde_json::to_string_pretty(&schema).expect("the schema is serializable");
    out.push('\n');
    out
}

/// One `anyOf` branch pinning a single action name, carrying its description so
/// editors show it during completion.
fn json_action_name(name: &str, description: &str) -> serde_json::Value {
    serde_json::json!({ "const": name, "description": description.trim() })
}

/// Repository-relative path of the generated keybinding reference.
#[cfg(test)]
const MARKDOWN_PATH: &str = "docs/KEYBINDINGS.md";
/// Repository-relative path of the generated keymap schema.
#[cfg(test)]
const SCHEMA_PATH: &str = "docs/keymap.schema.json";

/// Setting this environment variable makes the golden tests rewrite the
/// committed artifacts instead of failing:
///
/// ```text
/// RGITUI_BLESS=1 cargo test -p rgitui_workspace keymap::generate
/// ```
///
/// Re-run the tests afterwards to confirm the files now match.
#[cfg(test)]
const BLESS_ENV: &str = "RGITUI_BLESS";

/// Renders a minimal line diff so a stale golden file is easy to fix.
#[cfg(test)]
fn line_diff(expected: &str, actual: &str) -> String {
    let expected: Vec<&str> = expected.lines().collect();
    let actual: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    for index in 0..expected.len().max(actual.len()) {
        match (expected.get(index), actual.get(index)) {
            (Some(left), Some(right)) if left == right => {}
            (left, right) => {
                let _ = writeln!(out, "line {}:", index + 1);
                let _ = writeln!(out, "  committed: {}", left.unwrap_or(&"<missing>"));
                let _ = writeln!(out, "  generated: {}", right.unwrap_or(&"<missing>"));
            }
        }
    }
    out
}

/// Fails with a diff when a committed artifact no longer matches the registry.
///
/// With [`BLESS_ENV`] set, rewrites the file instead so the artifacts can be
/// regenerated without a separate binary.
#[cfg(test)]
fn assert_golden(path: &str, committed: &str, generated: &str) {
    if std::env::var_os(BLESS_ENV).is_some() {
        let absolute = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        std::fs::write(&absolute, generated)
            .unwrap_or_else(|error| panic!("could not write {}: {error}", absolute.display()));
        return;
    }
    assert!(
        committed == generated,
        "{path} is stale — regenerate it with `{BLESS_ENV}=1 cargo test -p rgitui_workspace \
         keymap::generate`.\n\n{}",
        line_diff(committed, generated)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybindings_doc_is_up_to_date() {
        assert_golden(
            MARKDOWN_PATH,
            include_str!("../../../../docs/KEYBINDINGS.md"),
            &keybindings_markdown(),
        );
    }

    #[test]
    fn keymap_schema_is_up_to_date() {
        assert_golden(
            SCHEMA_PATH,
            include_str!("../../../../docs/keymap.schema.json"),
            &keymap_json_schema(),
        );
    }

    /// A `||` context predicate must not split the table row it sits in.
    #[test]
    fn table_cells_escape_the_pipe_that_would_split_a_row() {
        assert_eq!(
            table_cell("Workspace || SettingsWindow"),
            "Workspace \\|\\| SettingsWindow"
        );

        let markdown = keybindings_markdown();
        for line in markdown.lines().filter(|line| line.starts_with('|')) {
            let without_escapes = line.replace("\\|", "");
            assert!(
                !without_escapes.contains("||"),
                "this table row contains an unescaped `||`, which splits it into empty cells: \
                 {line}"
            );
        }
    }

    #[test]
    fn the_doc_lists_every_command_exactly_once() {
        // Only the per-view tables: the shadowing table further down names some
        // commands again on purpose.
        let markdown = keybindings_markdown();
        let tables = markdown
            .split_once("\n## Key contexts")
            .map_or(markdown.as_str(), |(tables, _)| tables);
        for meta in ALL_COMMANDS {
            let row = format!("| `{}` |", meta.action_name);
            assert_eq!(
                tables.matches(&row).count(),
                1,
                "{} appears {} times in the command tables",
                meta.action_name,
                tables.matches(&row).count()
            );
        }
    }

    /// The shadowing table is only useful if it names real actions and real
    /// contexts, so it is checked against the registry rather than eyeballed.
    #[test]
    fn the_shadowing_table_names_registry_actions() {
        let section = shadowing_section();
        let rows: Vec<&str> = section
            .lines()
            .filter(|line| line.starts_with("| `"))
            .collect();
        assert!(!rows.is_empty(), "{section}");

        for row in rows {
            let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
            let [_keystroke, inner, label, outer] = cells.as_slice() else {
                panic!("a shadowing row has the wrong shape: {row}");
            };
            for action in [inner, outer] {
                let name = action.trim_matches('`');
                assert!(
                    super::super::registry::command_for_action(name).is_some(),
                    "`{name}` in the shadowing table is not a registry action"
                );
            }
            assert!(
                super::super::registry::CONTEXT_TREE
                    .iter()
                    .any(|node| node.label == *label && !node.modal),
                "`{label}` in the shadowing table is not a non-modal element label"
            );
        }
    }

    #[test]
    fn the_schema_enumerates_every_action_name() {
        let schema: serde_json::Value =
            serde_json::from_str(&keymap_json_schema()).expect("generated schema is valid JSON");
        let names: Vec<&str> = schema["$defs"]["actionName"]["anyOf"]
            .as_array()
            .expect("anyOf is an array")
            .iter()
            .map(|branch| branch["const"].as_str().expect("const is a string"))
            .collect();

        assert!(names.contains(&super::super::conflict::NO_ACTION));
        for meta in ALL_COMMANDS {
            assert!(
                names.contains(&meta.action_name),
                "{} is missing from the schema",
                meta.action_name
            );
        }
        assert_eq!(names.len(), ALL_COMMANDS.len() + 1);
    }

    #[test]
    fn command_ids_are_discoverable_from_the_schema_descriptions() {
        let schema = keymap_json_schema();
        for id in super::super::registry::CommandId::ALL {
            let needle = format!("Command id: `{}`.", id.as_str());
            assert!(schema.contains(&needle), "{needle} is missing");
        }
    }
}
