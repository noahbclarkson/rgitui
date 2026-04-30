use async_channel::{unbounded, Receiver, Sender};
use gpui::{App, Global};

use crate::ToastKind;

/// Actions the `SettingsWindow` asks the main workspace window to perform.
///
/// `SettingsView` lives in a separate OS window, so GPUI's window-scoped
/// `EventEmitter` cannot reach the workspace. These actions are sent via
/// [`SettingsWindowActionGlobal`] and consumed by a spawned loop on the
/// `Workspace` entity.
#[derive(Debug, Clone)]
pub enum SettingsWindowAction {
    /// Open the theme editor overlay in the workspace window.
    OpenThemeEditor,
    /// Theme just changed in settings; the workspace should repaint.
    ThemeChanged(String),
    /// Settings were saved; the workspace should re-configure dependent panels.
    SettingsChanged,
    /// Show a toast in the workspace window.
    Toast(ToastKind, String),
}

/// App-scoped sender for [`SettingsWindowAction`]s. The `Workspace` owns the
/// matching receiver inside a spawned consumer task, set up in `Workspace::new`.
pub struct SettingsWindowActionGlobal(pub Sender<SettingsWindowAction>);

impl Global for SettingsWindowActionGlobal {}

impl SettingsWindowActionGlobal {
    /// Best-effort send through the global. Silently drops the message if the
    /// global has not been installed yet or the receiver was closed — neither
    /// is recoverable from the caller's vantage point.
    pub fn try_send(cx: &App, action: SettingsWindowAction) {
        if let Some(global) = cx.try_global::<Self>() {
            let _ = global.0.try_send(action);
        }
    }
}

/// Build an unbounded channel suitable for installing as the global sender and
/// driving the workspace consumer loop.
pub fn channel() -> (Sender<SettingsWindowAction>, Receiver<SettingsWindowAction>) {
    unbounded()
}
