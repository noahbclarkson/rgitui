//! The app's side of the performance harness.
//!
//! Two things live here. The first is a shim: every function below exists in
//! both a real and a do-nothing form, chosen by the `perf` feature, so that
//! call sites in `layout.rs`, `macros.rs` and `main.rs` are plain calls with no
//! `#[cfg]` around them. The duplication is confined to this one file, which is
//! a better trade than scattering conditional compilation through the views.
//!
//! The second is the census roots. `rgitui_perf` sits below every crate that
//! holds memory and cannot name their types, so it takes a callback; this is
//! that callback. Walking starts here because the workspace is the only place
//! that can see every tab, and the per-tab labels it produces —
//! `tab[0]/graph/commits`, `tab[0]/caches/diff` — are what make a report
//! actionable rather than merely accurate.

#[cfg(feature = "perf")]
pub use enabled::*;

#[cfg(not(feature = "perf"))]
pub use disabled::*;

#[cfg(feature = "perf")]
mod enabled {
    use gpui::{App, Window};
    use rgitui_perf::{Census, PerfSession};

    use crate::workspace::Workspace;

    /// Starts a run if `RGITUI_PERF` asked for one.
    ///
    /// A misconfigured harness fails loudly rather than starting the app
    /// without instrumentation: a run that silently produced no report would
    /// look like a run that found nothing wrong.
    pub fn install(workspace: gpui::WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
        let Some(mode) = rgitui_perf::Mode::from_env().unwrap_or_else(|error| {
            log::error!("{error}");
            None
        }) else {
            return;
        };

        let config = match rgitui_perf::SessionConfig::from_mode(mode) {
            Ok(config) => config,
            Err(error) => {
                log::error!("cannot start perf session: {error}");
                return;
            }
        };

        let census_handle = workspace.clone();
        let idle_handle = workspace;

        let result = PerfSession::install(
            config,
            Box::new(move |census, cx| {
                if let Some(workspace) = census_handle.upgrade() {
                    workspace.read(cx).census(census, cx);
                }
            }),
            Box::new(move |cx| {
                idle_handle
                    .upgrade()
                    .is_none_or(|workspace| workspace.read(cx).is_operation_idle())
            }),
            window,
            cx,
        );

        if let Err(error) = result {
            log::error!("cannot start perf session: {error}");
        }
    }

    /// Records that the workspace rendered, so the harness can tell a vsync
    /// tick that drew from one that did not.
    pub fn note_render(cx: &mut App) {
        PerfSession::note_render(cx);
    }

    /// Records a dispatched command for latency measurement and for the
    /// recorded trace.
    pub fn note_dispatch(action: &str, tab: Option<usize>, cx: &mut App) {
        PerfSession::note_dispatch(action, tab, cx);
    }

    /// Writes the report. Called when the window closes, which is what ends a
    /// recorded session.
    pub fn finish(cx: &mut App) {
        match PerfSession::finish(cx) {
            Ok(Some(path)) => log::info!("perf report written to {}", path.display()),
            Ok(None) => {}
            Err(error) => log::error!("cannot write perf report: {error}"),
        }
    }

    /// Whether a run is in progress.
    pub fn is_active(cx: &App) -> bool {
        PerfSession::is_active(cx)
    }

    /// The census entry point, walking every open tab.
    pub fn census_workspace(workspace: &Workspace, census: &mut Census, cx: &App) {
        workspace.census(census, cx);
    }
}

#[cfg(not(feature = "perf"))]
mod disabled {
    use gpui::{App, Window};

    use crate::workspace::Workspace;

    /// No-op: this build was not compiled with the harness.
    pub fn install(_workspace: gpui::WeakEntity<Workspace>, _window: &mut Window, _cx: &mut App) {}

    /// No-op: see [`install`].
    #[inline]
    pub fn note_render(_cx: &mut App) {}

    /// No-op: see [`install`].
    #[inline]
    pub fn note_dispatch(_action: &str, _tab: Option<usize>, _cx: &mut App) {}

    /// No-op: see [`install`].
    #[inline]
    pub fn finish(_cx: &mut App) {}

    /// Always false: see [`install`].
    #[inline]
    pub fn is_active(_cx: &App) -> bool {
        false
    }
}
