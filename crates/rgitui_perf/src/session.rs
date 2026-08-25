//! The live run: what is collected, when, and what gets written at the end.
//!
//! # Not perturbing what it measures
//!
//! A harness that costs as much as the thing it profiles reports its own
//! overhead. Three decisions keep that from happening:
//!
//! - **Nothing is written to disk during a run.** Samples buffer in memory and
//!   are written once at the end. A 10-minute soak at 120Hz buffers well under
//!   a megabyte, so there is no reason to pay for I/O jitter mid-measurement.
//! - **Per-tick work is trivial** — a subtraction, a push, a counter compare.
//!   The OS counters are read twice a second, not per frame.
//! - **The heap census runs only at explicit points**: a scenario `snapshot`
//!   step, a mark boundary, and end of run. Walking every commit in a large
//!   repository is far too expensive to do on a timer, and doing so would
//!   manufacture the stalls the run is supposed to detect.
//!
//! # Inverted dependency for the census
//!
//! This crate sits below everything it measures and must never depend on
//! `rgitui_workspace`. The app therefore hands in a [`CensusFn`] at install
//! time that knows how to walk its own roots, and the session just calls it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{App, BorrowAppContext as _, Global, Window};
use serde::{Deserialize, Serialize};

use crate::driver::{Scenario, TraceEvent};
use crate::frame::FrameRecorder;
use crate::gpu::{GpuSample, GpuSampler};
use crate::heap::Census;
use crate::process::{ProcessSample, ProcessSampler};
use crate::report::{
    ActionReport, MarkReport, Report, RunInfo, Severity, Summary, Thresholds, SCHEMA_VERSION,
};
use crate::tasks::TaskRecorder;
use crate::Mode;

/// Walks the app's roots into a [`Census`].
///
/// Supplied by the app because this crate cannot name the types that hold the
/// memory.
pub type CensusFn = Box<dyn Fn(&mut Census, &App)>;

/// Reports whether the app has any git operation in flight.
///
/// Supplied by the app for the same reason as [`CensusFn`]: the operation state
/// lives in `rgitui_workspace`, which this crate sits below.
pub type IdleFn = Box<dyn Fn(&App) -> bool>;

/// Whether the app currently has no git operation in flight.
///
/// Returns `true` when no session is installed or the app never registered a
/// predicate, so a `wait_for` step degrades to "don't wait" rather than
/// deadlocking a run.
pub fn is_idle(cx: &App) -> bool {
    let Some(session) = cx.try_global::<PerfSession>() else {
        return true;
    };
    match &session.idle_fn {
        Some(idle) => idle(cx),
        None => true,
    }
}

/// Reports where the active tab's repository lives.
///
/// Supplied by the app for the same reason as [`IdleFn`]. A scenario that
/// dirties the working tree has to find it, and guessing from `argv` would
/// break the moment a run opened more than one repository.
pub type RepoPathFn = Box<dyn Fn(&App) -> Option<PathBuf>>;

/// Path of the repository the active tab has open, if a session is installed
/// and the app registered a way to ask.
pub fn active_repo_path(cx: &App) -> Option<PathBuf> {
    cx.try_global::<PerfSession>()?
        .repo_path_fn
        .as_ref()
        .and_then(|repo_path| repo_path(cx))
}

/// How often the OS counters are read. Fast enough to catch a memory step
/// change within a scenario step, cheap enough to be invisible.
const COUNTER_INTERVAL: Duration = Duration::from_millis(500);

/// Environment variable overriding where reports are written.
pub const OUTPUT_DIR_ENV: &str = "RGITUI_PERF_OUT";

/// One reading of everything sampled on the counter timer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSample {
    /// Milliseconds since the run started.
    pub t_ms: f64,
    pub process: ProcessSample,
    pub gpu: GpuSample,
    /// Census total, present only on ticks where a census was taken.
    pub census_bytes: Option<usize>,
}

/// What a run was asked to do.
pub struct SessionConfig {
    pub mode: Mode,
    /// Scenario to replay. `None` in record mode.
    pub scenario: Option<Scenario>,
    /// Where the report is written.
    pub output_dir: PathBuf,
    /// Corpus the run is measuring against, taken from the scenario, for the
    /// report metadata.
    pub corpus: Option<String>,
    pub thresholds: Thresholds,
}

