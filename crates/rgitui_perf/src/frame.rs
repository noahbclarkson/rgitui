//! Presented-frame cadence — the number that decides whether scrolling feels smooth.
//!
//! # Why cadence and not draw duration
//!
//! Zed's own 120fps work found frame *durations* consistently under 4ms while
//! frames were still being dropped: the cost of drawing and the rate of
//! presenting are different measurements, and only the second one is what a
//! user perceives. So this module measures the interval between vsync ticks,
//! not the time inside `draw`.
//!
//! # How it hooks in without distorting what it measures
//!
//! gpui drains `Window::on_next_frame` callbacks on every vsync tick, before
//! the `invalidator.is_dirty()` check that decides whether to actually draw. A
//! callback that re-registers itself therefore rides the real frame clock for
//! as long as the window is ticking.
//!
//! Deliberately absent: `Window::request_animation_frame`. It would force a
//! redraw every tick, which would both hide real stalls and invent GPU work
//! that the app would not otherwise do. Riding the natural clock also makes
//! *idle* redraws visible, which is its own bug class — an app that keeps
//! drawing while nothing changed is burning battery for nothing.

use serde::{Deserialize, Serialize};

/// Aggregated frame-interval statistics over one region of a run.
///
/// All durations are milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameStats {
    /// Ticks observed.
    pub frames: u64,
    /// Ticks on which the window actually redrew.
    pub drawn_frames: u64,
    /// Wall time the region covered.
    pub duration_ms: f64,
    /// Median interval between ticks.
    pub p50_ms: f64,
    /// 95th percentile interval — the one that shows up as occasional hitching.
    pub p95_ms: f64,
    /// 99th percentile interval.
    pub p99_ms: f64,
    /// Worst single interval in the region.
    pub max_ms: f64,
    /// Intervals longer than 1.5x the display's refresh interval, i.e. frames
    /// the user would perceive as dropped.
    pub dropped_frames: u64,
    /// Refresh interval this region was judged against.
    pub refresh_interval_ms: f64,
}

impl FrameStats {
    /// Mean frames per second across the region.
    pub fn fps(&self) -> f64 {
        if self.duration_ms <= 0.0 {
            return 0.0;
        }
        self.frames as f64 / (self.duration_ms / 1000.0)
    }
}

/// Collects frame intervals for the lifetime of a run.
#[derive(Debug, Default)]
pub struct FrameRecorder {
    intervals_ms: Vec<f64>,
    drawn: Vec<bool>,
    refresh_interval_ms: f64,
}

impl FrameRecorder {
    /// Starts recording against an assumed display refresh interval.
    ///
    /// The interval decides what counts as a dropped frame, so it is taken from
    /// the display rather than assumed to be 60Hz.
    pub fn new(refresh_interval_ms: f64) -> Self {
        Self {
            intervals_ms: Vec::new(),
            drawn: Vec::new(),
            refresh_interval_ms,
        }
    }

    /// Records one tick.
    pub fn record(&mut self, interval_ms: f64, drawn: bool) {
        self.intervals_ms.push(interval_ms);
        self.drawn.push(drawn);
    }

    /// Number of ticks recorded so far. Used to express scenario steps as
    /// "settle for N frames" without the caller tracking its own counter.
    pub fn len(&self) -> usize {
        self.intervals_ms.len()
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.intervals_ms.is_empty()
    }

    /// Aggregates ticks in `range` into statistics.
    pub fn stats(&self, range: std::ops::Range<usize>) -> FrameStats {
        let end = range.end.min(self.intervals_ms.len());
        let start = range.start.min(end);
        let intervals = &self.intervals_ms[start..end];
        if intervals.is_empty() {
            return FrameStats {
                refresh_interval_ms: self.refresh_interval_ms,
                ..FrameStats::default()
            };
        }

        let mut sorted = intervals.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let dropped_threshold = self.refresh_interval_ms * 1.5;

        FrameStats {
            frames: intervals.len() as u64,
            drawn_frames: self.drawn[start..end].iter().filter(|d| **d).count() as u64,
            duration_ms: intervals.iter().sum(),
            p50_ms: percentile(&sorted, 0.50),
            p95_ms: percentile(&sorted, 0.95),
            p99_ms: percentile(&sorted, 0.99),
            max_ms: *sorted.last().unwrap_or(&0.0),
            dropped_frames: intervals
                .iter()
                .filter(|ms| **ms > dropped_threshold)
                .count() as u64,
            refresh_interval_ms: self.refresh_interval_ms,
        }
    }

