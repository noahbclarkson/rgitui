//! What a frame *costs*, as distinct from how often frames arrive.
//!
//! [`crate::frame`] measures the interval between vsync ticks, which is what
//! decides whether scrolling feels smooth on this machine. It is also the
//! reason a healthy run is nearly unreadable: every interval sits on the
//! refresh period, so p50 is 10.00ms whether the app spent 1ms drawing or 9ms,
//! and the two look identical right up until the moment the second one starts
//! dropping frames.
//!
//! This module measures the other half, from gpui's own per-frame record:
//!
//! - **draw duration** — CPU time inside `Window::draw`. Against the refresh
//!   interval it gives *headroom*, which is the only figure here that predicts
//!   behaviour on hardware slower than the machine under the desk. A frame
//!   costing 2ms of a 9.75ms budget survives a 4x slower machine; one costing
//!   6ms does not, and both look like a flat 10.00ms cadence today.
//! - **dirty-to-draw** — from a frame's first invalidation to the end of its
//!   draw. This is the app's contribution to input latency: the keystroke is
//!   already handled, and this is what remains before anything reaches the
//!   screen.
//! - **invalidations per frame** — how many `notify()` calls coalesced into one
//!   draw. Consistently high counts mean the app is asking to redraw far more
//!   often than it has anything new to show, which costs nothing visible here
//!   and costs a great deal on a machine that cannot absorb it.

use serde::{Deserialize, Serialize};

use crate::frame::percentile;

/// Aggregated per-frame cost over one region of a run. Durations in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawStats {
    /// Frames gpui recorded a draw for.
    pub draws: u64,
    /// Median CPU time inside `Window::draw`.
    pub draw_p50_ms: f64,
    /// 95th percentile draw duration — what a slower machine multiplies.
    pub draw_p95_ms: f64,
    /// 99th percentile draw duration.
    pub draw_p99_ms: f64,
    /// Worst single draw.
    pub draw_max_ms: f64,
    /// Total CPU time spent drawing across the region.
    pub draw_total_ms: f64,
    /// Median time from a frame's first invalidation to the end of its draw.
    pub dirty_to_draw_p50_ms: f64,
    /// 95th percentile invalidation-to-drawn latency.
    pub dirty_to_draw_p95_ms: f64,
    /// Worst invalidation-to-drawn latency.
    pub dirty_to_draw_max_ms: f64,
    /// Invalidations coalesced into the median frame.
    pub invalidations_p50: f64,
    /// Invalidations coalesced into the 95th-percentile frame.
    pub invalidations_p95: f64,
    /// Total invalidations across the region.
    pub invalidations_total: u64,
    /// Frames gpui drew that nothing had invalidated.
    ///
    /// The authoritative form of the question, taken from gpui's own count.
    /// Deriving it from "did the app dispatch a command recently" instead files
    /// every hover, scroll and animation tick as an idle repaint, and the
    /// finding that follows sends someone hunting a repaint loop that was never
    /// there.
    pub idle_redraws: u64,
    /// Refresh interval the headroom figures are judged against.
    pub refresh_interval_ms: f64,
}

impl DrawStats {
    /// Share of one frame's budget the median draw consumes, as a percentage.
    pub fn budget_used_p50_percent(&self) -> f64 {
        self.percent_of_budget(self.draw_p50_ms)
    }

    /// Share of one frame's budget the p95 draw consumes, as a percentage.
    pub fn budget_used_p95_percent(&self) -> f64 {
        self.percent_of_budget(self.draw_p95_ms)
    }

    /// How much slower a machine could be before p95 draws stop fitting in the
    /// frame budget.
    ///
    /// This is the headroom figure: 4.0 means the work would still fit on a
    /// machine four times slower, while anything at or below 1.0 means frames
    /// are already over budget here. It deliberately reads CPU work only —
    /// a slower GPU or a slower disk is a different question — so treat it as
    /// an upper bound on how much slack there is, not a guarantee.
    pub fn cpu_slowdown_headroom(&self) -> Option<f64> {
        if self.draw_p95_ms <= 0.0 || self.refresh_interval_ms <= 0.0 {
            return None;
        }
        Some(self.refresh_interval_ms / self.draw_p95_ms)
    }