impl SessionConfig {
    /// Builds a config from the parsed mode, loading the scenario if there is one.
    pub fn from_mode(mode: Mode) -> anyhow::Result<Self> {
        let scenario = match &mode {
            Mode::Replay(path) => {
                let scenario = Scenario::load(path)?;
                scenario.validate_marks()?;
                Some(scenario)
            }
            Mode::Record => None,
        };

        let label = scenario
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| mode.label().to_string());
        // The scenario declares which corpus it is written against, and the
        // report is the only place a reader — or `compare`, which warns when
        // two runs used different inputs — can learn it afterwards.
        let corpus = scenario
            .as_ref()
            .and_then(|s| s.corpus)
            .map(|tier| tier.name().to_string());
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        // Not `target/perf`: that is where Cargo puts the `perf` profile's own
        // build artifacts, and a run would drop report directories in among
        // `deps/` and `incremental/` for `cargo clean` to take away later.
        let root = match std::env::var(OUTPUT_DIR_ENV) {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => PathBuf::from("target").join("perf-runs"),
        };

        Ok(Self {
            mode,
            scenario,
            output_dir: root.join(format!("{stamp}-{label}")),
            corpus,
            thresholds: Thresholds::default(),
        })
    }
}

/// A measurement region that has been opened but not yet closed.
struct OpenMark {
    name: String,
    started: Instant,
    start_frame: usize,
    start_working_set: u64,
}

/// A closed region, held as extents rather than as statistics.
///
/// Aggregating at `end_mark` would freeze whatever refresh interval the
/// recorders happened to hold at that moment — the 60Hz placeholder, for every
/// mark that closes before the run ends. Keeping the extents and aggregating
/// once, at the end, means every mark and the summary are judged against the
/// same budget.
struct CompletedMark {
    name: String,
    wall_ms: f64,
    frames: std::ops::Range<usize>,
    drawn_from: Instant,
    drawn_to: Instant,
    working_set_delta_bytes: i64,
}

/// The run in progress. Installed as a gpui [`Global`] so that the app's
/// dispatch hook and render path can reach it without threading a handle
/// through every view.
pub struct PerfSession {
    config: SessionConfig,
    started: Instant,
    census_fn: CensusFn,
    idle_fn: Option<IdleFn>,
    repo_path_fn: Option<RepoPathFn>,

    frames: FrameRecorder,
    draws: crate::draw::DrawRecorder,
    frame_timings: gpui::profiler::FrameTimingCollector,
    tasks: TaskRecorder,
    collector: gpui::ProfilingCollector,
    process: Option<ProcessSampler>,
    gpu: GpuSampler,

    samples: Vec<TimeSample>,
    trace: Vec<TraceEvent>,
    action_latencies: HashMap<String, Vec<f64>>,
    /// Dispatches waiting for the frame that will show their effect.
    pending_actions: Vec<(String, Instant)>,

    open_marks: Vec<OpenMark>,
    completed_marks: Vec<CompletedMark>,

    /// Bumped by the app's root view every time it renders, which is how the
    /// session knows a tick actually drew rather than merely ticked.
    render_count: u64,
    last_render_count: u64,
    /// Whether anything happened since the previous tick that could legitimately
    /// dirty the window. Drives the idle-redraw heuristic.
    activity_since_tick: bool,

    last_tick: Option<Instant>,
    last_process: ProcessSample,
    peak_working_set: u64,
    settle_timeouts: u64,
    /// Held for the run's lifetime: sampling starts when this is constructed
    /// and the profile is written when it drops.
    cpu: Option<crate::cpu::CpuProfiler>,
    peak_private_bytes: u64,
    peak_census_bytes: usize,
    /// Node breakdown from the census that produced [`Self::peak_census_bytes`].
    peak_census_nodes: Vec<crate::heap::CensusNode>,
    /// Time from process start until the app first drew real content.
    first_content: Option<Duration>,
    finished: bool,
}

impl Global for PerfSession {}

