//! Comparing two runs, which is how "did that optimisation work" gets answered.
//!
//! # Why there is a noise floor
//!
//! Two identical runs of the same scenario never produce identical numbers.
//! Thermal state, background processes and allocator luck move frame
//! percentiles by a few percent on any machine. A comparison that reported
//! every difference would therefore report a regression on every run, and a
//! tool that is wrong most of the time gets ignored — which costs more than
//! having no tool, because it also costs the time spent looking.
//!
//! So each metric carries a floor below which a change is called noise, and
//! each floor has two halves: a change must be large *in proportion* and large
//! *in absolute terms*. The first version had only the percentage, and two
//! identical runs promptly reported five regressions — a mark spanning one
//! keystroke that moved 8.5ms to 13.8ms, and two dropped frames out of 954.
//! Both percentages were real. Neither was worth anyone's afternoon.
//!
//! The floors also differ by what kind of number is being compared. Memory is
//! held tightest, because the same scenario over the same corpus allocates
//! nearly the same bytes every run. Maxima are held loosest, because an
//! extremum is a single sample and swings freely. Measure your own machine with
//! `rgitui-perf compare` on two unchanged runs: it should come back clean.

use serde::{Deserialize, Serialize};

use crate::report::Report;

/// Which way a change points, for the metric in question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Measurably worse than the baseline.
    Regression,
    /// Measurably better than the baseline.
    Improvement,
    /// Within the noise floor.
    Unchanged,
}

/// One metric's movement between two runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDelta {
    /// Dotted metric name, e.g. `frames.p99_ms`.
    pub metric: String,
    pub baseline: f64,
    pub candidate: f64,
    /// Change as a fraction of the baseline. `None` when the baseline was zero,
    /// where a percentage would be meaningless rather than infinite.
    pub relative: Option<f64>,
    pub direction: Direction,
}

impl MetricDelta {
    /// Absolute change, candidate minus baseline.
    pub fn absolute(&self) -> f64 {
        self.candidate - self.baseline
    }
}

/// The result of comparing two reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub baseline_scenario: String,
    pub candidate_scenario: String,
    /// Every metric compared, regressions first.
    pub metrics: Vec<MetricDelta>,
    /// Finding codes present in the candidate but not the baseline.
    pub new_findings: Vec<String>,
    /// Finding codes present in the baseline but not the candidate.
    pub resolved_findings: Vec<String>,
    /// Notes about why a comparison may not mean what it appears to.
    pub caveats: Vec<String>,
}

impl Comparison {
    /// Whether anything moved beyond its noise floor in the wrong direction.
    pub fn has_regressions(&self) -> bool {
        self.metrics
            .iter()
            .any(|metric| metric.direction == Direction::Regression)
            || !self.new_findings.is_empty()
    }
}

/// How far a metric must move, in *both* relative and absolute terms, before
/// the change is believed.
///
/// Requiring both is what the first calibration run taught. With a percentage
/// alone, two identical runs reported five regressions: a mark covering one
/// keystroke moved 8.5ms to 13.8ms and was called 62% worse, and two dropped
/// frames out of 954 were called 33% worse. Both are true percentages and
/// neither is a finding. An absolute floor beside the relative one is what
/// separates "moved a lot" from "moved a lot of nothing".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoiseFloor {
    /// Relative and absolute change required of a frame percentile, in
    /// milliseconds. Percentiles are stable across runs, so the bar is low:
    /// 2ms is a fifth of a frame at 100Hz and worth knowing about.
    pub frame_timing: (f64, f64),
    /// Relative and absolute change required of a wall-clock duration, in
    /// milliseconds. Higher than a percentile's because a mark covering a
    /// single action is only a few frames long to begin with.
    pub duration: (f64, f64),
    /// Relative and absolute change required of a maximum, in milliseconds.
    /// An extremum is one sample by definition and swings freely between runs,
    /// so it takes a large move to mean anything.
    pub extremum: (f64, f64),
    /// Relative and absolute change required of a count.
    pub count: (f64, f64),
    /// Relative and absolute change required of a memory figure, in bytes.
    /// Tighter in relative terms than any timing: the same scenario over the
    /// same corpus allocates nearly the same bytes every run.
    pub memory: (f64, f64),
}

