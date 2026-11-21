//! Application-wide commands that belong to no window, and the menu bar that
//! offers them.
//!
//! macOS gives an app no Quit item and no Cmd+Q of its own: both come from the
//! application menu, so without one the only way out is the Dock. Windows and
//! Linux have no global menu bar, and gpui only stores the menus there, so the
//! same setup is harmless on every platform.

use gpui::{App, BorrowAppContext, Menu, MenuItem};
use rgitui_i18n::t;

use crate::keymap::{actions, KeymapState};

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

/// Records a clean exit and quits.
///
/// Quitting is deliberate, so the next launch restores the saved workspace
/// without offering crash recovery.
pub fn quit(cx: &mut App) {
    if cx.has_global::<rgitui_settings::SettingsState>() {
        cx.update_global::<rgitui_settings::SettingsState, _>(|settings, _| {
            settings.mark_clean_exit();
        });
    }
    cx.quit();
}

fn set_menus(cx: &mut App) {
    // macOS titles the application menu with the app's name whatever this says.
    cx.set_menus([
        Menu::new("rgitui").items([MenuItem::action(t!("app_menu.quit"), actions::Quit)])
    ]);
}