impl PerfSession {
    /// Starts a run: installs the global, arms the frame chain, starts the
    /// counter timer, and begins replay if a scenario was given.
    pub fn install(
        config: SessionConfig,
        census_fn: CensusFn,
        idle_fn: IdleFn,
        repo_path_fn: RepoPathFn,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        if crate::is_heap_profiling() && config.scenario.is_some() {
            log::warn!(
                "the dhat allocator is active, so this run's timings describe dhat rather than \
                 rgitui — treat the heap data as the only valid output"
            );
        }

        if let Some(scenario) = &config.scenario {
            verify_actions_exist(scenario, cx)?;
        }

        // gpui's profiler is off behind a runtime flag as well as a cargo
        // feature, and recording nothing is its whole point when idle: with the
        // flag clear it retains no timings and grows no buffer. Turning it on
        // is therefore part of starting a run, not part of building the binary.
        gpui::profiler::set_trace_enabled(true);
        // Separate flag, separate ring buffer: task timings say where dispatched
        // work went, frame timings say what each draw cost. Only the second one
        // can see `Render::render`, layout and paint, which are not tasks and so
        // never appear in the task table however long they take.
        gpui::profiler::set_frame_trace_enabled(true);

        let process = match ProcessSampler::new() {
            Ok(sampler) => Some(sampler),
            Err(error) => {
                log::error!("process counters unavailable, memory data will be missing: {error}");
                None
            }
        };

        let started = Instant::now();
        let scenario = config.scenario.clone();
        let session = Self {
            config,
            started,
            census_fn,
            idle_fn: Some(idle_fn),
            repo_path_fn: Some(repo_path_fn),
            // Recalibrated from observed ticks once the run has data; 60Hz is
            // only the placeholder until then.
            frames: FrameRecorder::new(16.667),
            draws: crate::draw::DrawRecorder::new(16.667),
            frame_timings: gpui::profiler::FrameTimingCollector::new(),
            tasks: TaskRecorder::new(),
            collector: gpui::ProfilingCollector::new(started),
            process,
            gpu: GpuSampler::new(),
            samples: Vec::new(),
            trace: Vec::new(),
            action_latencies: HashMap::new(),
            pending_actions: Vec::new(),
            open_marks: Vec::new(),
            completed_marks: Vec::new(),
            render_count: 0,
            last_render_count: 0,
            activity_since_tick: true,
            last_tick: None,
            last_process: ProcessSample::default(),
            peak_working_set: 0,
            settle_timeouts: 0,
            cpu: crate::cpu::CpuProfiler::start(),
            peak_private_bytes: 0,
            peak_census_bytes: 0,
            peak_census_nodes: Vec::new(),
            first_content: None,
            finished: false,
        };

        cx.set_global(session);
        arm_frame_chain(window);
        start_counter_timer(cx);

        match scenario {
            Some(scenario) => crate::replay::spawn(scenario, window, cx),
            // In record mode the user decides when the run ends, so the report
            // is written on window close rather than after a last step.
            None => log::info!("recording — the report is written when the window closes"),
        }

        log::info!("perf session started");
        Ok(())
    }

    /// Whether a run is in progress.
    pub fn is_active(cx: &App) -> bool {
        cx.has_global::<Self>()
    }

    /// Frames ticked so far, which replay uses to wait for pixels.
    /// Whether the app has drawn its workspace at least once.
    ///
    /// The harness installs as soon as the workspace *entity* exists, which is
    /// while the splash is still on screen — the entity is built behind it and
    /// handed over when the repository is ready. A scenario that starts
    /// dispatching before that handover is talking to a window that has no
    /// workspace in it, and every step lands on nothing.
    pub fn has_rendered(cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|session| session.render_count > 0)
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Draws recorded so far, after taking whatever gpui has finished since the
    /// last look.
    ///
    /// Replay waits on this rather than on [`Self::frame_count`]. The frame
    /// chain records a tick on every vsync whether or not the window was
    /// dirty, so waiting for a tick returns immediately on an idle app — which
    /// is exactly the case a `settle: frame` step exists to catch. Draining
    /// here rather than waiting for the counter timer keeps the answer within
    /// a poll interval instead of within 500ms.
    pub fn drawn_count(cx: &mut App) -> usize {
        if !cx.has_global::<Self>() {
            return 0;
        }
        cx.update_global::<Self, _>(|session, _| {
            session.drain_frame_timings();
            session.draws.len()
        })
    }