impl Default for NoiseFloor {
    /// Calibrated by comparing two identical runs of `graph-scroll` and raising
    /// each floor until that comparison came back clean. Re-measure on your own
    /// machine — `rgitui-perf compare` on two unchanged runs should report no
    /// regressions, and if it does these numbers are too tight for your hardware.
    fn default() -> Self {
        Self {
            frame_timing: (0.15, 2.0),
            duration: (0.15, 10.0),
            extremum: (0.50, 10.0),
            count: (0.25, 10.0),
            memory: (0.08, 8.0 * 1024.0 * 1024.0),
        }
    }
}

/// What kind of number a metric is, which decides its noise floor and which
/// direction counts as better.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A frame-interval percentile.
    FrameTiming,
    /// A wall-clock duration.
    Duration,
    /// A maximum or worst-case.
    Extremum,
    /// A number of events.
    Count,
    /// A number of bytes.
    Memory,
}

impl Kind {
    /// The `(relative, absolute)` pair this kind must clear.
    fn floor(self, noise: &NoiseFloor) -> (f64, f64) {
        match self {
            Kind::FrameTiming => noise.frame_timing,
            Kind::Duration => noise.duration,
            Kind::Extremum => noise.extremum,
            Kind::Count => noise.count,
            Kind::Memory => noise.memory,
        }
    }
}

/// Compares two runs.
///
/// Every metric here is one where lower is better, so the direction logic is
/// uniform. Adding a higher-is-better metric would need that assumption
/// revisited rather than a new row.
pub fn compare(baseline: &Report, candidate: &Report, noise: &NoiseFloor) -> Comparison {
    let mut metrics = Vec::new();

    let pairs: [(&str, Kind, f64, f64); 14] = [
        (
            "frames.p50_ms",
            Kind::FrameTiming,
            baseline.summary.frames.p50_ms,
            candidate.summary.frames.p50_ms,
        ),
        (
            "frames.p95_ms",
            Kind::FrameTiming,
            baseline.summary.frames.p95_ms,
            candidate.summary.frames.p95_ms,
        ),
        (
            "frames.p99_ms",
            Kind::FrameTiming,
            baseline.summary.frames.p99_ms,
            candidate.summary.frames.p99_ms,
        ),
        (
            // What the frame cost to produce, as distinct from how often
            // frames arrived. Cadence is pinned to the display and hides a
            // change in draw cost completely until the budget runs out.
            "draw.p50_ms",
            Kind::FrameTiming,
            baseline.summary.draws.draw_p50_ms,
            candidate.summary.draws.draw_p50_ms,
        ),
        (
            "draw.p95_ms",
            Kind::FrameTiming,
            baseline.summary.draws.draw_p95_ms,
            candidate.summary.draws.draw_p95_ms,
        ),
        (
            "draw.max_ms",
            Kind::Extremum,
            baseline.summary.draws.draw_max_ms,
            candidate.summary.draws.draw_max_ms,
        ),
        (
            "latency.dirty_to_draw_p95_ms",
            Kind::FrameTiming,
            baseline.summary.draws.dirty_to_draw_p95_ms,
            candidate.summary.draws.dirty_to_draw_p95_ms,
        ),
        (
            "frames.max_ms",
            Kind::Extremum,
            baseline.summary.frames.max_ms,
            candidate.summary.frames.max_ms,
        ),
        (
            "frames.dropped",
            Kind::Count,
            baseline.summary.frames.dropped_frames as f64,
            candidate.summary.frames.dropped_frames as f64,
        ),
        (
            "draws.idle_redraws",
            Kind::Count,
            baseline.summary.draws.idle_redraws as f64,
            candidate.summary.draws.idle_redraws as f64,
        ),
        (
            "tasks.worst_foreground_ms",
            Kind::Extremum,
            baseline.summary.worst_foreground_task_ms,
            candidate.summary.worst_foreground_task_ms,
        ),
        (
            "memory.peak_working_set_bytes",
            Kind::Memory,
            baseline.summary.peak_working_set_bytes as f64,
            candidate.summary.peak_working_set_bytes as f64,
        ),
        (
            "memory.peak_census_bytes",
            Kind::Memory,
            baseline.summary.peak_census_bytes as f64,
            candidate.summary.peak_census_bytes as f64,
        ),
        (
            "memory.private_bytes",
            Kind::Memory,
            baseline.summary.final_process.private_bytes as f64,
            candidate.summary.final_process.private_bytes as f64,
        ),
    ];

    for (metric, kind, base, cand) in pairs {
        metrics.push(delta(metric, kind, base, cand, noise));
    }

    for mark in &candidate.marks {
        let Some(base) = baseline.marks.iter().find(|m| m.name == mark.name) else {
            continue;
        };
        metrics.push(delta(
            &format!("mark.{}.wall_ms", mark.name),
            Kind::Duration,
            base.wall_ms,
            mark.wall_ms,
            noise,
        ));
        metrics.push(delta(
            &format!("mark.{}.p95_ms", mark.name),
            Kind::Duration,
            base.frames.p95_ms,
            mark.frames.p95_ms,
            noise,
        ));
    }

    // Regressions first, then by how far each moved: the top of the list should
    // be the thing most worth looking at.
    metrics.sort_by(|a, b| {
        rank(a.direction).cmp(&rank(b.direction)).then(
            b.relative
                .unwrap_or(0.0)
                .abs()
                .total_cmp(&a.relative.unwrap_or(0.0).abs()),
        )
    });

    let baseline_codes: Vec<&str> = baseline.findings.iter().map(|f| f.code.as_str()).collect();
    let candidate_codes: Vec<&str> = candidate.findings.iter().map(|f| f.code.as_str()).collect();

    Comparison {
        baseline_scenario: baseline.run.scenario.clone(),
        candidate_scenario: candidate.run.scenario.clone(),
        new_findings: candidate_codes
            .iter()
            .filter(|code| !baseline_codes.contains(code))
            .map(|code| (*code).to_string())
            .collect(),
        resolved_findings: baseline_codes
            .iter()
            .filter(|code| !candidate_codes.contains(code))
            .map(|code| (*code).to_string())
            .collect(),
        caveats: caveats(baseline, candidate),
        metrics,
    }
}

