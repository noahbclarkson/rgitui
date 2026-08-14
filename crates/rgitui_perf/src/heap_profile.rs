//! Allocation-site heap profiling, for the questions the census cannot answer.
//!
//! [`crate::heap`] reports memory by feature — "the diff cache holds 31 MB" —
//! which is what you want when deciding what to cache less of. It is blind to
//! anything rgitui does not own: allocations inside gpui's element arena,
//! libgit2's object cache, syntect's parser state. This module covers that gap
//! by replacing the global allocator and recording a backtrace per allocation.
//!
//! # Use it in its own run
//!
//! dhat costs 2-5x runtime. Frame timings collected under it describe dhat, so
//! the report labels any such run as timing-invalid rather than trusting anyone
//! to remember. Build with `--features perf-dhat`, read the heap output, and
//! measure timings in a separate `--features perf` run.
//!
//! # Reading the output
//!
//! `dhat-heap.json` is far too large to read directly — a short run produces
//! tens of megabytes of backtraces. Either load it into
//! <https://nnethercote.github.io/dh_view/dh_view.html>, or use the roll-up in
//! [`summarize_sites`], which collapses each backtrace to its first frame
//! belonging to rgitui and ranks what is left.

use serde::{Deserialize, Serialize};

#[cfg(feature = "dhat-heap")]
pub use dhat::Alloc;

/// Holds the profiler open for the life of the process.
///
/// dhat writes its output when this is dropped, so it has to outlive
/// everything being measured — bind it in `main` and leave it bound.
#[cfg(feature = "dhat-heap")]
pub struct HeapProfiler {
    _profiler: dhat::Profiler,
}

#[cfg(feature = "dhat-heap")]
impl HeapProfiler {
    /// Starts profiling. The output lands in `dhat-heap.json` in the working
    /// directory when the returned value drops.
    pub fn start() -> Self {
        log::warn!(
            "dhat heap profiling is active — this run is 2-5x slower than normal and its \
             timings describe dhat, not rgitui"
        );
        Self {
            _profiler: dhat::Profiler::new_heap(),
        }
    }
}

/// One allocation site, rolled up from every backtrace that passed through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationSite {
    /// The first frame in the backtrace that belongs to rgitui rather than to
    /// `std`, `alloc` or the allocator shim.
    pub frame: String,
    /// Bytes still live at the point of peak total heap usage. This is the
    /// number that matters for footprint: total-ever-allocated rewards
    /// short-lived churn that costs nothing at rest.
    pub bytes_at_peak: u64,
    /// Blocks live at peak.
    pub blocks_at_peak: u64,
    /// Bytes allocated over the whole run, live or not. High values with low
    /// `bytes_at_peak` mean allocation churn — a CPU cost, not a memory one.
    pub total_bytes: u64,
}

/// Rolls a `dhat-heap.json` up into ranked allocation sites.
///
/// dhat's format is a tree of program points, each carrying an index into a
/// shared frame table. Collapsing each to its first non-runtime frame is what
/// turns "`__rust_alloc` allocated everything" into a list of names worth
/// reading.
pub fn summarize_sites(dhat_json: &str, limit: usize) -> anyhow::Result<Vec<AllocationSite>> {
    let parsed: DhatFile = serde_json::from_str(dhat_json)
        .map_err(|error| anyhow::anyhow!("cannot parse dhat output: {error}"))?;

    let mut by_frame: std::collections::HashMap<String, AllocationSite> =
        std::collections::HashMap::new();

    for point in &parsed.pps {
        let frame = point
            .fs
            .iter()
            .filter_map(|index| parsed.ftbl.get(*index))
            .map(|frame| frame_name(frame))
            .find(|frame| is_interesting(frame))
            .unwrap_or("<runtime>")
            .to_string();

        let entry = by_frame
            .entry(frame.clone())
            .or_insert_with(|| AllocationSite {
                frame,
                bytes_at_peak: 0,
                blocks_at_peak: 0,
                total_bytes: 0,
            });
        entry.bytes_at_peak += point.gb.unwrap_or(0);
        entry.blocks_at_peak += point.gbk.unwrap_or(0);
        entry.total_bytes += point.tb.unwrap_or(0);
    }

    let mut sites: Vec<AllocationSite> = by_frame.into_values().collect();
    sites.sort_by(|a, b| b.bytes_at_peak.cmp(&a.bytes_at_peak));
    sites.truncate(limit);
    Ok(sites)
}

/// Whether a backtrace frame names something worth attributing an allocation to.
///
/// Every allocation passes through the allocator shim and most pass through a
/// container's growth path, so those frames appear at the top of nearly every
/// backtrace and identify nothing. Skipping them is what makes the roll-up a
/// list of distinct call sites rather than one row for `__rust_alloc`.
fn is_interesting(frame: &str) -> bool {
    const RUNTIME_PREFIXES: [&str; 8] = [
        "__rust_alloc",
        "alloc::",
        "core::",
        "std::",
        "<alloc::",
        "<core::",
        "<std::",
        "dhat::",
    ];

    !RUNTIME_PREFIXES
        .iter()
        .any(|prefix| frame.starts_with(prefix))
}

