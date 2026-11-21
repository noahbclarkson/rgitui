//! How rgitui quits, and the menu bar that offers it.
//!
//! macOS gives an app no Quit item and no Cmd+Q of its own: both come from the
//! application menu, so without one the only way out was the Dock. Windows and
//! Linux have no global menu bar, and gpui only stores the menus there, so the
//! same setup is harmless on every platform.
//!
//! Every deliberate exit goes through [`quit`]: the Quit command, the menu item
//! and closing the main window alike. That is what records the exit as clean,
//! so the next launch does not mistake it for a crash.

use gpui::{App, BorrowAppContext, Menu, MenuItem, WindowId};
use rgitui_i18n::t;

use crate::keymap::{actions, KeymapState};
use crate::SettingsWindow;

/// Installs the global command handlers and the menu bar.
///
/// Call after the keymap and translations are loaded: macOS reads a menu
/// item's keystroke from the keymap when the menu is built, and the labels are
/// translated then too. Both are rebuilt when either changes.
pub fn init(cx: &mut App) {
    // The workspace root handles `Quit` itself; this catches it everywhere
    // else — the settings window, and the menu bar while no window has focus.
    cx.on_action(|_: &actions::Quit, cx| quit(cx));

    set_menus(cx);
    cx.observe_global::<KeymapState>(set_menus).detach();
    cx.observe_global::<rgitui_i18n::ActiveLanguage>(set_menus)
        .detach();
}

/// Quits when `main_window` closes.
///
/// The settings window cannot do anything without the workspace, so it must
/// not keep the app alive. On macOS nothing else would end the process either:
/// gpui keeps an app running after its last window closes, with no window left
/// to reopen from the Dock.
pub fn quit_when_closed(main_window: WindowId, cx: &mut App) {
    cx.on_window_closed(move |cx, closed| {
        if closed == main_window {
            quit(cx);
        }
    })
    .detach();
}

/// Saves what the user has not yet saved, records a clean exit and quits.
///
/// Deferred, because it is reached from inside a window's own key dispatch or
/// close, while that window is leased and cannot be updated — and the settings
/// window is one it has to update.
pub fn quit(cx: &mut App) {
    cx.defer(|cx| {
        commit_settings_edits(cx);
        if cx.has_global::<rgitui_settings::SettingsState>() {
            // A blocking save, which waits for the settings writer to drain the
            // saves queued just above before the process goes away.
            cx.update_global::<rgitui_settings::SettingsState, _>(|settings, _| {
                settings.mark_clean_exit();
            });
        }
        cx.quit();
    });
}

/// Commits a settings field the user is still editing.
///
/// Closing the settings window does this on the way out; quitting closes it
/// without going through that path, and a pasted API key should not be lost
/// because the user quit rather than pressed Enter.
fn commit_settings_edits(cx: &mut App) {
    for window in cx.windows() {
        if let Some(settings_window) = window.downcast::<SettingsWindow>() {
            let _ = settings_window.update(cx, |settings_window, _, cx| {
                settings_window.commit_pending_edits(cx);
            });
        }
    }
}

fn set_menus(cx: &mut App) {
    // macOS titles the application menu with the app's name whatever this says.
    cx.set_menus([
        Menu::new("rgitui").items([MenuItem::action(t!("app_menu.quit"), actions::Quit)])
    ]);
}
