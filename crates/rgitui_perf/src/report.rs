//! The run's output, shaped so that reading it is the fast path.
//!
//! # Why the shape matters
//!
//! A run produces tens of thousands of samples. Handing that to a reader — a
//! person or an agent — and expecting a conclusion is how profiling data ends
//! up unused. So the raw series go to `samples.jsonl` and `frames.jsonl`, and
//! `report.json` carries only what a decision needs: a summary, a ranked list
//! of *findings* that name what is wrong, and the top rows of each detail
//! table. It is budgeted to stay small enough to read in one go.
//!
//! [`Report::findings`] is the part that makes the file useful rather than
//! merely available. Thresholds are evaluated here, in code, so the report says
//! "the foreground executor blocked for 41ms at refresh.rs:506" rather than
//! leaving a reader to notice that themselves.

use serde::{Deserialize, Serialize};

use crate::frame::FrameStats;
use crate::gpu::GpuSample;
use crate::heap::CensusNode;
use crate::process::ProcessSample;
use crate::tasks::LocationStats;

/// Schema version of `report.json`.
///
/// Bumped on any breaking change to the shape so that `compare` refuses to
/// compare reports it would misread, rather than producing a plausible and
/// wrong delta.
pub const SCHEMA_VERSION: u32 = 1;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Worth knowing, not worth acting on alone.
    Info,
    /// A real cost that a user could notice.
    Warning,
    /// A user would definitely notice this.
    Critical,
}

/// Something the run detected, stated as a problem rather than a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    /// Stable machine-readable identifier, e.g. `frame.p99_over_budget`.
    /// Comparisons key off this, so it must not change with the wording.
    pub code: String,
    /// One sentence naming what is wrong.
    pub message: String,
    /// The numbers behind the claim.
    pub evidence: serde_json::Value,
}

/// Metadata describing what produced a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunInfo {
    /// ISO-8601 start time.
    pub started_at: String,
    /// Scenario name, or `record` for a live session.
    pub scenario: String,
    /// Corpus the run measured against.
    pub corpus: Option<String>,
    /// Git revision of rgitui under test.
    pub revision: Option<String>,
    /// Cargo profile the binary was built with. A run built with `dev` is not
    /// comparable to one built with `perf`, so this is recorded, not assumed.
    pub profile: String,
    /// Whether the dhat allocator was active, which invalidates timings.
    pub heap_profiling: bool,
    pub os: String,
    pub cpu_count: usize,
}

/// The headline numbers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Frame cadence across the whole run.
    pub frames: FrameStats,
    /// Longest single task on the frame thread, in milliseconds.
    pub worst_foreground_task_ms: f64,
    /// Process memory at the end of the run.
    pub final_process: ProcessSample,
    /// Highest working set observed.
    pub peak_working_set_bytes: u64,
    /// Bytes the semantic census accounted for at its largest.
    pub peak_census_bytes: usize,
    /// Process private bytes minus census bytes at peak. A large gap is
    /// memory rgitui's own structures do not explain.
    pub unaccounted_bytes: i64,
    /// Mean CPU utilisation as a percentage of one core.
    pub cpu_percent_mean: Option<f64>,
    /// Last GPU reading of the run.
    pub gpu: GpuSample,
}

/// A named region of a run, opened with `mark` and closed with `end_mark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkReport {
    pub name: String,
    pub wall_ms: f64,
    pub frames: FrameStats,
    /// Working set change across the region. Growth that never comes back is
    /// what distinguishes a cache from a leak.
    pub working_set_delta_bytes: i64,
}

/// Latency of one command, measured from dispatch to the next presented frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionReport {
    /// The `namespace::Name` that was dispatched.
    pub action: String,
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

/// The complete run output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub run: RunInfo,
    pub summary: Summary,
    /// Ranked most severe first.
    pub findings: Vec<Finding>,
    pub marks: Vec<MarkReport>,
    pub actions: Vec<ActionReport>,
    /// Worst task locations, foreground first.
    pub hot_locations: Vec<LocationStats>,
    /// Largest census nodes at peak.
    pub heap: Vec<CensusNode>,
}

/// Rows kept in each detail table, so `report.json` stays readable in one pass.
pub const MAX_HOT_LOCATIONS: usize = 40;
/// Census nodes kept. See [`MAX_HOT_LOCATIONS`].
pub const MAX_HEAP_NODES: usize = 60;