    /// Aggregates every tick recorded.
    pub fn stats_all(&self) -> FrameStats {
        self.stats(0..self.intervals_ms.len())
    }

    /// The raw intervals, for writing the drill-down time series.
    pub fn intervals_ms(&self) -> &[f64] {
        &self.intervals_ms
    }

    /// The refresh interval currently used to judge dropped frames.
    pub fn refresh_interval_ms(&self) -> f64 {
        self.refresh_interval_ms
    }

    /// Adopts the display's refresh interval, preferring what the OS reports.
    ///
    /// Estimating it from observed ticks is only sound while the window is
    /// busy. An idle gpui window throttles its frame callbacks well below the
    /// panel's rate — measured at ~25Hz against a 102Hz display — so a run that
    /// spends most of its time idle calibrates to the throttle and then judges
    /// every dropped frame and every headroom figure against a budget four
    /// times too generous. Asking the OS avoids inferring a constant from data
    /// that only sometimes carries it.
    pub fn adopt_refresh_interval(&mut self) {
        match display_refresh_interval_ms() {
            Some(interval) => self.refresh_interval_ms = interval,
            None => self.calibrate(),
        }
    }

    /// Derives the display's refresh interval from the ticks observed.
    ///
    /// The fallback for when the OS will not say. gpui's frame loop is paced
    /// off vsync while the window is busy, so the *most common* interval is the
    /// refresh interval — provided the window was busy. Taking the mode rather
    /// than the mean or median makes the estimate immune to stalls: a run that
    /// dropped a third of its frames still calibrates correctly, which matters
    /// because that is exactly the run whose dropped-frame count needs to be
    /// right.
    ///
    /// Leaves the existing value alone when there are too few samples to be
    /// confident, so a short run keeps its assumed default rather than
    /// calibrating to noise.
    pub fn calibrate(&mut self) {
        const MIN_SAMPLES: usize = 30;
        /// Bucket width in milliseconds. Fine enough to tell 60Hz (16.67) from
        /// 120Hz (8.33), coarse enough that jitter lands in one bucket.
        const BUCKET_MS: f64 = 0.5;

        if self.intervals_ms.len() < MIN_SAMPLES {
            return;
        }

        let mut histogram: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for interval in &self.intervals_ms {
            if *interval <= 0.0 {
                continue;
            }
            *histogram.entry((interval / BUCKET_MS) as u64).or_default() += 1;
        }

        // Ties break towards the shorter interval: mistaking 120Hz for 60Hz
        // would silently double the frame budget and hide real stutter.
        let Some(bucket) = histogram
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(bucket, _)| bucket)
        else {
            return;
        };

        self.refresh_interval_ms = (bucket as f64 + 0.5) * BUCKET_MS;
    }
}

/// The primary display's refresh interval in milliseconds, as the OS reports it.
///
/// `dmDisplayFrequency` is a whole number of hertz, so a 102.4Hz panel reports
/// 102 and the interval is out by half a percent. That is far inside the
/// tolerance of everything judged against it, and enormously better than the
/// alternative of inferring the rate from a window that throttles when idle.
#[cfg(windows)]
fn display_refresh_interval_ms() -> Option<f64> {
    use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};

    let mut mode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };

    // SAFETY: `mode` is a correctly sized DEVMODEW as required by the API, and
    // a null device name asks for the primary display.
    let ok = unsafe { EnumDisplaySettingsW(None, ENUM_CURRENT_SETTINGS, &mut mode) };
    if !ok.as_bool() {
        return None;
    }

    // 0 and 1 are the documented "unspecified / hardware default" values.
    match mode.dmDisplayFrequency {
        0 | 1 => None,
        hz => Some(1000.0 / f64::from(hz)),
    }
}