    /// Records that the app's root view rendered.
    ///
    /// Called from `Render::render`, so it must stay to a single increment.
    ///
    /// This counts frames on which the *workspace root* re-rendered. gpui
    /// caches views that are not dirty, so a frame that repainted only a child
    /// panel does not reach here and is recorded as not drawn. The count is
    /// therefore a lower bound, which is the safe direction to be wrong in:
    /// [`crate::frame::FrameStats::idle_redraws`] under-reports rather than
    /// inventing repaint loops that were never there.
    pub fn note_render(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global::<Self, _>(|session, _| {
            session.render_count += 1;
        });
    }

    /// Records that the app has put real content on screen for the first time.
    ///
    /// This is the startup number that matters and the one nothing measured.
    /// `startup-to-idle` waits on an idle predicate that goes true when no git
    /// operation is *running*, which on a 20,000-commit repository went true
    /// 2.8 seconds before any commit was drawn. Measured from process start
    /// rather than from harness install, because the seconds before the window
    /// exists are still seconds the user spent waiting.
    ///
    /// Only the first call counts; the app calls this from a render path.
    pub fn note_first_content(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global::<Self, _>(|session, _| {
            if session.first_content.is_none() {
                session.first_content = Some(crate::process_start().elapsed());
            }
        });
    }

    /// Records a dispatched command.
    ///
    /// The latency is closed out on the next tick that draws, so what lands in
    /// the report is keypress-to-pixels rather than keypress-to-return.
    pub fn note_dispatch(action: &str, tab: Option<usize>, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global::<Self, _>(|session, _| {
            let now = Instant::now();
            session.activity_since_tick = true;
            session.pending_actions.push((action.to_string(), now));
            session.trace.push(TraceEvent {
                t_ns: now.duration_since(session.started).as_nanos(),
                action: action.to_string(),
                tab,
            });
        });
    }

    /// Records a step that gave up waiting for a frame.
    ///
    /// Worth counting rather than shrugging at. A step that settles on a frame
    /// and never gets one costs the full timeout, so a scenario which has
    /// stopped causing repaints slows to a crawl and still reports success —
    /// with every wall time in it inflated by however many times this fired.
    pub fn note_settle_timeout(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global::<Self, _>(|session, _| session.settle_timeouts += 1);
    }

    /// Opens a measurement region.
    pub fn begin_mark(&mut self, name: &str) {
        self.open_marks.push(OpenMark {
            name: name.to_string(),
            started: Instant::now(),
            start_frame: self.frames.len(),
            start_working_set: self.last_process.working_set_bytes,
        });
    }

    /// Closes the most recent region with this name.
    pub fn end_mark(&mut self, name: &str) {
        let Some(index) = self.open_marks.iter().rposition(|mark| mark.name == name) else {
            log::warn!("end_mark {name:?} with no matching mark");
            return;
        };
        let mark = self.open_marks.remove(index);
        let ended = Instant::now();
        self.completed_marks.push(CompletedMark {
            name: mark.name,
            wall_ms: ended.duration_since(mark.started).as_secs_f64() * 1000.0,
            frames: mark.start_frame..self.frames.len(),
            drawn_from: mark.started,
            drawn_to: ended,
            working_set_delta_bytes: self.last_process.working_set_bytes as i64
                - mark.start_working_set as i64,
        });
    }

    /// Walks the app's roots and records the total.
    ///
    /// Expensive on a large repository, which is why it is only called at
    /// explicit points rather than on the counter timer.
    pub fn take_census(&mut self, cx: &App) -> usize {
        // Counting threads means walking every thread on the system — tens of
        // milliseconds — so it rides along with the census rather than with the
        // twice-a-second counters, where the stall would land on an arbitrary
        // frame and be reported as the app's fault.
        if let Some(sampler) = self.process.as_mut() {
            self.last_process.thread_count = sampler.sample_thread_count();
        }

        let mut census = Census::new();
        (self.census_fn)(&mut census, cx);
        let total = census.total_bytes();

        // The largest breakdown seen is the one worth reporting: a census taken
        // after the app happens to have evicted its caches would understate
        // what the run actually cost.
        if total >= self.peak_census_bytes {
            self.peak_census_bytes = total;
            self.peak_census_nodes = census.into_nodes();
        }
        if let Some(sample) = self.samples.last_mut() {
            sample.census_bytes = Some(total);
        }
        total
    }

