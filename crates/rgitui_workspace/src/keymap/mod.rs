//! User-definable keybindings.
//!
//! * [`macros`] defines `commands!`, the single source of truth.
//! * [`registry`] declares every command with it, producing [`CommandId`], the
//!   gpui action structs and [`ALL_COMMANDS`].
//! * [`conflict`] detects ambiguous bindings before gpui silently resolves them.
//! * [`loader`] reads `keymap.json` and assembles the final binding list.
//!
//! [`init`] wires it up: it applies the bindings and watches `keymap.json` so
//! saving the file reloads it. Reload outcomes land in [`KeymapState`], which the
//! workspace observes in order to toast problems.

#[macro_use]
pub(crate) mod macros;

pub mod conflict;
pub mod loader;
pub mod registry;

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, Global};

pub use loader::keymap_path;
pub use registry::{actions, attach_actions, CommandId, CommandMeta, ALL_COMMANDS};

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
}

impl Global for KeymapState {}

impl KeymapState {
    /// Takes the pending problems, leaving the state empty.
    pub fn take_problems(&mut self) -> Vec<String> {
        std::mem::take(&mut self.problems)
    }
}

/// Keeps the `keymap.json` watcher alive for the lifetime of the app.
struct KeymapWatcher {
    #[allow(dead_code, reason = "dropping the watcher stops the notifications")]
    watcher: Box<dyn std::any::Any>,
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
        watcher: Box::new(watcher),
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