    fn percent_of_budget(&self, ms: f64) -> f64 {
        if self.refresh_interval_ms <= 0.0 {
            return 0.0;
        }
        ms / self.refresh_interval_ms * 100.0
    }
}

/// One frame's cost, reduced to the three numbers this module reports on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawSample {
    /// When the draw finished, taken from gpui's own `FrameTiming`.
    ///
    /// Draws reach this recorder in batches on a timer rather than as they
    /// happen, so their position in the buffer says nothing about when they
    /// occurred. A measurement region has to select them by this instead.
    pub drawn_at: std::time::Instant,
    /// CPU milliseconds inside `Window::draw`.
    pub draw_ms: f64,
    /// Milliseconds from first invalidation to end of draw, when gpui observed
    /// the invalidation. Frames already dirty before tracing began have none.
    pub dirty_to_draw_ms: Option<f64>,
    /// Invalidations coalesced into this frame.
    pub invalidations: u64,
}

/// Collects per-frame costs for the lifetime of a run.
#[derive(Debug, Default)]
pub struct DrawRecorder {
    samples: Vec<DrawSample>,
    refresh_interval_ms: f64,
}

impl DrawRecorder {
    /// Starts recording against the display's refresh interval, which is what
    /// the headroom figures are measured against.
    pub fn new(refresh_interval_ms: f64) -> Self {
        Self {
            samples: Vec::new(),
            refresh_interval_ms,
        }
    }

    /// Records one drawn frame.
    pub fn record(&mut self, sample: DrawSample) {
        self.samples.push(sample);
    }

    /// Adopts the refresh interval measured from real ticks.
    ///
    /// Headroom is a ratio against the frame budget, so it is only meaningful
    /// once the budget is the display's rather than the 60Hz guess a run starts
    /// with. Called at the end of a run, alongside the cadence calibration.
    pub fn set_refresh_interval_ms(&mut self, refresh_interval_ms: f64) {
        self.refresh_interval_ms = refresh_interval_ms;
    }

    /// Frames recorded so far, so a caller can slice a region by index the way
    /// marks already slice frame intervals.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Aggregates the frames in `range`.
    ///
    /// An out-of-bounds or inverted range yields empty statistics rather than
    /// panicking: marks are closed by scenarios, and a scenario that closes a
    /// mark it never opened should not take the run down with it.
    pub fn stats(&self, range: std::ops::Range<usize>) -> DrawStats {
        let start = range.start.min(self.samples.len());
        let end = range.end.min(self.samples.len()).max(start);
        let window = &self.samples[start..end];

        if window.is_empty() {
            return DrawStats {
                refresh_interval_ms: self.refresh_interval_ms,
                ..DrawStats::default()
            };
        }

        let mut draws: Vec<f64> = window.iter().map(|sample| sample.draw_ms).collect();
        draws.sort_by(f64::total_cmp);

        let mut latencies: Vec<f64> = window
            .iter()
            .filter_map(|sample| sample.dirty_to_draw_ms)
            .collect();
        latencies.sort_by(f64::total_cmp);

        let mut invalidations: Vec<f64> = window
            .iter()
            .map(|sample| sample.invalidations as f64)
            .collect();
        invalidations.sort_by(f64::total_cmp);

        DrawStats {
            draws: window.len() as u64,
            draw_p50_ms: percentile(&draws, 0.50),
            draw_p95_ms: percentile(&draws, 0.95),
            draw_p99_ms: percentile(&draws, 0.99),
            draw_max_ms: draws.last().copied().unwrap_or_default(),
            draw_total_ms: draws.iter().sum(),
            dirty_to_draw_p50_ms: percentile(&latencies, 0.50),
            dirty_to_draw_p95_ms: percentile(&latencies, 0.95),
            dirty_to_draw_max_ms: latencies.last().copied().unwrap_or_default(),
            invalidations_p50: percentile(&invalidations, 0.50),
            invalidations_p95: percentile(&invalidations, 0.95),
            invalidations_total: window.iter().map(|sample| sample.invalidations).sum(),
            idle_redraws: window
                .iter()
                .filter(|sample| sample.invalidations == 0)
                .count() as u64,
            refresh_interval_ms: self.refresh_interval_ms,
        }
    }

