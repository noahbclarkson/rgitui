//! Performance and memory instrumentation for rgitui.
//!
//! The harness answers four questions that the app could not answer before:
//! how long frames actually take, where main-thread time goes, what the heap
//! is made of, and how much each of those moves when the code changes.
//!
//! # Shape
//!
//! Everything is gated behind the `enabled` feature. With the feature off the
//! public API is a set of inlined no-ops, so the app's call sites carry no
//! `#[cfg]` and the shipped binary carries none of this code.
//!
//! This crate sits at the bottom of the dependency graph next to
//! [`rgitui_settings`] and [`rgitui_theme`]: it must never depend on the crates
//! it measures. Types that own memory implement [`HeapSize`] next to their own
//! definitions instead, and the harness walks them through that trait.
//!
//! # Modules
//!
//! - [`heap`] — what the memory is made of, labelled by feature rather than by
//!   call stack.
//! - [`frame`] — presented-frame cadence, the number that decides whether
//!   scrolling feels smooth.
//! - [`tasks`] — main-thread time attributed to the `spawn` site that caused
//!   it, read out of gpui's own profiler.
//! - [`process`] / [`gpu`] — OS-level counters for the whole process.
//! - [`driver`] — the scenario model: steps, marks, recorded traces.
//! - [`replay`] — executing a scenario against the running app.
//! - [`session`] — the live run that drives all of the above.
//! - [`report`] — aggregation into a pre-digested `report.json`.
//! - [`compare`] — whether two runs differ by more than noise.
//! - [`corpus`] — deterministic repositories to measure against.

pub mod compare;
pub mod corpus;
pub mod cpu;
pub mod draw;
pub mod driver;
pub mod frame;
pub mod gpu;
pub mod heap;
mod heap_foreign;
pub mod heap_profile;
pub mod process;
pub mod replay;
pub mod report;
pub mod session;
pub mod tasks;

pub use heap::{Census, CensusNode, HeapSize};
pub use report::{Finding, Report, Severity};
pub use session::{CensusFn, PerfSession, SessionConfig};

/// Environment variable that turns the harness on and picks its mode.
///
/// The app takes its first CLI argument as a repository path
/// (`rgitui/src/main.rs`), so the harness is configured out of band rather than
/// competing for argument slots.
pub const MODE_ENV: &str = "RGITUI_PERF";

/// How the harness was asked to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Instrument a session the user drives by hand.
    Record,
    /// Drive the session from a scenario script.
    Replay(std::path::PathBuf),
}

impl Mode {
    /// Parses [`MODE_ENV`], returning `None` when the harness is switched off.
    ///
    /// Accepts `record` and `replay:<path>`. An unrecognised value is a
    /// configuration mistake rather than a request to stay quiet, so it is
    /// reported instead of ignored.
    pub fn from_env_value(value: &str) -> anyhow::Result<Option<Self>> {
        let value = value.trim();
        if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("off") {
            return Ok(None);
        }
        if value.eq_ignore_ascii_case("record") {
            return Ok(Some(Self::Record));
        }
        if let Some(path) = value.strip_prefix("replay:") {
            let path = path.trim();
            anyhow::ensure!(
                !path.is_empty(),
                "{MODE_ENV}=replay: needs a scenario path, e.g. replay:scenarios/graph-scroll.json"
            );
            return Ok(Some(Self::Replay(std::path::PathBuf::from(path))));
        }
        anyhow::bail!(
            "{MODE_ENV}={value:?} is not a recognised mode — use `record` or `replay:<path>`"
        )
    }

    /// Reads [`MODE_ENV`] from the environment.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        match std::env::var(MODE_ENV) {
            Ok(value) => Self::from_env_value(&value),
            Err(_) => Ok(None),
        }
    }

    /// Short label used in report filenames and the run metadata.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Replay(_) => "replay",
        }
    }
}

/// Environment variable that overrides where an instrumented run keeps its
/// settings and caches. Set it to compare runs that deliberately share state.
pub const STATE_ENV: &str = "RGITUI_PERF_STATE";

/// Directory an instrumented run should use for settings and caches, if any.
///
/// A measured run must not read or write the installed app's state. Sharing it
/// makes a run depend on what the previous one left behind — the history cache
/// is populated by whichever run got there first, and `clean_exit` is false for
/// every run after the first, so the app offers crash recovery that a genuine
/// first start would not. It also has the run append its corpus to the user's
/// recent repositories, which is somebody's real settings file.
///
/// Returns `None` when the harness is off, leaving normal launches alone.
pub fn state_root() -> Option<std::path::PathBuf> {
    if !is_enabled() || std::env::var_os(MODE_ENV).is_none() {
        return None;
    }
    if let Some(explicit) = std::env::var_os(STATE_ENV) {
        return Some(std::path::PathBuf::from(explicit));
    }
    Some(std::env::temp_dir().join("rgitui-perf-state"))
}

/// Written into a state root the first time the harness uses it, so a cold run
/// can tell a directory it owns from one it was pointed at by mistake.
const STATE_MARKER: &str = ".rgitui-perf-state";

/// Resolves the state directory and puts it in the state the scenario asked
/// for, returning the root to redirect to.
///
/// A cold run clears the `config` and `cache` subdirectories, and [`STATE_ENV`]
/// is caller-supplied — so `RGITUI_PERF_STATE=$HOME`, or a project directory,
/// would have this delete somebody's `config/` and `cache/` before it had
/// established any right to. Sparing the root itself is not protection: those
/// two names are as common as directory names get. A root is therefore usable
/// only if the harness created it, or if it is empty and can be adopted;
/// anything else is refused, and refused *before* anything is removed.
pub fn prepare_state_root() -> anyhow::Result<Option<std::path::PathBuf>> {
    let Some(root) = state_root() else {
        return Ok(None);
    };
    claim_state_root(&root)?;

    if starts_cold() {
        for directory in ["config", "cache"] {
            let path = root.join(directory);
            if let Err(error) = std::fs::remove_dir_all(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("could not clear {} for a cold run: {error}", path.display());
                }
            }
        }
    }
    Ok(Some(root))
}

