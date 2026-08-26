//! `rgitui-perf` — generate corpora, drive runs, and read what they produced.
//!
//! The reading commands exist because a run writes more data than anyone wants
//! to open. `report.json` is already digested down to conclusions, and
//! `summarize` and `top` narrow it further for a terminal; `compare` answers
//! the only question that matters after a change, which is whether the numbers
//! moved.

use std::path::{Path, PathBuf};

use rgitui_perf::compare::{compare, Comparison, Direction, NoiseFloor};
use rgitui_perf::corpus::{self, Tier};
use rgitui_perf::driver::{trace_to_scenario, TraceEvent};
use rgitui_perf::report::{bytes_to_mb, Report, Severity};

const USAGE: &str = "\
rgitui-perf — performance harness for rgitui

USAGE:
    rgitui-perf <COMMAND> [ARGS]

COMMANDS:
    gen-corpus [TIER...]        Generate benchmark repositories. With no tier,
                                builds tiny, small, medium and pathological.
                                `large` is only built when named explicitly.
    corpus-path <TIER>          Print where a tier's repository lives.

    summarize <PATH>            Print the findings and headline numbers of a
                                run. PATH is a report.json or its directory.
    top <PATH> [N]              Print the N worst task locations (default 20).
    heap <PATH> [N]             Print the N largest heap nodes (default 20).
    dhat <PATH> [N]             Rank the N allocation sites holding the most
                                memory at peak, from a dhat-heap.json produced
                                by a `--features perf-dhat` run (default 25).
    compare <BASE> <CANDIDATE>  Diff two runs, regressions first. Exits 1 if
                                anything regressed, so it can gate a change.

    trace-to-scenario <TRACE> <OUT> [NAME]
                                Turn a recorded session into a replayable
                                scenario.

RUNNING A SCENARIO:
    Build with the harness and point it at a corpus:

        cargo run --profile perf --features perf -p rgitui -- \\
            \"$(rgitui-perf corpus-path medium)\"

    with RGITUI_PERF=replay:crates/rgitui_perf/scenarios/graph-scroll.json
    in the environment. Use RGITUI_PERF=record to measure a session you drive
    by hand instead; that report is written when you close the window.