    /// Reads the OS counters and drains gpui's task timings.
    fn sample_counters(&mut self) {
        let process = match self.process.as_mut().map(|sampler| sampler.sample()) {
            Some(Ok(sample)) => sample,
            Some(Err(error)) => {
                log::warn!("process counter read failed: {error}");
                self.last_process
            }
            None => self.last_process,
        };
        self.last_process = process;
        self.peak_working_set = self.peak_working_set.max(process.working_set_bytes);
        self.peak_private_bytes = self.peak_private_bytes.max(process.private_bytes);

        let gpu = self.gpu.sample();
        self.samples.push(TimeSample {
            t_ms: self.started.elapsed().as_secs_f64() * 1000.0,
            process,
            gpu,
            census_bytes: None,
        });

        self.drain_task_timings();
        self.drain_frame_timings();
    }

    /// Moves gpui's per-frame draw records into the recorder.
    ///
    /// Drained on the counter timer for the same reason as the task timings:
    /// gpui keeps a bounded ring of them and drops the oldest once it is full,
    /// and the frames worth having are usually the early ones.
    fn drain_frame_timings(&mut self) {
        for timing in self.frame_timings.collect_unseen() {
            self.charge_actions_answered_by(&timing);
            self.draws.record(crate::draw::DrawSample {
                drawn_at: timing.draw_end,
                draw_ms: timing.draw_duration().as_secs_f64() * 1000.0,
                dirty_to_draw_ms: timing
                    .dirty_to_draw_duration()
                    .map(|duration| duration.as_secs_f64() * 1000.0),
                invalidations: timing.invalidations,
            });
        }
    }

    /// Moves gpui's per-thread task timings into the recorder.
    ///
    /// gpui caps what it retains per thread and drops the oldest entries past
    /// that, so draining on a timer rather than at the end is what keeps a long
    /// run from silently discarding its earliest — and often most interesting —
    /// stalls.
    fn drain_task_timings(&mut self) {
        // Completed tasks only. A task still running has no duration yet, and
        // including it would report the time it has been alive so far as though
        // that were its cost.
        let all = gpui::profiler::get_all_timings(gpui::TasksIncluded::OnlyCompleted);

        // Matched on thread *id*, not name. Background threads are frequently
        // unnamed, and so is the main thread on some platforms — comparing
        // names then makes every unnamed worker look like the frame thread, and
        // the foreground/background split is the one distinction in this report
        // that has to be right.
        let foreground_id = hashed_thread_id(std::thread::current().id());

        for delta in self.collector.collect_unseen(all) {
            let is_foreground = delta.thread_id == foreground_id;
            for timing in delta.new_timings {
                self.tasks.record(
                    &timing.location.file,
                    timing.location.line,
                    delta.thread_name.as_deref(),
                    is_foreground,
                    timing.duration as f64 / 1_000_000.0,
                );
            }
        }
    }

    /// One frame tick.
    ///
    /// Cadence only. Action latency is closed out against gpui's own frame
    /// records in [`Self::drain_frame_timings`], because a tick says a frame
    /// was presented and says nothing about whether it contained the effect of
    /// any particular dispatch.
    fn tick(&mut self, now: Instant) {
        let drawn = self.render_count != self.last_render_count;
        self.last_render_count = self.render_count;

        if let Some(previous) = self.last_tick {
            let interval_ms = now.duration_since(previous).as_secs_f64() * 1000.0;
            self.frames.record(interval_ms, drawn);
        }
        self.last_tick = Some(now);
    }