/// Reasons a comparison might be misleading, stated up front.
///
/// A number that looks like a 40% regression because one run was a debug build
/// is worse than no number, so the mismatches that would cause it are named
/// rather than left for the reader to spot.
fn caveats(baseline: &Report, candidate: &Report) -> Vec<String> {
    let mut caveats = Vec::new();

    if baseline.run.scenario != candidate.run.scenario {
        caveats.push(format!(
            "different scenarios ({} vs {}) — these runs did different work",
            baseline.run.scenario, candidate.run.scenario
        ));
    }
    if baseline.run.profile != candidate.run.profile {
        caveats.push(format!(
            "different build profiles ({} vs {}) — the timings are not comparable",
            baseline.run.profile, candidate.run.profile
        ));
    }
    if baseline.run.corpus != candidate.run.corpus {
        caveats.push(format!(
            "different corpora ({:?} vs {:?}) — the inputs differ, not just the code",
            baseline.run.corpus, candidate.run.corpus
        ));
    }
    if baseline.run.heap_profiling != candidate.run.heap_profiling {
        caveats.push(
            "one run used the dhat allocator and the other did not — every timing differs \
             for that reason alone"
                .to_string(),
        );
    }
    if baseline.run.os != candidate.run.os {
        caveats.push(format!(
            "different operating systems ({} vs {})",
            baseline.run.os, candidate.run.os
        ));
    }

    caveats
}

/// Sort key placing regressions above unchanged above improvements.
fn rank(direction: Direction) -> u8 {
    match direction {
        Direction::Regression => 0,
        Direction::Unchanged => 1,
        Direction::Improvement => 2,
    }
}

/// Builds one metric row, applying the kind's noise floor.
fn delta(
    metric: &str,
    kind: Kind,
    baseline: f64,
    candidate: f64,
    noise: &NoiseFloor,
) -> MetricDelta {
    let relative = if baseline == 0.0 {
        None
    } else {
        Some((candidate - baseline) / baseline)
    };

    // A change must clear the relative *and* the absolute floor. Either alone
    // produces confident nonsense: a percentage flags trivial movement on small
    // numbers, and an absolute threshold flags proportionally irrelevant
    // movement on large ones.
    let (relative_floor, absolute_floor) = kind.floor(noise);
    let absolute = candidate - baseline;
    let significant = absolute.abs() >= absolute_floor;

    let direction = match relative {
        Some(change) if significant && change > relative_floor => Direction::Regression,
        Some(change) if significant && change < -relative_floor => Direction::Improvement,
        Some(_) => Direction::Unchanged,
        // A metric that was zero and is no longer still has to clear the
        // absolute floor, or one dropped frame in a run of a thousand would be
        // reported as a regression of infinite percent.
        None if absolute >= absolute_floor => Direction::Regression,
        None => Direction::Unchanged,
    };

    MetricDelta {
        metric: metric.to_string(),
        baseline,
        candidate,
        relative,
        direction,
    }
}