/// Establishes that `root` is the harness's to clear, creating or adopting it.
///
/// Refusing is the whole point, so it fails the run rather than carrying on
/// against the user's own directories.
fn claim_state_root(root: &std::path::Path) -> anyhow::Result<()> {
    let marker = root.join(STATE_MARKER);
    if marker.is_file() {
        return Ok(());
    }

    match std::fs::read_dir(root) {
        Ok(mut entries) => {
            // A directory that exists and holds something the harness did not
            // put there. Whatever it is, it is not this tool's to empty. An
            // empty one is adopted.
            if entries.next().is_some() {
                anyhow::bail!(
                    "{} already has contents and no {STATE_MARKER} marker, so it is not a                      directory this harness created. A run would delete its config/ and cache/                      subdirectories. Point {STATE_ENV} at a new or empty directory.",
                    root.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(root).map_err(|error| {
                anyhow::anyhow!(
                    "cannot create the perf state root {}: {error}",
                    root.display()
                )
            })?;
        }
        Err(error) => anyhow::bail!("cannot inspect {}: {error}", root.display()),
    }

    std::fs::write(
        &marker,
        "This directory holds settings and caches for rgitui performance runs.
         A cold run empties its config/ and cache/ subdirectories.
",
    )
    .map_err(|error| anyhow::anyhow!("cannot mark {} as the harness's: {error}", root.display()))?;
    Ok(())
}

/// Whether this run should begin with no persistent caches.
///
/// A scenario that does not say gets a cold start, because that is the case
/// that is easy to measure by accident and hard to notice: a warm run reports
/// the second-launch cost of a repository under a name that reads like a first
/// launch.
fn starts_cold() -> bool {
    match Mode::from_env() {
        Ok(Some(Mode::Replay(path))) => driver::Scenario::load(&path)
            .map(|scenario| scenario.start_from == driver::StartState::Cold)
            .unwrap_or(true),
        // A hand-driven session is a continuing one; wiping its settings
        // between launches would make it useless for the thing it is for.
        Ok(Some(Mode::Record)) => false,
        _ => false,
    }
}

/// When the process started, for measurements that begin before the app does.
///
/// Anything timed from harness install understates startup by however long the
/// platform, the window and the font system took, which is exactly the part a
/// user waiting for the app to appear is counting. First call wins, so calling
/// it early in `main` sets the baseline for everything after.
pub fn process_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// Whether this build carries the instrumentation at all.
pub const fn is_enabled() -> bool {
    cfg!(feature = "enabled")
}

/// Whether this build replaced the global allocator with dhat's.
///
/// Frame timings collected under dhat describe dhat, not the app, so the
/// session refuses to combine the two.
pub const fn is_heap_profiling() -> bool {
    cfg!(feature = "dhat-heap")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_the_harness_has_not_seen_is_refused() {
        // The case that matters: `RGITUI_PERF_STATE=$HOME`, or a project
        // directory. A cold run would delete `config/` and `cache/` under it.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/settings.json"), "{}").unwrap();

        let error = claim_state_root(dir.path()).unwrap_err().to_string();

        assert!(error.contains(STATE_MARKER), "error: {error}");
        assert!(
            dir.path().join("config/settings.json").is_file(),
            "the refusal happens before anything is removed"
        );
    }

    #[test]
    fn an_empty_directory_is_adopted_and_marked() {
        let dir = tempfile::TempDir::new().unwrap();
        claim_state_root(dir.path()).unwrap();
        assert!(dir.path().join(STATE_MARKER).is_file());
    }

    #[test]
    fn a_root_that_does_not_exist_yet_is_created() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("nested/state");
        claim_state_root(&root).unwrap();
        assert!(root.join(STATE_MARKER).is_file());
    }

    #[test]
    fn a_root_the_harness_marked_is_accepted_again() {
        let dir = tempfile::TempDir::new().unwrap();
        claim_state_root(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("cache")).unwrap();

        // Contents now, but they are the harness's own from the last run.
        claim_state_root(dir.path()).unwrap();
    }

    #[test]
    fn mode_parses_record() {
        assert_eq!(Mode::from_env_value("record").unwrap(), Some(Mode::Record));
        assert_eq!(
            Mode::from_env_value(" RECORD ").unwrap(),
            Some(Mode::Record)
        );
    }

    #[test]
    fn mode_parses_replay_with_path() {
        let mode = Mode::from_env_value("replay:scenarios/graph-scroll.json")
            .unwrap()
            .unwrap();
        assert_eq!(
            mode,
            Mode::Replay(std::path::PathBuf::from("scenarios/graph-scroll.json"))
        );
    }

    #[test]
    fn mode_is_off_for_empty_and_disabled_values() {
        for value in ["", "  ", "0", "off", "OFF"] {
            assert_eq!(
                Mode::from_env_value(value).unwrap(),
                None,
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn mode_rejects_replay_without_a_path() {
        let error = Mode::from_env_value("replay:").unwrap_err().to_string();
        assert!(error.contains("needs a scenario path"), "error: {error}");
    }

    #[test]
    fn mode_rejects_unknown_values() {
        let error = Mode::from_env_value("profile").unwrap_err().to_string();
        assert!(error.contains("not a recognised mode"), "error: {error}");
    }
}