    /// Charges every dispatch that this frame answered.
    ///
    /// A frame answers every dispatch that finished before its draw began.
    /// Dispatch and draw both run on the main thread and cannot interleave, so
    /// a dispatch recorded before `draw_start` had already mutated the state
    /// that `Render::render` then read. The latency is dispatch to
    /// end-of-draw, which is what "keypress to pixels" actually means.
    ///
    /// The boundary is deliberately the draw's start and not `dirty_at`. Only
    /// the *first* command between two draws sets `dirty_at`; the rest arrive
    /// after it and are still rendered by the same frame. Measuring from the
    /// invalidation would leave them pending and charge them to a later frame
    /// with a latency that includes a frame they were never waiting on — worst
    /// of all in `key-repeat-degradation`, which exists to queue several
    /// commands per frame.
    ///
    /// Closing on the next *tick* instead — which is what this used to do —
    /// measured the remainder of the frame period and almost nothing else, so a
    /// fast action and a slow one both reported about half a refresh interval.
    fn charge_actions_answered_by(&mut self, timing: &gpui::profiler::FrameTiming) {
        let answered_from = timing.draw_start;

        let mut still_pending = Vec::with_capacity(self.pending_actions.len());
        for (action, dispatched) in std::mem::take(&mut self.pending_actions) {
            if answered_from >= dispatched {
                let latency_ms = timing.draw_end.duration_since(dispatched).as_secs_f64() * 1000.0;
                self.action_latencies
                    .entry(action)
                    .or_default()
                    .push(latency_ms);
            } else {
                still_pending.push((action, dispatched));
            }
        }
        self.pending_actions = still_pending;
    }

    /// Aggregates everything and writes the run's files.
    pub fn finish(cx: &mut App) -> anyhow::Result<Option<PathBuf>> {
        if !cx.has_global::<Self>() {
            return Ok(None);
        }

        let census_total = cx.update_global::<Self, _>(|session, cx| {
            if session.finished {
                return None;
            }
            Some(session.take_census(cx))
        });
        let Some(census_total) = census_total else {
            return Ok(None);
        };

        cx.update_global::<Self, _>(|session, _| {
            session.finished = true;
            session.write(census_total).map(Some)
        })
    }

    /// Builds the report and writes `report.json`, `report.md` and the raw series.
    fn write(&mut self, census_total: usize) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(&self.config.output_dir).map_err(|error| {
            anyhow::anyhow!(
                "cannot create report directory {}: {error}",
                self.config.output_dir.display()
            )
        })?;

        // gpui holds finished frames and tasks until somebody collects them,
        // and the counter timer only does that every 500ms. A scenario ending
        // inside that window would leave its last draws and its last tasks in
        // gpui — and, worse, leave the actions those draws answered sitting in
        // `pending_actions`, which reports a critical `actions.never_drawn`
        // for work the app did.
        self.drain_frame_timings();
        self.drain_task_timings();

        // Before anything reads a statistic. `stats()` stamps the recorder's
        // current refresh interval onto every `FrameStats` and `DrawStats` it
        // produces, so a mark closed while the recorders still hold the 60Hz
        // placeholder keeps that placeholder forever — and a report would then
        // judge its marks against 16.7ms while judging its summary against the
        // display, from the same samples.
        self.frames.adopt_refresh_interval();
        self.draws
            .set_refresh_interval_ms(self.frames.refresh_interval_ms());

        for mark in std::mem::take(&mut self.open_marks) {
            log::warn!(
                "mark {:?} was never closed; reporting it up to end of run",
                mark.name
            );
            self.completed_marks.push(CompletedMark {
                name: mark.name,
                wall_ms: mark.started.elapsed().as_secs_f64() * 1000.0,
                frames: mark.start_frame..self.frames.len(),
                drawn_from: mark.started,
                drawn_to: Instant::now(),
                // The delta, as `end_mark` computes it. Recording the starting
                // footprint instead turned an ordinary 100-200MB baseline into
                // 100-200MB of apparent growth, well past the 20MB threshold,
                // so every salvaged report accused itself of a memory leak.
                working_set_delta_bytes: self.last_process.working_set_bytes as i64
                    - mark.start_working_set as i64,
            });
        }

        let peak_census = self.peak_census_bytes.max(census_total);
        let mut actions: Vec<ActionReport> = self
            .action_latencies
            .iter()
            .map(|(action, latencies)| {
                let mut sorted = latencies.clone();
                sorted.sort_by(|a, b| a.total_cmp(b));
                ActionReport {
                    action: action.clone(),
                    count: sorted.len() as u64,
                    p50_ms: crate::frame::percentile(&sorted, 0.50),
                    p95_ms: crate::frame::percentile(&sorted, 0.95),
                    max_ms: sorted.last().copied().unwrap_or(0.0),
                }
            })
            .collect();
        actions.sort_by(|a, b| b.p95_ms.total_cmp(&a.p95_ms));