/// Median of a set of values, used to combine repeated runs of one scenario.
///
/// The median rather than the mean because a single run interrupted by
/// something else on the machine should not drag the result; that outlier is
/// noise, not signal.
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Some(sorted[middle])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Finding, RunInfo, Severity, Summary, SCHEMA_VERSION};

    fn report(scenario: &str) -> Report {
        Report {
            schema_version: SCHEMA_VERSION,
            run: RunInfo {
                started_at: "2026-08-13T00:00:00Z".into(),
                scenario: scenario.into(),
                corpus: Some("medium".into()),
                revision: None,
                profile: "release-like".into(),
                heap_profiling: false,
                cpu_sampling: false,
                os: "windows".into(),
                cpu_count: 8,
            },
            summary: Summary::default(),
            findings: Vec::new(),
            marks: Vec::new(),
            actions: Vec::new(),
            hot_locations: Vec::new(),
            heap: Vec::new(),
        }
    }

    /// A mark of the given wall time, with frame statistics left at zero so a
    /// test asserting on durations is not also asserting on frame percentiles.
    fn mark(name: &str, wall_ms: f64) -> crate::report::MarkReport {
        crate::report::MarkReport {
            name: name.to_string(),
            wall_ms,
            frames: crate::frame::FrameStats::default(),
            draws: crate::draw::DrawStats::default(),
            working_set_delta_bytes: 0,
        }
    }

    fn find<'a>(comparison: &'a Comparison, metric: &str) -> &'a MetricDelta {
        comparison
            .metrics
            .iter()
            .find(|m| m.metric == metric)
            .unwrap_or_else(|| panic!("no metric {metric}"))
    }

    #[test]
    fn an_identical_run_reports_nothing_changed() {
        let base = report("graph-scroll");
        let mut candidate = report("graph-scroll");
        candidate.summary.frames.p99_ms = 10.0;
        let mut base = base;
        base.summary.frames.p99_ms = 10.0;

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert!(!comparison.has_regressions());
        assert_eq!(
            find(&comparison, "frames.p99_ms").direction,
            Direction::Unchanged
        );
    }

    #[test]
    fn a_change_inside_the_noise_floor_is_not_called_a_regression() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.summary.frames.p99_ms = 10.0;
        candidate.summary.frames.p99_ms = 10.8; // 8%, below the 10% timing floor

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(
            find(&comparison, "frames.p99_ms").direction,
            Direction::Unchanged
        );
        assert!(!comparison.has_regressions());
    }

    #[test]
    fn a_change_past_the_noise_floor_is_a_regression() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.summary.frames.p99_ms = 10.0;
        candidate.summary.frames.p99_ms = 14.0;

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        let delta = find(&comparison, "frames.p99_ms");
        assert_eq!(delta.direction, Direction::Regression);
        assert!((delta.relative.unwrap() - 0.4).abs() < 1e-9);
        assert!(comparison.has_regressions());
    }

    #[test]
    fn getting_faster_is_reported_as_an_improvement() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.summary.frames.p99_ms = 20.0;
        candidate.summary.frames.p99_ms = 10.0;

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(
            find(&comparison, "frames.p99_ms").direction,
            Direction::Improvement
        );
        assert!(!comparison.has_regressions());
    }

    #[test]
    fn memory_is_held_to_a_tighter_proportional_floor_than_frame_timing() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.summary.peak_working_set_bytes = 100_000_000;
        candidate.summary.peak_working_set_bytes = 112_000_000; // 12%, 12 MB
        base.summary.frames.p99_ms = 10.0;
        candidate.summary.frames.p99_ms = 11.2; // also 12%, but only 1.2ms

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(
            find(&comparison, "memory.peak_working_set_bytes").direction,
            Direction::Regression,
            "12 MB more memory is real"
        );
        assert_eq!(
            find(&comparison, "frames.p99_ms").direction,
            Direction::Unchanged,
            "the same 12% is only 1.2ms of frame time, which is run-to-run noise"
        );
    }

    #[test]
    fn a_handful_of_newly_dropped_frames_is_noise_but_a_flood_is_not() {
        let mut base = report("s");
        base.summary.frames.dropped_frames = 0;

        // Two identical runs of `graph-scroll` differed by two dropped frames
        // out of 954. Against a zero baseline that is an infinite percentage
        // and still nothing worth reporting.
        let mut a_few = report("s");
        a_few.summary.frames.dropped_frames = 3;
        let delta = find(
            &compare(&base, &a_few, &NoiseFloor::default()),
            "frames.dropped",
        )
        .clone();
        assert_eq!(delta.direction, Direction::Unchanged);
        assert_eq!(delta.relative, None, "no percentage exists against zero");

        // A run that started dropping frames in earnest is the finding this
        // metric exists for, and the absolute floor still lets it through.
        let mut many = report("s");
        many.summary.frames.dropped_frames = 120;
        assert_eq!(
            find(
                &compare(&base, &many, &NoiseFloor::default()),
                "frames.dropped"
            )
            .direction,
            Direction::Regression
        );
    }

    #[test]
    fn a_large_proportional_move_on_a_tiny_duration_is_not_a_regression() {
        // The calibration case: a mark spanning one keystroke moved 8.5ms to
        // 13.8ms between two identical runs. 62% worse, and five milliseconds.
        let mut base = report("s");
        let mut candidate = report("s");
        base.marks.push(mark("graph-jump-to-end", 8.49));
        candidate.marks.push(mark("graph-jump-to-end", 13.78));

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(
            find(&comparison, "mark.graph-jump-to-end.wall_ms").direction,
            Direction::Unchanged
        );
        assert!(!comparison.has_regressions());
    }

    #[test]
    fn a_large_proportional_move_on_a_substantial_duration_still_regresses() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.marks.push(mark("graph-select-500", 7_940.0));
        candidate.marks.push(mark("graph-select-500", 11_000.0));

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(
            find(&comparison, "mark.graph-select-500.wall_ms").direction,
            Direction::Regression
        );
    }

    #[test]
    fn a_new_finding_counts_as_a_regression_on_its_own() {
        let base = report("s");
        let mut candidate = report("s");
        candidate.findings.push(Finding {
            severity: Severity::Critical,
            code: "task.foreground_stall".into(),
            message: "…".into(),
            evidence: serde_json::json!({}),
        });

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(comparison.new_findings, vec!["task.foreground_stall"]);
        assert!(comparison.has_regressions());
    }

    #[test]
    fn a_finding_that_disappeared_is_reported_as_resolved() {
        let mut base = report("s");
        let candidate = report("s");
        base.findings.push(Finding {
            severity: Severity::Warning,
            code: "heap.large_node".into(),
            message: "…".into(),
            evidence: serde_json::json!({}),
        });

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(comparison.resolved_findings, vec!["heap.large_node"]);
        assert!(!comparison.has_regressions());
    }

    #[test]
    fn mismatched_runs_are_flagged_before_their_numbers_are_believed() {
        let base = report("graph-scroll");
        let mut candidate = report("diff-browse");
        candidate.run.profile = "debug".into();
        candidate.run.heap_profiling = true;

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(comparison.caveats.len(), 3, "{:?}", comparison.caveats);
        assert!(comparison.caveats.iter().any(|c| c.contains("scenarios")));
        assert!(comparison.caveats.iter().any(|c| c.contains("profiles")));
        assert!(comparison.caveats.iter().any(|c| c.contains("dhat")));
    }

    #[test]
    fn regressions_sort_above_everything_else() {
        let mut base = report("s");
        let mut candidate = report("s");
        base.summary.frames.p99_ms = 10.0;
        candidate.summary.frames.p99_ms = 14.0;
        base.summary.frames.p50_ms = 20.0;
        candidate.summary.frames.p50_ms = 5.0;

        let comparison = compare(&base, &candidate, &NoiseFloor::default());
        assert_eq!(comparison.metrics[0].direction, Direction::Regression);
        assert_eq!(
            comparison.metrics.last().unwrap().direction,
            Direction::Improvement
        );
    }

    #[test]
    fn median_ignores_a_single_interrupted_run() {
        assert_eq!(median(&[10.0, 11.0, 200.0]), Some(11.0));
        assert_eq!(median(&[10.0, 12.0]), Some(11.0));
        assert_eq!(median(&[]), None);
    }
}