/// Strips dhat's `#N: ` prefix, which records the frame's depth in one
/// particular backtrace rather than anything about the function.
///
/// Stripping it before it becomes a map key is what lets the same function
/// merge into one row when it is reached at different depths — otherwise a hot
/// site called from two places reports as two half-sized ones and neither looks
/// worth investigating.
fn frame_name(frame: &str) -> &str {
    frame
        .split_once(':')
        .filter(|(prefix, _)| prefix.starts_with('#'))
        .map(|(_, rest)| rest.trim())
        .unwrap_or(frame)
        .trim()
}

/// The subset of dhat's output format this roll-up reads.
///
/// Deliberately partial: dhat carries per-block lifetime and access histograms
/// that the interactive viewer uses and a ranked list does not.
#[derive(Deserialize)]
struct DhatFile {
    /// Frame table — backtrace entries reference these by index.
    ftbl: Vec<String>,
    /// Program points, each one distinct backtrace.
    pps: Vec<DhatProgramPoint>,
}

#[derive(Deserialize)]
struct DhatProgramPoint {
    /// Indices into [`DhatFile::ftbl`], innermost frame first.
    fs: Vec<usize>,
    /// Bytes live at global peak.
    gb: Option<u64>,
    /// Blocks live at global peak.
    gbk: Option<u64>,
    /// Total bytes ever allocated here.
    tb: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal file in dhat's shape: two program points whose backtraces both
    /// start in the runtime and diverge at the first rgitui frame.
    const SAMPLE: &str = r##"{
        "ftbl": [
            "#0: __rust_alloc",
            "#1: alloc::raw_vec::RawVec<T>::grow",
            "#2: rgitui_git::project::refresh::gather",
            "#3: rgitui_diff::DiffViewer::prepare_rows",
            "#4: main"
        ],
        "pps": [
            { "fs": [0, 1, 2, 4], "gb": 4000, "gbk": 10, "tb": 9000 },
            { "fs": [0, 1, 3, 4], "gb": 7000, "gbk": 20, "tb": 8000 }
        ]
    }"##;

    #[test]
    fn sites_are_attributed_past_the_allocator_and_container_frames() {
        let sites = summarize_sites(SAMPLE, 10).unwrap();
        assert_eq!(sites.len(), 2);
        assert!(
            sites.iter().all(|site| site.frame.starts_with("rgitui")),
            "runtime frames identify nothing: {sites:?}"
        );
    }

    #[test]
    fn sites_are_ranked_by_bytes_live_at_peak() {
        let sites = summarize_sites(SAMPLE, 10).unwrap();
        assert_eq!(sites[0].frame, "rgitui_diff::DiffViewer::prepare_rows");
        assert_eq!(sites[0].bytes_at_peak, 7000);
        assert_eq!(sites[1].bytes_at_peak, 4000);
    }

    #[test]
    fn the_same_function_at_different_stack_depths_is_one_site() {
        // The same allocating function reached from two call paths, so dhat
        // records it at depth 2 in one backtrace and depth 3 in the other.
        let two_depths = r##"{
            "ftbl": [
                "#0: __rust_alloc",
                "#1: alloc::vec::Vec<T>::push",
                "#2: rgitui_git::load",
                "#3: rgitui_git::load",
                "#4: caller_a",
                "#5: caller_b"
            ],
            "pps": [
                { "fs": [0, 2, 4], "gb": 100, "gbk": 1, "tb": 100 },
                { "fs": [0, 1, 3, 5], "gb": 200, "gbk": 2, "tb": 200 }
            ]
        }"##;

        let sites = summarize_sites(two_depths, 10).unwrap();
        assert_eq!(
            sites.len(),
            1,
            "depth is not identity — these are one site: {sites:?}"
        );
        assert_eq!(sites[0].frame, "rgitui_git::load");
        assert_eq!(sites[0].bytes_at_peak, 300);
    }

    #[test]
    fn backtraces_sharing_a_site_are_summed_into_one_row() {
        let doubled = r##"{
            "ftbl": ["#0: __rust_alloc", "#1: rgitui_git::load", "#2: a", "#3: b"],
            "pps": [
                { "fs": [0, 1, 2], "gb": 100, "gbk": 1, "tb": 100 },
                { "fs": [0, 1, 3], "gb": 250, "gbk": 2, "tb": 250 }
            ]
        }"##;

        let sites = summarize_sites(doubled, 10).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].bytes_at_peak, 350);
        assert_eq!(sites[0].blocks_at_peak, 3);
    }

    #[test]
    fn a_backtrace_that_is_entirely_runtime_is_still_reported() {
        let runtime_only = r##"{
            "ftbl": ["#0: __rust_alloc", "#1: std::io::read"],
            "pps": [{ "fs": [0, 1], "gb": 64, "gbk": 1, "tb": 64 }]
        }"##;

        let sites = summarize_sites(runtime_only, 10).unwrap();
        assert_eq!(sites[0].frame, "<runtime>");
        assert_eq!(sites[0].bytes_at_peak, 64);
    }

    #[test]
    fn the_limit_is_applied_after_ranking() {
        let sites = summarize_sites(SAMPLE, 1).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].frame, "rgitui_diff::DiffViewer::prepare_rows");
    }

    #[test]
    fn malformed_output_is_reported_rather_than_silently_empty() {
        let error = summarize_sites("not json", 10).unwrap_err().to_string();
        assert!(error.contains("cannot parse dhat output"), "{error}");
    }
}