        let mut report = Report {
            schema_version: SCHEMA_VERSION,
            run: RunInfo {
                started_at: chrono::Utc::now().to_rfc3339(),
                scenario: self
                    .config
                    .scenario
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| self.config.mode.label().to_string()),
                corpus: self.config.corpus.clone(),
                revision: git_revision(),
                profile: build_profile().to_string(),
                heap_profiling: crate::is_heap_profiling(),
                cpu_sampling: self.cpu.is_some(),
                os: std::env::consts::OS.to_string(),
                cpu_count: std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(0),
            },
            summary: Summary {
                frames: self.frames.stats_all(),
                draws: self.draws.stats_all(),
                time_to_first_content_ms: self
                    .first_content
                    .map(|elapsed| elapsed.as_secs_f64() * 1000.0),
                worst_foreground_task_ms: self.tasks.worst_foreground_ms(),
                final_process: self.last_process,
                peak_working_set_bytes: self.peak_working_set,
                peak_private_bytes: (self.peak_private_bytes > 0)
                    .then_some(self.peak_private_bytes),
                peak_census_bytes: peak_census,
                // Peak against peak. Subtracting a peak census from a final
                // reading charged the run for however much the process shrank
                // after its high-water mark, which is a bias, not a measurement.
                // `None` rather than a number where the platform does not
                // report private bytes at all — on those the old form produced
                // a large negative from a hardcoded zero, and printed it.
                unaccounted_bytes: (self.peak_private_bytes > 0)
                    .then(|| self.peak_private_bytes as i64 - peak_census as i64),
                actions_without_a_frame: self.pending_actions.len() as u64,
                settle_timeouts: self.settle_timeouts,
                cpu_percent_mean: self.mean_cpu_percent(),
                gpu: self
                    .samples
                    .last()
                    .map(|sample| sample.gpu.clone())
                    .unwrap_or_default(),
            },
            findings: Vec::new(),
            marks: std::mem::take(&mut self.completed_marks)
                .into_iter()
                .map(|mark| MarkReport {
                    name: mark.name,
                    wall_ms: mark.wall_ms,
                    frames: self.frames.stats(mark.frames),
                    draws: self.draws.stats_between(mark.drawn_from, mark.drawn_to),
                    working_set_delta_bytes: mark.working_set_delta_bytes,
                })
                .collect(),
            actions,
            hot_locations: self.tasks.stats(),
            heap: std::mem::take(&mut self.peak_census_nodes),
        };

        report.truncate_tables();
        report.detect_findings(&self.config.thresholds);

        let dir = &self.config.output_dir;
        std::fs::write(dir.join("report.json"), report.to_json()?)?;
        std::fs::write(dir.join("report.md"), report.to_markdown())?;
        std::fs::write(dir.join("samples.jsonl"), to_jsonl(&self.samples)?)?;
        std::fs::write(dir.join("frames.jsonl"), frames_to_jsonl(&self.frames))?;
        if !self.trace.is_empty() {
            std::fs::write(dir.join("trace.jsonl"), to_jsonl(&self.trace)?)?;
        }

        let critical = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        log::info!(
            "perf report written to {} ({} finding(s), {critical} critical)",
            dir.display(),
            report.findings.len()
        );

        Ok(dir.clone())
    }

    /// Mean CPU as a percentage of one core across the whole run.
    fn mean_cpu_percent(&self) -> Option<f64> {
        let first = self.samples.first()?;
        let last = self.samples.last()?;
        let wall_micros = ((last.t_ms - first.t_ms) * 1000.0) as u64;
        ProcessSample::cpu_percent_between(&first.process, &last.process, wall_micros)
    }
}

