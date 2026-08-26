//! Main-thread time attributed to the code that caused it.
//!
//! gpui already records, per thread, the start and end of every task its
//! dispatcher runs, tagged with the `#[track_caller]` source location of the
//! `spawn` that created it. This module drains that buffer and rolls it up.
//!
//! The interesting output is the *foreground* table. Anything slow on the
//! foreground executor is holding the frame clock, so a location with a high
//! `total_ms` there is a direct explanation for dropped frames — which is
//! exactly the attribution that raw frame timings cannot give you.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Rolled-up timings for one source location on one thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationStats {
    /// Source file that spawned the task.
    pub file: String,
    /// Line within [`LocationStats::file`].
    pub line: u32,
    /// Thread the task ran on, as gpui named it. `None` for unnamed threads.
    pub thread: Option<String>,
    /// Whether this ran on the thread that drives the frame clock.
    pub is_foreground: bool,
    /// Tasks observed from this location.
    pub count: u64,
    /// Summed duration.
    pub total_ms: f64,
    /// 95th percentile single-task duration.
    pub p95_ms: f64,
    /// Longest single task.
    pub max_ms: f64,
}

/// Whether a source location belongs to the harness rather than to the app.
///
/// The scenario driver runs as a foreground task like any other, so gpui's
/// profiler records it faithfully — and a replay step that waits on a frame
/// looks exactly like the app blocking the frame thread for as long as the wait
/// took. Reporting that would be worse than useless: it puts a critical finding
/// at the top of every report, blaming the app for the observer's own presence.
fn is_harness_location(file: &str) -> bool {
    // Matched on the path rather than the crate name because that is all gpui's
    // `#[track_caller]` location carries. Both separators appear depending on
    // the platform the build ran on.
    file.contains("rgitui_perf/src") || file.contains("rgitui_perf\\src")
}

/// Accumulates task timings across a run.
#[derive(Debug, Default)]
pub struct TaskRecorder {
    samples: HashMap<LocationKey, Vec<f64>>,
}

/// Identity of a rolled-up row: one source location on one thread.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocationKey {
    file: String,
    line: u32,
    thread: Option<String>,
    is_foreground: bool,
}

impl TaskRecorder {
    /// Starts an empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one completed task, unless the harness itself spawned it.
    pub fn record(
        &mut self,
        file: &str,
        line: u32,
        thread: Option<&str>,
        is_foreground: bool,
        duration_ms: f64,
    ) {
        if is_harness_location(file) {
            return;
        }

        let key = LocationKey {
            file: file.to_string(),
            line,
            thread: thread.map(str::to_string),
            is_foreground,
        };
        self.samples.entry(key).or_default().push(duration_ms);
    }

    /// Rolls up every location, worst total first.
    ///
    /// Foreground locations sort ahead of background ones at equal totals:
    /// a millisecond spent on the frame thread costs the user more than a
    /// millisecond spent off it, and the report should lead with it.
    pub fn stats(&self) -> Vec<LocationStats> {
        let mut rows: Vec<LocationStats> = self
            .samples
            .iter()
            .map(|(key, durations)| {
                let mut sorted = durations.clone();
                sorted.sort_by(|a, b| a.total_cmp(b));
                LocationStats {
                    file: key.file.clone(),
                    line: key.line,
                    thread: key.thread.clone(),
                    is_foreground: key.is_foreground,
                    count: durations.len() as u64,
                    total_ms: durations.iter().sum(),
                    p95_ms: crate::frame::percentile(&sorted, 0.95),
                    max_ms: sorted.last().copied().unwrap_or(0.0),
                }
            })
            .collect();

        rows.sort_by(|a, b| {
            b.is_foreground
                .cmp(&a.is_foreground)
                .then(b.total_ms.total_cmp(&a.total_ms))
        });
        rows
    }

    /// The `limit` worst locations.
    pub fn top(&self, limit: usize) -> Vec<LocationStats> {
        let mut rows = self.stats();
        rows.truncate(limit);
        rows
    }

    /// Longest single task seen on the foreground thread.
    ///
    /// This is the headline jank number: it is the worst stall the frame clock
    /// was made to wait through.
    pub fn worst_foreground_ms(&self) -> f64 {
        self.stats()
            .iter()
            .filter(|row| row.is_foreground)
            .map(|row| row.max_ms)
            .fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_roll_up_repeated_samples_from_one_location() {
        let mut recorder = TaskRecorder::new();
        recorder.record("a.rs", 10, Some("main"), true, 1.0);
        recorder.record("a.rs", 10, Some("main"), true, 3.0);

        let stats = recorder.stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[0].total_ms, 4.0);
        assert_eq!(stats[0].max_ms, 3.0);
    }

    #[test]
    fn separate_locations_and_threads_stay_separate() {
        let mut recorder = TaskRecorder::new();
        recorder.record("a.rs", 10, Some("main"), true, 1.0);
        recorder.record("a.rs", 11, Some("main"), true, 1.0);
        recorder.record("a.rs", 10, Some("worker"), false, 1.0);

        assert_eq!(recorder.stats().len(), 3);
    }

    #[test]
    fn foreground_locations_lead_the_table_over_slower_background_ones() {
        let mut recorder = TaskRecorder::new();
        recorder.record("bg.rs", 1, Some("worker"), false, 500.0);
        recorder.record("fg.rs", 1, Some("main"), true, 5.0);

        let stats = recorder.stats();
        assert_eq!(stats[0].file, "fg.rs", "foreground work must sort first");
        assert_eq!(stats[1].file, "bg.rs");
    }

    #[test]
    fn worst_foreground_ignores_background_work() {
        let mut recorder = TaskRecorder::new();
        recorder.record("bg.rs", 1, Some("worker"), false, 500.0);
        recorder.record("fg.rs", 1, Some("main"), true, 12.0);

        assert_eq!(recorder.worst_foreground_ms(), 12.0);
    }

    #[test]
    fn the_harness_does_not_report_its_own_driver_as_the_app_stalling() {
        let mut recorder = TaskRecorder::new();
        recorder.record(
            "crates\\rgitui_perf\\src\\replay.rs",
            268,
            Some("main"),
            true,
            28.9,
        );
        recorder.record(
            "crates/rgitui_perf/src/session.rs",
            10,
            Some("main"),
            true,
            5.0,
        );
        recorder.record(
            "crates/rgitui_git/src/project/refresh.rs",
            506,
            Some("main"),
            true,
            12.0,
        );

        let stats = recorder.stats();
        assert_eq!(
            stats.len(),
            1,
            "only the app's own work belongs here: {stats:?}"
        );
        assert!(stats[0].file.contains("refresh.rs"));
        assert_eq!(recorder.worst_foreground_ms(), 12.0);
    }

    #[test]
    fn worst_foreground_is_zero_when_nothing_ran_on_the_frame_thread() {
        let mut recorder = TaskRecorder::new();
        recorder.record("bg.rs", 1, Some("worker"), false, 500.0);
        assert_eq!(recorder.worst_foreground_ms(), 0.0);
    }
}
