//! The `commands!` macro — the single source of truth for rgitui commands.
//!
//! One declaration per command produces, all at once:
//!
//! * a [`CommandId`](crate::CommandId) variant,
//! * a `gpui` [`Action`](gpui::Action) unit struct in the declared namespace,
//! * the canonical keymap name (`namespace::PascalCase` — gpui panics if an
//!   action name contains `::` itself, so the namespace is the only separator),
//! * the default binding(s) — keystrokes paired with the key context each one
//!   is scoped to,
//! * the command-palette availability predicate,
//! * whether the command is offered in the command palette at all,
//! * and the doc comment, reused as the description in `docs/KEYBINDINGS.md`
//!   and in the `keymap.json` JSON schema.
//!
//! Plus, for the whole set, [`ALL_COMMANDS`](crate::keymap::ALL_COMMANDS) —
//! which drives binding, validation, doc generation and the shortcuts UI.
//!
//! # Syntax
//!
//! ```ignore
//! commands! {
//!     view Workspace in rgitui context "Workspace && !modal" {
//!         /// Toggle the command palette.
//!         CommandPalette "secondary-shift-p" in "Workspace" [hidden];
//!         /// Stage every change in the working tree.
//!         StageAll "secondary-s" if has_changes;
//!         /// Unstage everything.
//!         UnstageAll ["secondary-shift-s", "secondary-u"] if has_changes;
//!         /// Move the selection down.
//!         SelectNext ["down", "j" in "List && !TextInput"] in "List";
//!         /// Pull from the tracked remote.
//!         Pull unbound if has_remotes;
//!     }
//! }
//! ```
//!
//! * `view <Name> in <namespace> context "<predicate>"` opens a block. `<Name>`
//!   groups the commands in the generated docs; `context` is the default key
//!   context for every binding in the block.
//! * The keystroke slot takes one string, a bracketed list of strings (several
//!   default keystrokes for one command), or the bare word `unbound` for a
//!   command that is reachable only from the palette.
//! * `in "<predicate>"` overrides the block's key context. Written after the
//!   keystroke slot it applies to the whole command; written after one keystroke
//!   *inside* the bracketed list it applies to that keystroke alone. The latter
//!   is what lets `down` stay live inside a text field while the vim-style `j`
//!   for the same command stands down (`!TextInput`).
//! * `if <predicate_fn>` names a `fn(CommandContext) -> bool` in scope;
//!   the default is `always_show`.
//! * `[hidden]` keeps the command out of the command palette.

/// Picks the more specific of two key contexts: the override if one was
/// written, otherwise the inherited default.
macro_rules! command_context {
    ($inherited:literal) => {
        $inherited
    };
    ($inherited:literal, $override:literal) => {
        $override
    };
}

/// Expands the keystroke slot of a `commands!` entry into the command's default
/// bindings: `&'static [(keystrokes, key context)]`.
///
/// The leading argument(s) are the inherited context — the `view` block default,
/// then the per-command `in "..."` when one was written — and each keystroke may
/// narrow it further with its own `in "..."`. The two arities are spelled out
/// separately because `macro_rules!` cannot expand a repetition of the inherited
/// context inside the repetition over keystrokes.
macro_rules! command_bindings {
    ($inherited:literal, unbound) => {
        &[] as &[(&'static str, &'static str)]
    };
    ($inherited:literal, [$($keystroke:literal $(in $context:literal)?),* $(,)?]) => {
        &[$(
            ($keystroke, command_context!($inherited $(, $context)?)),
        )*] as &[(&'static str, &'static str)]
    };
    ($inherited:literal, $keystroke:literal) => {
        &[($keystroke, $inherited)] as &[(&'static str, &'static str)]
    };
    ($view:literal, $command:literal, unbound) => {
        &[] as &[(&'static str, &'static str)]
    };
    (
        $view:literal, $command:literal,
        [$($keystroke:literal $(in $context:literal)?),* $(,)?]
    ) => {
        &[$(
            ($keystroke, command_context!($command $(, $context)?)),
        )*] as &[(&'static str, &'static str)]
    };
    ($view:literal, $command:literal, $keystroke:literal) => {
        &[($keystroke, $command)] as &[(&'static str, &'static str)]
    };
}

/// Resolves a command's palette availability predicate.
macro_rules! command_predicate {
    () => {
        $crate::command_palette::always_show
    };
    ($predicate:ident) => {
        $predicate
    };
}

/// Resolves whether a command is offered in the command palette.
///
/// Only the literal marker `[hidden]` is accepted, so a typo is a compile error
/// rather than a silently ignored flag.
macro_rules! command_in_palette {
    () => {
        true
    };
    ([hidden]) => {
        false
    };
}

/// Number of bytes a PascalCase identifier occupies once converted to snake_case.
pub(crate) const fn snake_case_len(name: &str) -> usize {
    let bytes = name.as_bytes();
    let mut len = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() && i > 0 {
            len += 1;
        }
        len += 1;
        i += 1;
    }
    len
}