/// Re-registers a frame callback from inside the previous one.
///
/// gpui drains these on every vsync tick regardless of whether it draws, so the
/// chain rides the real frame clock. It deliberately does not call
/// `request_animation_frame`: forcing a redraw every tick would hide genuine
/// stalls behind constant work and invent GPU load the app would never do.
fn arm_frame_chain(window: &Window) {
    window.on_next_frame(move |window, cx| {
        let now = Instant::now();
        if cx.has_global::<PerfSession>() {
            cx.update_global::<PerfSession, _>(|session, _| session.tick(now));
        }
        arm_frame_chain(window);
    });
}

/// Hashes a [`std::thread::ThreadId`] the way gpui's profiler does.
///
/// `ThreadTimingsDelta` identifies its thread by a hash rather than the id
/// itself, so recognising the frame thread among the deltas means reproducing
/// that hash. Both sides use `DefaultHasher`, whose output is stable within one
/// process run — which is all this needs, since both sides are this process.
fn hashed_thread_id(id: std::thread::ThreadId) -> u64 {
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

/// Reads the OS counters twice a second for the life of the run.
fn start_counter_timer(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(COUNTER_INTERVAL).await;
        let still_running = cx.update(|cx| {
            if !cx.has_global::<PerfSession>() {
                return false;
            }
            cx.update_global::<PerfSession, _>(|session, _| {
                if session.finished {
                    return false;
                }
                session.sample_counters();
                true
            })
        });
        if !still_running {
            break;
        }
    })
    .detach();
}

/// Fails a run whose scenario names a command that no longer exists.
///
/// Checked before the first step rather than at the step, so a stale scenario
/// is a clear error instead of a run that quietly measures fewer actions than
/// it claims to.
fn verify_actions_exist(scenario: &Scenario, cx: &App) -> anyhow::Result<()> {
    let registered = cx.all_action_names();
    let missing: Vec<&str> = scenario
        .referenced_actions()
        .into_iter()
        .filter(|name| !registered.contains(name))
        .collect();
    anyhow::ensure!(
        missing.is_empty(),
        "scenario {:?} names {} action(s) this build does not have: {}",
        scenario.name,
        missing.len(),
        missing.join(", ")
    );
    Ok(())
}

/// Cargo profile this binary was built with, so a report never implies that a
/// debug build's numbers are comparable to a release one's.
fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release-like"
    }
}

/// Short git revision of the tree under test, when it can be determined.
fn git_revision() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

/// Serialises a slice as newline-delimited JSON.
fn to_jsonl<T: Serialize>(items: &[T]) -> anyhow::Result<String> {
    let mut out = String::new();
    for item in items {
        out.push_str(&serde_json::to_string(item)?);
        out.push('\n');
    }
    Ok(out)
}

/// Writes frame intervals as one compact record per tick.
fn frames_to_jsonl(frames: &FrameRecorder) -> String {
    let mut out = String::new();
    for (index, interval) in frames.intervals_ms().iter().enumerate() {
        out.push_str(&format!("{{\"i\":{index},\"ms\":{interval:.4}}}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replay_config_records_the_corpus_its_scenario_declares() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("scenario.json");
        std::fs::write(
            &path,
            r#"{"name":"probe","corpus":"medium","steps":[{"wait":{"ms":1}}]}"#,
        )
        .unwrap();

        let config = SessionConfig::from_mode(Mode::Replay(path)).unwrap();

        // `compare` warns when two runs used different inputs, and it can only
        // do that if the run metadata carries which one was used.
        assert_eq!(config.corpus.as_deref(), Some("medium"));
    }

    #[test]
    fn jsonl_writes_one_record_per_line() {
        let samples = vec![
            TimeSample {
                t_ms: 0.0,
                process: ProcessSample::default(),
                gpu: GpuSample::default(),
                census_bytes: None,
            },
            TimeSample {
                t_ms: 500.0,
                process: ProcessSample::default(),
                gpu: GpuSample::default(),
                census_bytes: Some(1024),
            },
        ];

        let jsonl = to_jsonl(&samples).unwrap();
        assert_eq!(jsonl.lines().count(), 2);
        assert!(jsonl.lines().all(|line| line.starts_with('{')));
    }

    #[test]
    fn config_places_reports_under_a_timestamped_directory() {
        let config = SessionConfig::from_mode(Mode::Record).unwrap();
        let name = config
            .output_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.ends_with("-record"), "directory name: {name}");
        assert!(config.scenario.is_none());
    }
}
