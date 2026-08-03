//! User-definable keybindings.
//!
//! * [`macros`] defines `commands!`, the single source of truth.
//! * [`registry`] declares every command with it, producing [`CommandId`], the
//!   gpui action structs and [`ALL_COMMANDS`].
//! * [`conflict`] detects ambiguous bindings before gpui silently resolves them.
//! * [`shadow`] reports, informationally, where a panel binding wins a keystroke
//!   from a global one — legitimate scoping, but worth knowing about.
//! * [`loader`] reads `keymap.json` and assembles the final binding list.
//! * [`display`] renders a keystroke the way the user reads it.
//! * [`summary`] records what the load produced — the effective binding of every
//!   command, where it came from and what went wrong — and is what every
//!   shortcut shown anywhere in the UI is read from.
//! * [`generate`] renders the committed `docs/KEYBINDINGS.md` and
//!   `docs/keymap.schema.json`.
//!
//! [`init`] wires it up: it applies the bindings and watches `keymap.json` so
//! saving the file reloads it. Reload outcomes land in [`KeymapState`], which the
//! workspace observes in order to toast problems.

#[macro_use]
pub(crate) mod macros;

pub mod conflict;
pub mod display;
pub mod generate;
pub mod loader;
pub mod registry;
pub mod shadow;
pub mod summary;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Global};

pub use display::{humanize_sequence, KeystrokeStyle};
pub use loader::{ensure_keymap_file, keymap_path, keymap_stub};
pub use registry::{actions, attach_actions, CommandId, CommandMeta, ALL_COMMANDS};
pub use summary::{
    CommandBindings, CommandGroup, EffectiveBinding, KeymapNote, KeymapSummary, NoteSeverity,
};

/// Declares a view's key context and attaches the commands it handles.
///
/// `key_context` is the space-separated identifier list gpui matches binding
/// contexts against — a view's own name plus any group it joins, e.g.
/// `"BlameView List"`. `views` names the `commands!` blocks whose `on_action`
/// handlers to install, typically the shared `"Menu"` block plus the view's own.
///
/// Every handler funnels into `dispatch`, so a view keeps one entry point for
/// keyboard-invoked commands. gpui stops propagating an action at the first
/// handler it reaches walking outwards from the focused element, which is what
/// makes `Esc`, `Enter` and `j`/`k` mean the right thing per view without any
/// `if focused` checks.
pub fn bind_actions<E, V>(
    element: E,
    key_context: &'static str,
    views: &[&'static str],
    cx: &mut gpui::Context<V>,
    dispatch: fn(&mut V, CommandId, &mut gpui::Window, &mut gpui::Context<V>),
) -> E
where
    E: gpui::InteractiveElement,
    V: 'static,
{
    let mut element = element.key_context(key_context);
    for view in views {
        element = attach_actions(element, view, cx, dispatch);
    }
    element
}

/// Debounce applied after a `keymap.json` change before reloading, so an editor
/// writing the file in several steps triggers one reload.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);

/// The outcome of the most recent keymap load.
///
/// A gpui [`Global`], so the workspace can `observe_global` it and surface
/// problems as toasts — including for reloads that happen long after startup.
pub struct KeymapState {
    /// Problems from the most recent load, ready to show to the user.
    pub problems: Vec<String>,
    /// Number of bindings currently applied.
    pub binding_count: usize,
    /// Incremented on every load, so observers can tell reloads apart.
    pub generation: usize,
    /// The bindings in force, labelled for display. Read at render time by every
    /// surface that shows a shortcut; `Arc` because those reads happen while the
    /// rest of the app state is already borrowed.
    pub summary: Arc<KeymapSummary>,
}

impl Global for KeymapState {}

impl KeymapState {
    /// Takes the pending problems, leaving the state empty.
    pub fn take_problems(&mut self) -> Vec<String> {
        std::mem::take(&mut self.problems)
    }

    /// Whether there is anything left to report.
    ///
    /// The workspace observes this global and reports problems by *taking* them,
    /// but taking goes through `update_global`, which notifies global observers
    /// again. Checking this first — and doing nothing when it is false — is what
    /// stops that from being an endless cycle, so
    /// `take_problems()` must always leave it `false`.
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }
}

/// The bindings in force, for rendering a shortcut anywhere in the UI.
///
/// Falls back to the registry defaults when the keymap has not loaded yet, so a
/// caller never has to decide what to show in that case.
pub fn summary(cx: &App) -> Arc<KeymapSummary> {
    cx.try_global::<KeymapState>()
        .map(|state| state.summary.clone())
        .unwrap_or_else(summary::fallback)
}

/// The humanised keystrokes for a command, or `None` when it is unbound.
///
/// The one accessor every shortcut hint in the UI goes through — see
/// [`summary`] for why a literal would be a lie.
pub fn shortcut(id: CommandId, cx: &App) -> Option<String> {
    summary(cx).display(id)
}