/// Thresholds that turn measurements into findings.
///
/// Defaults are deliberately conservative: a report that cries wolf gets
/// ignored, and the point of the findings list is that it is worth reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Multiple of the refresh interval above which p99 is a finding.
    pub frame_p99_refresh_multiple: f64,
    /// Fraction of frames that may be dropped before it is a finding.
    pub dropped_frame_fraction: f64,
    /// Foreground task duration that counts as a stall, in milliseconds.
    /// Half a 120Hz frame: long enough to be real, short enough to catch
    /// things before they become visible hitches.
    pub foreground_task_ms: f64,
    /// Census node size that is worth questioning, in bytes.
    pub large_census_node_bytes: usize,
    /// Working set a mark may add without it being reported, in bytes.
    pub mark_growth_bytes: i64,
    /// Process memory the census may fail to explain, in bytes.
    pub unaccounted_bytes: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            frame_p99_refresh_multiple: 2.0,
            dropped_frame_fraction: 0.01,
            foreground_task_ms: 8.0,
            large_census_node_bytes: 100 * 1024 * 1024,
            mark_growth_bytes: 20 * 1024 * 1024,
            unaccounted_bytes: 200 * 1024 * 1024,
        }
    }
}

impl Report {
    /// Evaluates `thresholds` against the report's own data and fills in
    /// [`Report::findings`], most severe first.
    pub fn detect_findings(&mut self, thresholds: &Thresholds) {
        let mut findings = Vec::new();

        let frames = &self.summary.frames;
        if frames.frames > 0 {
            let budget = frames.refresh_interval_ms * thresholds.frame_p99_refresh_multiple;
            if frames.p99_ms > budget {
                findings.push(Finding {
                    severity: Severity::Critical,
                    code: "frame.p99_over_budget".into(),
                    message: format!(
                        "99th percentile frame interval was {:.1}ms against a {:.1}ms budget — \
                         visible stutter under this workload",
                        frames.p99_ms, budget
                    ),
                    evidence: serde_json::json!({
                        "p99_ms": frames.p99_ms,
                        "budget_ms": budget,
                        "refresh_interval_ms": frames.refresh_interval_ms,
                        "max_ms": frames.max_ms,
                    }),
                });
            }

            let dropped_fraction = frames.dropped_frames as f64 / frames.frames as f64;
            if dropped_fraction > thresholds.dropped_frame_fraction {
                findings.push(Finding {
                    severity: Severity::Warning,
                    code: "frame.dropped".into(),
                    message: format!(
                        "{:.2}% of frames were dropped ({} of {})",
                        dropped_fraction * 100.0,
                        frames.dropped_frames,
                        frames.frames
                    ),
                    evidence: serde_json::json!({
                        "dropped_frames": frames.dropped_frames,
                        "total_frames": frames.frames,
                        "fraction": dropped_fraction,
                    }),
                });
            }

            if frames.idle_redraws > 0 {
                findings.push(Finding {
                    severity: Severity::Info,
                    code: "frame.idle_redraws".into(),
                    message: format!(
                        "{} frames redrew with nothing dirty — wasted GPU work while idle",
                        frames.idle_redraws
                    ),
                    evidence: serde_json::json!({ "idle_redraws": frames.idle_redraws }),
                });
            }
        }

        for location in self
            .hot_locations
            .iter()
            .filter(|row| row.is_foreground && row.max_ms > thresholds.foreground_task_ms)
        {
            findings.push(Finding {
                severity: Severity::Critical,
                code: "task.foreground_stall".into(),
                message: format!(
                    "{}:{} blocked the frame thread for {:.1}ms (worst of {} run(s))",
                    location.file, location.line, location.max_ms, location.count
                ),
                evidence: serde_json::json!({
                    "file": location.file,
                    "line": location.line,
                    "max_ms": location.max_ms,
                    "total_ms": location.total_ms,
                    "count": location.count,
                }),
            });
        }

        for node in self
            .heap
            .iter()
            .filter(|node| node.bytes > thresholds.large_census_node_bytes)
        {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "heap.large_node".into(),
                message: format!(
                    "{} holds {:.1} MB{}",
                    node.path,
                    bytes_to_mb(node.bytes as i64),
                    node.count
                        .map(|count| format!(" across {count} entries"))
                        .unwrap_or_default()
                ),
                evidence: serde_json::json!({
                    "path": node.path,
                    "bytes": node.bytes,
                    "count": node.count,
                }),
            });
        }

        for mark in self
            .marks
            .iter()
            .filter(|mark| mark.working_set_delta_bytes > thresholds.mark_growth_bytes)
        {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "memory.mark_growth".into(),
                message: format!(
                    "{} grew the working set by {:.1} MB and did not give it back",
                    mark.name,
                    bytes_to_mb(mark.working_set_delta_bytes)
                ),
                evidence: serde_json::json!({
                    "mark": mark.name,
                    "delta_bytes": mark.working_set_delta_bytes,
                }),
            });
        }

        if self.summary.unaccounted_bytes > thresholds.unaccounted_bytes {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "memory.unaccounted".into(),
                message: format!(
                    "{:.1} MB of process memory is not explained by rgitui's own structures — \
                     allocator slack, a dependency, or a leak",
                    bytes_to_mb(self.summary.unaccounted_bytes)
                ),
                evidence: serde_json::json!({
                    "unaccounted_bytes": self.summary.unaccounted_bytes,
                    "census_bytes": self.summary.peak_census_bytes,
                    "private_bytes": self.summary.final_process.private_bytes,
                }),
            });
        }

        if self.run.heap_profiling {
            findings.push(Finding {
                severity: Severity::Info,
                code: "run.timings_invalid".into(),
                message: "the dhat allocator was active, so every timing in this report \
                          describes dhat rather than rgitui — use it for heap data only"
                    .into(),
                evidence: serde_json::json!({ "heap_profiling": true }),
            });
        }

        findings.sort_by_key(|finding| std::cmp::Reverse(finding.severity));
        self.findings = findings;
    }

    /// Trims detail tables to their documented caps.
    pub fn truncate_tables(&mut self) {
        self.hot_locations.truncate(MAX_HOT_LOCATIONS);
        self.heap.truncate(MAX_HEAP_NODES);
    }

    /// Serialises to pretty JSON.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a report, rejecting a schema this build cannot read correctly.
    pub fn from_json(source: &str) -> anyhow::Result<Self> {
        let report: Self = serde_json::from_str(source)?;
        anyhow::ensure!(
            report.schema_version == SCHEMA_VERSION,
            "report schema v{} cannot be read by this build (expected v{SCHEMA_VERSION})",
            report.schema_version
        );
        Ok(report)
    }

    /// Renders the human-readable summary.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# Performance report — {}\n\n", self.run.scenario));
        out.push_str(&format!(
            "- Started: {}\n- Profile: `{}`\n- Corpus: {}\n\n",
            self.run.started_at,
            self.run.profile,
            self.run.corpus.as_deref().unwrap_or("—")
        ));

        if self.findings.is_empty() {
            out.push_str("## Findings\n\nNothing crossed a threshold.\n\n");
        } else {
            out.push_str("## Findings\n\n");
            for finding in &self.findings {
                out.push_str(&format!(
                    "- **{:?}** (`{}`) — {}\n",
                    finding.severity, finding.code, finding.message
                ));
            }
            out.push('\n');
        }

        let frames = &self.summary.frames;
        out.push_str("## Frames\n\n");
        out.push_str(&format!(
            "| Frames | FPS | p50 | p95 | p99 | max | Dropped |\n\
             |---:|---:|---:|---:|---:|---:|---:|\n\
             | {} | {:.1} | {:.2}ms | {:.2}ms | {:.2}ms | {:.2}ms | {} |\n\n",
            frames.frames,
            frames.fps(),
            frames.p50_ms,
            frames.p95_ms,
            frames.p99_ms,
            frames.max_ms,
            frames.dropped_frames
        ));

        out.push_str("## Memory\n\n");
        out.push_str(&format!(
            "- Working set (final): {:.1} MB\n- Working set (peak): {:.1} MB\n\
             - Census (peak): {:.1} MB\n- Unaccounted: {:.1} MB\n\n",
            bytes_to_mb(self.summary.final_process.working_set_bytes as i64),
            bytes_to_mb(self.summary.peak_working_set_bytes as i64),
            bytes_to_mb(self.summary.peak_census_bytes as i64),
            bytes_to_mb(self.summary.unaccounted_bytes)
        ));

        if !self.heap.is_empty() {
            out.push_str("### Largest heap nodes\n\n| Path | MB | Count |\n|:--|---:|---:|\n");
            for node in self.heap.iter().take(15) {
                out.push_str(&format!(
                    "| `{}` | {:.1} | {} |\n",
                    node.path,
                    bytes_to_mb(node.bytes as i64),
                    node.count
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "—".into())
                ));
            }
            out.push('\n');
        }

        if !self.hot_locations.is_empty() {
            out.push_str(
                "## Hot task locations\n\n| Location | Thread | Count | Total | p95 | Max |\n\
                 |:--|:--|---:|---:|---:|---:|\n",
            );
            for row in self.hot_locations.iter().take(15) {
                out.push_str(&format!(
                    "| `{}:{}` | {} | {} | {:.1}ms | {:.1}ms | {:.1}ms |\n",
                    row.file,
                    row.line,
                    if row.is_foreground {
                        "**foreground**"
                    } else {
                        "background"
                    },
                    row.count,
                    row.total_ms,
                    row.p95_ms,
                    row.max_ms
                ));
            }
            out.push('\n');
        }

        out
    }
}