#[cfg(not(windows))]
fn display_refresh_interval_ms() -> Option<f64> {
    None
}

/// Linear-interpolated percentile of an already-sorted slice.
///
/// Interpolating matters at the small sample counts a short scenario produces:
/// nearest-rank would quantise p99 onto whichever single sample happened to
/// land there, making the metric jump between runs for no real reason.
pub fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = quantile.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let weight = rank - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_interpolates_between_samples() {
        let sorted = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&sorted, 0.0), 10.0);
        assert_eq!(percentile(&sorted, 1.0), 40.0);
        assert_eq!(percentile(&sorted, 0.5), 25.0);
    }

    #[test]
    fn percentile_handles_degenerate_inputs() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[7.0], 0.99), 7.0);
    }

    #[test]
    fn stats_count_dropped_frames_past_one_and_a_half_refresh_intervals() {
        let mut recorder = FrameRecorder::new(8.0);
        for _ in 0..10 {
            recorder.record(8.0, true);
        }
        recorder.record(13.0, true); // 1.625x — dropped
        recorder.record(11.0, true); // 1.375x — not dropped

        let stats = recorder.stats_all();
        assert_eq!(stats.frames, 12);
        assert_eq!(stats.dropped_frames, 1);
        assert_eq!(stats.max_ms, 13.0);
    }

    #[test]
    fn stats_count_the_ticks_that_drew() {
        let mut recorder = FrameRecorder::new(8.0);
        recorder.record(8.0, true);
        recorder.record(8.0, true);
        recorder.record(8.0, false);

        assert_eq!(recorder.stats_all().drawn_frames, 2);
    }

    #[test]
    fn stats_over_a_sub_range_ignore_ticks_outside_it() {
        let mut recorder = FrameRecorder::new(8.0);
        recorder.record(100.0, true);
        recorder.record(8.0, true);
        recorder.record(8.0, true);

        let stats = recorder.stats(1..3);
        assert_eq!(stats.frames, 2);
        assert_eq!(stats.max_ms, 8.0);
        assert_eq!(stats.dropped_frames, 0);
    }

    #[test]
    fn calibration_finds_the_refresh_interval_from_the_most_common_tick() {
        let mut recorder = FrameRecorder::new(16.667);
        for _ in 0..100 {
            recorder.record(8.3, true);
        }
        recorder.calibrate();
        assert!(
            (recorder.refresh_interval_ms() - 8.3).abs() < 0.5,
            "calibrated to {}",
            recorder.refresh_interval_ms()
        );
    }

    #[test]
    fn calibration_survives_a_run_that_dropped_many_frames() {
        let mut recorder = FrameRecorder::new(16.667);
        for _ in 0..60 {
            recorder.record(8.3, true);
        }
        for _ in 0..40 {
            recorder.record(50.0, true);
        }
        recorder.calibrate();

        assert!(
            (recorder.refresh_interval_ms() - 8.3).abs() < 0.5,
            "stalls must not drag the estimate: got {}",
            recorder.refresh_interval_ms()
        );
        assert_eq!(recorder.stats_all().dropped_frames, 40);
    }

    #[test]
    fn calibration_leaves_a_short_run_on_its_assumed_default() {
        let mut recorder = FrameRecorder::new(16.667);
        for _ in 0..5 {
            recorder.record(8.3, true);
        }
        recorder.calibrate();
        assert_eq!(recorder.refresh_interval_ms(), 16.667);
    }

    #[test]
    fn stats_of_an_empty_range_still_report_the_refresh_interval() {
        let recorder = FrameRecorder::new(16.67);
        let stats = recorder.stats_all();
        assert_eq!(stats.frames, 0);
        assert_eq!(stats.refresh_interval_ms, 16.67);
        assert_eq!(stats.fps(), 0.0);
    }
}