/// A tooltip captioned `text` and annotated with the command's current keystroke.
///
/// The keystroke is looked up when the tooltip is built rather than when the
/// button is, so it follows a `keymap.json` reload without the button having to
/// re-render. An unbound command gets a plain tooltip instead of an empty chip.
pub fn command_tooltip(
    text: impl Into<gpui::SharedString>,
    id: CommandId,
) -> impl Fn(&mut gpui::Window, &mut App) -> gpui::AnyView {
    let text = text.into();
    move |window, cx| match shortcut(id, cx) {
        Some(keystrokes) => rgitui_ui::Tooltip::with_shortcut(text.clone(), keystrokes)(window, cx),
        None => rgitui_ui::Tooltip::text(text.clone())(window, cx),
    }
}

/// Keeps the `keymap.json` watcher alive for the lifetime of the app.
struct KeymapWatcher {
    /// Never read: the watcher stops delivering notifications when dropped, so
    /// this field exists purely to keep it alive as long as the app is.
    _watcher: Box<dyn std::any::Any>,
}

impl Global for KeymapWatcher {}

/// Applies the default bindings followed by the user's `keymap.json`.
///
/// Every binding is cleared first, so this doubles as the reload path. Defaults
/// go in before user bindings because gpui prefers the binding added last.
pub fn reload(cx: &mut App) {
    let loaded = loader::load(cx);
    let binding_count = loaded.bindings.len();

    cx.clear_key_bindings();
    cx.bind_keys(loaded.bindings);

    for problem in &loaded.problems {
        log::warn!("keymap: {problem}");
    }
    log::info!("keymap: applied {binding_count} key bindings");

    let generation = cx
        .try_global::<KeymapState>()
        .map_or(0, |state| state.generation + 1);
    cx.set_global(KeymapState {
        problems: loaded.problems,
        binding_count,
        generation,
        summary: Arc::new(loaded.summary),
    });
}

/// Loads the keymap and starts watching `keymap.json` for changes.
///
/// Call once during startup, after settings are initialised (the config
/// directory must exist for the watcher to attach).
pub fn init(cx: &mut App) {
    reload(cx);
    watch_keymap_file(cx);
}

/// Watches the config directory and reloads when `keymap.json` changes.
///
/// The directory rather than the file is watched, so a keymap created after
/// startup — or replaced by an editor's atomic save — is still noticed.
fn watch_keymap_file(cx: &mut App) {
    use notify::{RecursiveMode, Watcher as _};

    let path = keymap_path();
    let Some(directory) = path.parent().map(PathBuf::from) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&directory) {
        log::warn!(
            "keymap: could not create {}, so keymap.json will not be watched: {error}",
            directory.display()
        );
        return;
    }

    let (tx, rx) = async_channel::unbounded::<()>();
    let watched = path.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        if event.paths.iter().any(|changed| changed == &watched) {
            let _ = tx.try_send(());
        }
    });

    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("keymap: could not start the keymap.json watcher: {error}");
            return;
        }
    };
    if let Err(error) = watcher.watch(&directory, RecursiveMode::NonRecursive) {
        log::warn!(
            "keymap: could not watch {}: {error}. Restart rgitui to pick up keymap.json changes.",
            directory.display()
        );
        return;
    }

    cx.set_global(KeymapWatcher {
        _watcher: Box::new(watcher),
    });

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while rx.recv().await.is_ok() {
            // Coalesce the burst of events an editor emits for one save.
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            while rx.try_recv().is_ok() {}
            cx.update(reload);
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_problems(problems: &[&str]) -> KeymapState {
        KeymapState {
            problems: problems.iter().map(|p| (*p).to_string()).collect(),
            binding_count: 0,
            generation: 1,
            summary: Arc::new(KeymapSummary::default()),
        }
    }

    /// The workspace reports keymap problems from inside a global observer, and
    /// reporting them goes through `update_global`, which notifies that same
    /// observer again. Convergence rests entirely on taking the problems making
    /// `has_problems` false, so the next pass returns without touching the
    /// global. If this ever stops holding, the app spins at 100% CPU on startup
    /// and on every keymap.json save.
    #[test]
    fn taking_the_problems_leaves_nothing_to_report() {
        let mut state = state_with_problems(&["bad binding", "unknown action"]);
        assert!(state.has_problems());

        let taken = state.take_problems();

        assert_eq!(taken.len(), 2);
        assert!(
            !state.has_problems(),
            "take_problems must drain the state, or the observer cycle never ends"
        );
        assert!(state.take_problems().is_empty());
    }

    #[test]
    fn a_clean_load_has_nothing_to_report() {
        assert!(!state_with_problems(&[]).has_problems());
    }
}
