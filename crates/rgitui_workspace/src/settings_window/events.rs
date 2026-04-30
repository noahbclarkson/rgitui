/// Events emitted by the settings view inside the dedicated settings window.
///
/// These flow only within the settings window: cross-window communication to
/// the workspace happens via [`super::SettingsWindowAction`] over an
/// `async-channel`-backed global instead. `ThemeChanged` is kept locally so
/// the `SettingsWindow` can call `window.refresh()` on its own surface.
#[derive(Debug, Clone)]
pub enum SettingsViewEvent {
    ThemeChanged(String),
}
