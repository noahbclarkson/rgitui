//! The settings UI as a standalone OS window.
//!
//! The user opens settings via the toolbar gear, Ctrl+,, or the command
//! palette. Each path calls `Workspace::open_or_focus_settings` (see
//! [`crate::Workspace`]), which either focuses the existing window
//! (deduplicated via [`gpui::WindowHandle`]) or creates a new one via
//! `cx.open_window`.
//!
//! Cross-window communication: settings actions that affect the workspace
//! (e.g. opening the theme editor, refreshing dependent panels after a
//! settings change) flow through [`SettingsWindowActionGlobal`], an
//! App-scoped `async-channel`. The settings window itself reads
//! [`rgitui_settings::SettingsState`] and [`rgitui_theme::ThemeState`]
//! directly via globals and reacts to changes via `cx.observe_global`.
//!
//! Window bounds are persisted on
//! [`rgitui_settings::AppSettings`] under `settings_window_bounds`. The
//! render loop updates the in-memory state on resize/move; the on-disk
//! write happens on window close to avoid per-frame keychain access.

mod channel;
mod events;
mod view;
mod window;
mod window_options;

pub use channel::{channel, SettingsWindowAction, SettingsWindowActionGlobal};
pub use events::SettingsViewEvent;
pub use view::SettingsView;
pub use window::SettingsWindow;
pub use window_options::settings_window_options;