/// Converts a PascalCase identifier to snake_case at compile time.
///
/// `N` must equal [`snake_case_len`] of `name`; [`command_id_str`] computes it.
pub(crate) const fn snake_case_bytes<const N: usize>(name: &str) -> [u8; N] {
    let bytes = name.as_bytes();
    let mut out = [0u8; N];
    let mut i = 0usize;
    let mut out_i = 0usize;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte.is_ascii_uppercase() {
            if i > 0 {
                out[out_i] = b'_';
                out_i += 1;
            }
            out[out_i] = byte.to_ascii_lowercase();
        } else {
            out[out_i] = byte;
        }
        out_i += 1;
        i += 1;
    }
    out
}

/// Compile-time snake_case rendering of a command variant name.
///
/// These strings are the stable on-disk identifiers used by the command palette
/// and by settings, so they must never drift from the historical hand-written
/// values — `command_id_strings_are_stable` in `registry.rs` pins all of them.
macro_rules! command_id_str {
    ($name:ident) => {{
        const SOURCE: &str = stringify!($name);
        const LEN: usize = $crate::keymap::macros::snake_case_len(SOURCE);
        const BYTES: &[u8; LEN] = &$crate::keymap::macros::snake_case_bytes::<LEN>(SOURCE);
        match ::core::str::from_utf8(BYTES) {
            Ok(id) => id,
            // Unreachable: the input is an ASCII Rust identifier.
            Err(_) => panic!("command id is not valid UTF-8"),
        }
    }};
}

/// Declares every rgitui command. See the [module docs](self) for the syntax.
macro_rules! commands {
    (
        $(
            view $view:ident in $namespace:ident context $view_context:literal {
                $(
                    $(#[doc = $doc:literal])*
                    $name:ident $keystrokes:tt
                        $(in $context:literal)?
                        $(if $predicate:ident)?
                        $([$hidden:ident])?
                    ;
                )*
            }
        )*
    ) => {
        /// Every user-invokable rgitui command.
        ///
        /// Generated by [`commands!`]. [`CommandId::as_str`] is the stable
        /// identifier used on disk; the gpui action name is
        /// [`CommandId::action_name`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CommandId {
            $($(
                $(#[doc = $doc])*
                $name,
            )*)*
        }

        /// The gpui [`Action`](gpui::Action) structs bound by the keymap.
        ///
        /// One unit struct per command, registered with gpui under
        /// `namespace::Name` so it can be named from `keymap.json`.
        pub mod actions {
            $($(
                $(#[doc = $doc])*
                #[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
                #[action(namespace = $namespace)]
                pub struct $name;
            )*)*
        }

        impl CommandId {
            /// Every command, in declaration order.
            pub const ALL: &'static [CommandId] = &[
                $($( CommandId::$name, )*)*
            ];

            /// The stable snake_case identifier for this command.
            ///
            /// Persisted in settings and matched by the command palette, so
            /// these strings are frozen; see `command_id_strings_are_stable`.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($( CommandId::$name => command_id_str!($name), )*)*
                }
            }

            /// The gpui action name used to refer to this command from `keymap.json`.
            pub const fn action_name(self) -> &'static str {
                match self {
                    $($(
                        CommandId::$name =>
                            concat!(stringify!($namespace), "::", stringify!($name)),
                    )*)*
                }
            }
        }

        /// Static metadata for every command, in declaration order.
        ///
        /// Drives default key binding, keymap validation, `docs/KEYBINDINGS.md`,
        /// the `keymap.json` schema and the shortcuts UI.
        pub const ALL_COMMANDS: &[CommandMeta] = &[
            $($(
                CommandMeta {
                    id: CommandId::$name,
                    view: stringify!($view),
                    namespace: stringify!($namespace),
                    action_name: concat!(stringify!($namespace), "::", stringify!($name)),
                    default_bindings: command_bindings!(
                        $view_context $(, $context)?, $keystrokes
                    ),
                    description: concat!("" $(, $doc)*),
                    in_palette: command_in_palette!($([$hidden])?),
                    availability: command_predicate!($($predicate)?),
                },
            )*)*
        ];

        /// Registers one `on_action` handler per command belonging to `view`.
        ///
        /// Every handler forwards its [`CommandId`] to `dispatch`, so the view
        /// keeps a single entry point for keyboard-invoked commands.
        pub fn attach_actions<E, V>(
            element: E,
            view: &str,
            cx: &mut gpui::Context<V>,
            dispatch: fn(&mut V, CommandId, &mut gpui::Window, &mut gpui::Context<V>),
        ) -> E
        where
            E: gpui::InteractiveElement,
            V: 'static,
        {
            let mut element = element;
            $(
                if view == stringify!($view) {
                    $(
                        element = element.on_action(cx.listener(
                            move |view, _: &actions::$name, window, cx| {
                                dispatch(view, CommandId::$name, window, cx);
                            },
                        ));
                    )*
                }
            )*
            element
        }
    };
}