";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        std::process::exit(2);
    };

    match command {
        "gen-corpus" => gen_corpus(&args[1..]),
        "corpus-path" => corpus_path(&args[1..]),
        "summarize" => summarize(&args[1..]),
        "top" => top(&args[1..]),
        "heap" => heap(&args[1..]),
        "dhat" => dhat(&args[1..]),
        "compare" => run_compare(&args[1..]),
        "trace-to-scenario" => convert_trace(&args[1..]),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("unknown command {other:?}\n");
            print!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn gen_corpus(args: &[String]) -> anyhow::Result<()> {
    let tiers: Vec<Tier> = if args.is_empty() {
        Tier::DEFAULT.to_vec()
    } else {
        args.iter()
            .map(|name| Tier::parse(name))
            .collect::<anyhow::Result<_>>()?
    };

    for tier in tiers {
        let started = std::time::Instant::now();
        let path = corpus::ensure(tier)?;
        println!(
            "{:<14} {}  ({:.1}s)",
            tier.name(),
            path.display(),
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

fn corpus_path(args: &[String]) -> anyhow::Result<()> {
    let name = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("corpus-path needs a tier name"))?;
    println!("{}", corpus::ensure(Tier::parse(name)?)?.display());
    Ok(())
}

/// Accepts either a `report.json` or the run directory holding one, because
/// the run directory is what gets logged and is therefore what gets pasted.
fn load_report(path: &str) -> anyhow::Result<Report> {
    let path = PathBuf::from(path);
    let file = if path.is_dir() {
        path.join("report.json")
    } else {
        path
    };
    let source = std::fs::read_to_string(&file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    Report::from_json(&source)
}

fn first_arg<'a>(args: &'a [String], command: &str) -> anyhow::Result<&'a str> {
    args.first()
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("{command} needs a path to a report"))
}

/// Optional trailing count, defaulting to `fallback`.
fn limit(args: &[String], fallback: usize) -> usize {
    args.get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn summarize(args: &[String]) -> anyhow::Result<()> {
    let report = load_report(first_arg(args, "summarize")?)?;
    let frames = &report.summary.frames;

    println!(
        "{} — {} on {} ({} profile)",
        report.run.scenario,
        report.run.revision.as_deref().unwrap_or("unknown revision"),
        report.run.os,
        report.run.profile
    );
    if let Some(corpus) = &report.run.corpus {
        println!("corpus: {corpus}");
    }
    println!();

    if report.findings.is_empty() {
        println!("FINDINGS: none — nothing crossed a threshold.");
    } else {
        println!("FINDINGS ({}):", report.findings.len());
        for finding in &report.findings {
            let marker = match finding.severity {
                Severity::Critical => "!!",
                Severity::Warning => " !",
                Severity::Info => "  ",
            };
            println!("  {marker} [{}] {}", finding.code, finding.message);
        }
    }
    println!();

    println!(
        "FRAMES   {} frames, {:.1} fps, p50 {:.2}ms p95 {:.2}ms p99 {:.2}ms max {:.2}ms",
        frames.frames,
        frames.fps(),
        frames.p50_ms,
        frames.p95_ms,
        frames.p99_ms,
        frames.max_ms
    );
    println!(
        "         {} dropped, {} idle redraws, {:.2}ms refresh interval",
        frames.dropped_frames, report.summary.draws.idle_redraws, frames.refresh_interval_ms
    );

    if let Some(ms) = report.summary.time_to_first_content_ms {
        println!("CONTENT  first frame with commits at {ms:.0}ms from process start");
    }

    let draws = &report.summary.draws;
    if draws.draws > 0 {
        // Cadence says whether it was smooth here; this says how much of the
        // budget was left, which is the part that predicts a slower machine.
        println!(
            "DRAW     p50 {:.2}ms p95 {:.2}ms p99 {:.2}ms max {:.2}ms over {} draws",
            draws.draw_p50_ms, draws.draw_p95_ms, draws.draw_p99_ms, draws.draw_max_ms, draws.draws
        );
        match draws.cpu_slowdown_headroom() {
            Some(headroom) => println!(
                "         p95 uses {:.0}% of the frame budget — fits until a {:.1}x slower machine",
                draws.budget_used_p95_percent(),
                headroom
            ),
            None => println!("         no draw cost recorded"),
        }
        println!(
            "LATENCY  invalidation to drawn: p50 {:.2}ms p95 {:.2}ms max {:.2}ms",
            draws.dirty_to_draw_p50_ms, draws.dirty_to_draw_p95_ms, draws.dirty_to_draw_max_ms
        );
        println!(
            "         {:.0} invalidations coalesced into the median frame ({} total)",
            draws.invalidations_p50, draws.invalidations_total
        );
    }

    println!(
        "MEMORY   working set {:.1} MB (peak {:.1} MB), private {:.1} MB",
        bytes_to_mb(report.summary.final_process.working_set_bytes as i64),
        bytes_to_mb(report.summary.peak_working_set_bytes as i64),
        bytes_to_mb(report.summary.final_process.private_bytes as i64)
    );
    println!(
        "         census {:.1} MB, unaccounted {}",
        bytes_to_mb(report.summary.peak_census_bytes as i64),
        unaccounted_display(&report)
    );
    if let Some(cpu) = report.summary.cpu_percent_mean {
        println!("CPU      {cpu:.1}% of one core (mean)");
    }
    print_gpu(&report);
    println!(
        "TASKS    worst foreground stall {:.1}ms",
        report.summary.worst_foreground_task_ms
    );

    if !report.marks.is_empty() {
        println!("\nMARKS");
        for mark in &report.marks {
            println!(
                "  {:<28} {:>8.0}ms  p95 {:>6.2}ms  {:>4} dropped  {:+.1} MB",
                mark.name,
                mark.wall_ms,
                mark.frames.p95_ms,
                mark.frames.dropped_frames,
                bytes_to_mb(mark.working_set_delta_bytes)
            );
        }
    }

    if !report.actions.is_empty() {
        println!("\nSLOWEST ACTIONS (dispatch to presented frame)");
        for action in report.actions.iter().take(10) {
            println!(
                "  {:<34} n={:<5} p50 {:>6.2}ms  p95 {:>6.2}ms  max {:>7.2}ms",
                action.action, action.count, action.p50_ms, action.p95_ms, action.max_ms
            );
        }
    }

    Ok(())
}

/// GPU counters are the most likely to be missing, so the reason is printed
/// rather than the section silently vanishing.
/// The unaccounted figure, or why there is not one.
///
/// Printing a number here regardless would mean printing one derived from a
/// private-bytes counter that does not exist off Windows — a large negative
/// that reads as a finding rather than as a missing input.
fn unaccounted_display(report: &Report) -> String {
    match report.summary.unaccounted_bytes {
        Some(unaccounted) => format!("{:.1} MB", bytes_to_mb(unaccounted)),
        None => "unavailable (no private-bytes counter on this platform)".to_string(),
    }
}

fn print_gpu(report: &Report) {
    let gpu = &report.summary.gpu;
    if gpu.is_available() {
        let dedicated = gpu.dedicated_bytes.unwrap_or(0);
        let shared = gpu.shared_bytes.unwrap_or(0);
        print!(
            "GPU      {:.1} MB dedicated, {:.1} MB shared",
            bytes_to_mb(dedicated as i64),
            bytes_to_mb(shared as i64)
        );
        if let Some(budget) = gpu.budget_bytes {
            print!(" of {:.1} MB budget", bytes_to_mb(budget as i64));
        }
        if let Some(utilisation) = gpu.utilisation_percent {
            print!(", {utilisation:.1}% utilisation");
        }
        println!();
    } else {
        println!(
            "GPU      unavailable — {}",
            if gpu.unavailable.is_empty() {
                "no reason recorded".to_string()
            } else {
                gpu.unavailable.join("; ")
            }
        );
    }
}

fn top(args: &[String]) -> anyhow::Result<()> {
    let report = load_report(first_arg(args, "top")?)?;
    let count = limit(args, 20);

    if report.hot_locations.is_empty() {
        println!("no task timings recorded");
        return Ok(());
    }

    println!(
        "{:<44} {:<12} {:>6} {:>10} {:>9} {:>9}",
        "LOCATION", "THREAD", "N", "TOTAL", "P95", "MAX"
    );
    for row in report.hot_locations.iter().take(count) {
        println!(
            "{:<44} {:<12} {:>6} {:>9.1}ms {:>8.1}ms {:>8.1}ms",
            truncate(&format!("{}:{}", row.file, row.line), 44),
            if row.is_foreground {
                "FOREGROUND"
            } else {
                "background"
            },
            row.count,
            row.total_ms,
            row.p95_ms,
            row.max_ms
        );
    }
    Ok(())
}

fn heap(args: &[String]) -> anyhow::Result<()> {
    let report = load_report(first_arg(args, "heap")?)?;
    let count = limit(args, 20);

    if report.heap.is_empty() {
        println!("no census recorded");
        return Ok(());
    }

    println!("{:<56} {:>10} {:>10}", "PATH", "MB", "COUNT");
    for node in report.heap.iter().take(count) {
        println!(
            "{:<56} {:>10.2} {:>10}",
            truncate(&node.path, 56),
            bytes_to_mb(node.bytes as i64),
            node.count
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }
    println!(
        "\ncensus total {:.1} MB, process private {:.1} MB, unaccounted {}",
        bytes_to_mb(report.summary.peak_census_bytes as i64),
        bytes_to_mb(report.summary.final_process.private_bytes as i64),
        unaccounted_display(&report)
    );
    Ok(())
}

/// Ranks the allocation sites in a `dhat-heap.json`.
///
/// The raw file is megabytes of backtraces, one per distinct call path, which
/// is why this exists rather than pointing everyone at the viewer: the question
/// is nearly always "what is holding the memory", and that is a ranked list.
fn dhat(args: &[String]) -> anyhow::Result<()> {
    let path = args.first().map(String::as_str).unwrap_or("dhat-heap.json");
    let count = limit(args, 25);

    let source = std::fs::read_to_string(path)
        .map_err(|error| anyhow::anyhow!("cannot read {path}: {error}"))?;
    let sites = rgitui_perf::heap_profile::summarize_sites(&source, count)?;

    println!(
        "{:<74} {:>10} {:>8} {:>12}",
        "ALLOCATION SITE", "PEAK MB", "BLOCKS", "TOTAL MB"
    );
    for site in &sites {
        println!(
            "{:<74} {:>10.2} {:>8} {:>12.1}",
            truncate(&site.frame, 74),
            bytes_to_mb(site.bytes_at_peak as i64),
            site.blocks_at_peak,
            bytes_to_mb(site.total_bytes as i64)
        );
    }

    let peak: u64 = sites.iter().map(|site| site.bytes_at_peak).sum();
    println!(
        "\n{:.1} MB across the {} site(s) shown",
        bytes_to_mb(peak as i64),
        sites.len()
    );
    Ok(())
}

/// Exits non-zero when anything regressed, so this can gate a change.
fn run_compare(args: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.len() >= 2,
        "compare needs a baseline and a candidate report"
    );
    let baseline = load_report(&args[0])?;
    let candidate = load_report(&args[1])?;
    let comparison = compare(&baseline, &candidate, &NoiseFloor::default());

    print_comparison(&comparison);

    if comparison.has_regressions() {
        std::process::exit(1);
    }
    Ok(())
}

fn print_comparison(comparison: &Comparison) {
    if !comparison.caveats.is_empty() {
        println!("CAVEATS — read these before believing anything below:");
        for caveat in &comparison.caveats {
            println!("  ! {caveat}");
        }
        println!();
    }

    println!(
        "{} (baseline) vs {} (candidate)\n",
        comparison.baseline_scenario, comparison.candidate_scenario
    );
    println!(
        "{:<40} {:>14} {:>14} {:>10}",
        "METRIC", "BASELINE", "CANDIDATE", "CHANGE"
    );
    for metric in &comparison.metrics {
        let marker = match metric.direction {
            Direction::Regression => "WORSE",
            Direction::Improvement => "better",
            Direction::Unchanged => "",
        };
        let change = match metric.relative {
            Some(relative) => format!("{:+.1}%", relative * 100.0),
            None => "n/a".to_string(),
        };
        println!(
            "{:<40} {:>14.2} {:>14.2} {:>10} {}",
            truncate(&metric.metric, 40),
            metric.baseline,
            metric.candidate,
            change,
            marker
        );
    }

    if !comparison.new_findings.is_empty() {
        println!("\nNEW FINDINGS: {}", comparison.new_findings.join(", "));
    }
    if !comparison.resolved_findings.is_empty() {
        println!("RESOLVED:     {}", comparison.resolved_findings.join(", "));
    }

    println!(
        "\n{}",
        if comparison.has_regressions() {
            "REGRESSED"
        } else {
            "no regressions"
        }
    );
}

fn convert_trace(args: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.len() >= 2,
        "trace-to-scenario needs a trace.jsonl and an output path"
    );
    let source = std::fs::read_to_string(&args[0])
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", args[0]))?;

    let events: Vec<TraceEvent> = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|error| anyhow::anyhow!("cannot parse trace: {error}"))?;
    anyhow::ensure!(!events.is_empty(), "trace contains no events");

    let name = args.get(2).cloned().unwrap_or_else(|| {
        Path::new(&args[1])
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "recorded".to_string())
    });

    // Long idle gaps are capped: replaying the minute you spent reading a diff
    // would make the scenario a minute longer without measuring anything.
    let scenario = trace_to_scenario(&name, &events, 2_000);
    std::fs::write(&args[1], serde_json::to_string_pretty(&scenario)?)?;

    println!(
        "wrote {} ({} steps from {} recorded actions)",
        args[1],
        scenario.steps.len(),
        events.len()
    );
    Ok(())
}