/// Bytes as megabytes, for display only.
pub fn bytes_to_mb(bytes: i64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_report() -> Report {
        Report {
            schema_version: SCHEMA_VERSION,
            run: RunInfo {
                started_at: "2026-08-13T00:00:00Z".into(),
                scenario: "test".into(),
                corpus: None,
                revision: None,
                profile: "perf".into(),
                heap_profiling: false,
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

    #[test]
    fn a_clean_run_produces_no_findings() {
        let mut report = empty_report();
        report.summary.frames = FrameStats {
            frames: 600,
            drawn_frames: 600,
            duration_ms: 5000.0,
            p50_ms: 8.3,
            p95_ms: 8.4,
            p99_ms: 8.6,
            max_ms: 9.0,
            dropped_frames: 0,
            idle_redraws: 0,
            refresh_interval_ms: 8.3,
        };
        report.detect_findings(&Thresholds::default());
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn a_p99_past_the_budget_is_critical() {
        let mut report = empty_report();
        report.summary.frames = FrameStats {
            frames: 100,
            duration_ms: 2000.0,
            p99_ms: 40.0,
            max_ms: 60.0,
            refresh_interval_ms: 8.3,
            ..FrameStats::default()
        };
        report.detect_findings(&Thresholds::default());

        let finding = report
            .findings
            .iter()
            .find(|f| f.code == "frame.p99_over_budget")
            .expect("expected a frame budget finding");
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn a_slow_foreground_task_is_reported_but_a_slow_background_one_is_not() {
        let mut report = empty_report();
        report.hot_locations = vec![
            LocationStats {
                file: "refresh.rs".into(),
                line: 506,
                thread: Some("main".into()),
                is_foreground: true,
                count: 3,
                total_ms: 90.0,
                p95_ms: 41.0,
                max_ms: 41.0,
            },
            LocationStats {
                file: "worker.rs".into(),
                line: 1,
                thread: Some("bg".into()),
                is_foreground: false,
                count: 1,
                total_ms: 900.0,
                p95_ms: 900.0,
                max_ms: 900.0,
            },
        ];
        report.detect_findings(&Thresholds::default());

        let stalls: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.code == "task.foreground_stall")
            .collect();
        assert_eq!(stalls.len(), 1, "background work must not be flagged");
        assert!(stalls[0].message.contains("refresh.rs:506"));
    }

    #[test]
    fn findings_are_ordered_most_severe_first() {
        let mut report = empty_report();
        report.summary.frames = FrameStats {
            frames: 100,
            duration_ms: 2000.0,
            p99_ms: 40.0,
            dropped_frames: 50,
            idle_redraws: 4,
            refresh_interval_ms: 8.3,
            ..FrameStats::default()
        };
        report.detect_findings(&Thresholds::default());

        let severities: Vec<_> = report.findings.iter().map(|f| f.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(severities, sorted);
        assert_eq!(severities.first(), Some(&Severity::Critical));
    }

    #[test]
    fn a_dhat_run_says_its_own_timings_are_meaningless() {
        let mut report = empty_report();
        report.run.heap_profiling = true;
        report.detect_findings(&Thresholds::default());

        assert!(report
            .findings
            .iter()
            .any(|f| f.code == "run.timings_invalid"));
    }

    #[test]
    fn report_round_trips_through_json() {
        let mut report = empty_report();
        report.detect_findings(&Thresholds::default());
        let parsed = Report::from_json(&report.to_json().unwrap()).unwrap();
        assert_eq!(parsed, report);
    }

    #[test]
    fn a_future_schema_is_refused_rather_than_misread() {
        let mut value: serde_json::Value =
            serde_json::from_str(&empty_report().to_json().unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);

        let error = Report::from_json(&value.to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot be read"), "error: {error}");
    }

    #[test]
    fn tables_are_truncated_to_their_caps() {
        let mut report = empty_report();
        report.heap = (0..MAX_HEAP_NODES + 25)
            .map(|i| CensusNode {
                path: format!("node{i}"),
                bytes: 1,
                count: None,
            })
            .collect();
        report.truncate_tables();
        assert_eq!(report.heap.len(), MAX_HEAP_NODES);
    }

    #[test]
    fn markdown_leads_with_the_findings() {
        let mut report = empty_report();
        report.summary.frames = FrameStats {
            frames: 100,
            duration_ms: 2000.0,
            p99_ms: 40.0,
            refresh_interval_ms: 8.3,
            ..FrameStats::default()
        };
        report.detect_findings(&Thresholds::default());

        let markdown = report.to_markdown();
        let findings_at = markdown.find("## Findings").unwrap();
        let frames_at = markdown.find("## Frames").unwrap();
        assert!(findings_at < frames_at);
        assert!(markdown.contains("frame.p99_over_budget"));
    }
}