    /// Aggregates the frames drawn within a wall-clock window.
    ///
    /// This is what a measurement region must use. Draws are drained from gpui
    /// on a timer, so slicing the buffer by the index it happened to have when
    /// a mark opened and closed puts up to one drain interval of unrelated
    /// frames inside the region at each edge — and leaves the region's frame
    /// intervals, which are recorded synchronously, describing a different span
    /// of the run than its draws do.
    pub fn stats_between(&self, start: std::time::Instant, end: std::time::Instant) -> DrawStats {
        let first = self
            .samples
            .partition_point(|sample| sample.drawn_at < start);
        let last = self
            .samples
            .partition_point(|sample| sample.drawn_at <= end);
        self.stats(first..last)
    }

    /// Aggregates every frame recorded.
    pub fn stats_all(&self) -> DrawStats {
        self.stats(0..self.samples.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(draw_ms: f64, dirty_to_draw_ms: Option<f64>, invalidations: u64) -> DrawSample {
        DrawSample {
            drawn_at: std::time::Instant::now(),
            draw_ms,
            dirty_to_draw_ms,
            invalidations,
        }
    }

    /// A sample stamped at a chosen offset from `origin`, for the tests that
    /// care about which frames a wall-clock window selects.
    fn sample_at(origin: std::time::Instant, offset_ms: u64, draw_ms: f64) -> DrawSample {
        DrawSample {
            drawn_at: origin + std::time::Duration::from_millis(offset_ms),
            draw_ms,
            dirty_to_draw_ms: None,
            invalidations: 1,
        }
    }

    #[test]
    fn a_window_selects_the_frames_drawn_inside_it_whatever_order_they_arrived() {
        // The failure this guards against: draws reach the recorder in batches
        // on a timer, so a region that slices by buffer index picks up frames
        // from up to a whole drain interval either side of itself.
        let origin = std::time::Instant::now();
        let mut recorder = DrawRecorder::new(10.0);
        for (offset, cost) in [(0, 1.0), (50, 2.0), (100, 3.0), (150, 4.0), (200, 5.0)] {
            recorder.record(sample_at(origin, offset, cost));
        }

        let stats = recorder.stats_between(
            origin + std::time::Duration::from_millis(50),
            origin + std::time::Duration::from_millis(150),
        );

        assert_eq!(stats.draws, 3, "the frames at 50ms, 100ms and 150ms");
        assert_eq!(stats.draw_total_ms, 9.0, "2.0 + 3.0 + 4.0");
    }

    #[test]
    fn a_window_with_no_frames_in_it_reports_none_rather_than_the_neighbours() {
        let origin = std::time::Instant::now();
        let mut recorder = DrawRecorder::new(10.0);
        recorder.record(sample_at(origin, 0, 1.0));
        recorder.record(sample_at(origin, 500, 2.0));

        let stats = recorder.stats_between(
            origin + std::time::Duration::from_millis(100),
            origin + std::time::Duration::from_millis(200),
        );

        assert_eq!(stats.draws, 0);
        assert_eq!(
            stats.refresh_interval_ms, 10.0,
            "an empty window still knows what budget it was judged against"
        );
    }

    #[test]
    fn the_window_boundaries_are_inclusive_at_both_ends() {
        // A mark opens and closes on an instant, and a frame drawn on exactly
        // that instant belongs to it. Excluding either edge would drop the
        // first frame of a region that starts by triggering a repaint.
        let origin = std::time::Instant::now();
        let mut recorder = DrawRecorder::new(10.0);
        recorder.record(sample_at(origin, 100, 1.0));
        recorder.record(sample_at(origin, 200, 2.0));

        let stats = recorder.stats_between(
            origin + std::time::Duration::from_millis(100),
            origin + std::time::Duration::from_millis(200),
        );

        assert_eq!(stats.draws, 2);
    }

    #[test]
    fn stats_summarise_draw_cost_and_latency_separately() {
        let mut recorder = DrawRecorder::new(10.0);
        for ms in [1.0, 2.0, 3.0, 4.0] {
            recorder.record(sample(ms, Some(ms * 2.0), 1));
        }

        let stats = recorder.stats_all();
        assert_eq!(stats.draws, 4);
        assert_eq!(stats.draw_max_ms, 4.0);
        assert_eq!(stats.draw_total_ms, 10.0);
        assert_eq!(stats.dirty_to_draw_max_ms, 8.0);
    }

    #[test]
    fn headroom_says_how_much_slower_a_machine_could_be() {
        let mut recorder = DrawRecorder::new(10.0);
        // Every frame costs 2ms of a 10ms budget.
        for _ in 0..20 {
            recorder.record(sample(2.0, None, 1));
        }

        let stats = recorder.stats_all();
        assert_eq!(stats.cpu_slowdown_headroom(), Some(5.0));
        assert_eq!(stats.budget_used_p95_percent(), 20.0);
    }

    #[test]
    fn a_frame_already_over_budget_reports_headroom_at_or_below_one() {
        let mut recorder = DrawRecorder::new(10.0);
        for _ in 0..10 {
            recorder.record(sample(12.0, None, 1));
        }

        let headroom = recorder.stats_all().cpu_slowdown_headroom().unwrap();
        assert!(headroom < 1.0, "12ms of a 10ms budget is over: {headroom}");
    }

    #[test]
    fn frames_without_an_observed_invalidation_do_not_count_as_zero_latency() {
        // A frame already dirty before tracing began carries no `dirty_at`.
        // Treating that as 0ms would drag the median toward zero and make the
        // app look more responsive than it is.
        let mut recorder = DrawRecorder::new(10.0);
        recorder.record(sample(1.0, None, 1));
        recorder.record(sample(1.0, Some(6.0), 1));
        recorder.record(sample(1.0, Some(8.0), 1));

        let stats = recorder.stats_all();
        assert_eq!(stats.draws, 3);
        // Median of the two observed latencies. Had the unobserved frame been
        // counted as 0ms the set would be [0, 6, 8] and the median 6.0, so this
        // value is what distinguishes "excluded" from "counted as instant".
        assert_eq!(stats.dirty_to_draw_p50_ms, 7.0);
        assert_eq!(stats.dirty_to_draw_max_ms, 8.0);
    }

    #[test]
    fn invalidation_counts_expose_redundant_notifies() {
        let mut recorder = DrawRecorder::new(10.0);
        for count in [1, 1, 1, 40] {
            recorder.record(sample(1.0, None, count));
        }

        let stats = recorder.stats_all();
        assert_eq!(stats.invalidations_total, 43);
        assert_eq!(stats.invalidations_p50, 1.0);
        assert!(
            stats.invalidations_p95 > 30.0,
            "the spike has to survive into p95 or it cannot be seen: {}",
            stats.invalidations_p95
        );
    }

    #[test]
    fn an_empty_region_reports_no_draws_rather_than_panicking() {
        let recorder = DrawRecorder::new(10.0);
        // Built from values rather than written as a literal reversed range,
        // which rustc rejects outright.
        let (start, end) = (5, 2);
        let stats = recorder.stats(start..end);
        assert_eq!(stats.draws, 0);
        assert_eq!(stats.cpu_slowdown_headroom(), None);
        assert_eq!(stats.refresh_interval_ms, 10.0);
    }
}