/// Shortens a string to fit a column, marking that it was cut.
///
/// Counted in characters, not bytes: repository paths and profiler symbols
/// carry non-ASCII often enough, and a byte offset lands inside a code point
/// and panics — a report printer that crashes on somebody's directory name is
/// worse than a column that is a few cells wide.
fn truncate(text: &str, width: usize) -> String {
    let characters = text.chars().count();
    if characters <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    // Keep the tail: a path's filename and line number identify it far better
    // than the leading directories do.
    let tail: String = text.chars().skip(characters - (width - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_cuts_between_characters_not_inside_them() {
        // A repository under a non-ASCII path, which byte slicing panicked on.
        let path = "crates/rgitui_git/src/проект/refresh.rs:506";
        let short = truncate(path, 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.starts_with('…'));
        assert!(path.ends_with(short.trim_start_matches('…')));
    }

    #[test]
    fn truncate_handles_a_zero_width_column() {
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn truncate_keeps_the_identifying_tail_of_a_long_path() {
        let path = "crates/rgitui_git/src/project/refresh.rs:506";
        assert_eq!(truncate(path, 100), path);

        let short = truncate(path, 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.ends_with("refresh.rs:506"));
        assert!(short.starts_with('…'));
    }

    #[test]
    fn limit_falls_back_when_no_count_is_given() {
        assert_eq!(limit(&["report.json".to_string()], 20), 20);
        assert_eq!(limit(&["report.json".to_string(), "5".to_string()], 20), 5);
        assert_eq!(
            limit(&["report.json".to_string(), "nonsense".to_string()], 20),
            20
        );
    }
}
