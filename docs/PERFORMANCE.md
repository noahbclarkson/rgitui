# Measuring rgitui

rgitui's quality is largely a perceived-latency problem: whether the graph
scrolls smoothly, how long it takes from a keypress to pixels, and how much the
process grows as you open tabs and browse diffs. This document is how you get
numbers for those instead of impressions.

Everything lives in `crates/rgitui_perf`, is off by default, and is not compiled
into a normal build.

## The one-minute version

```bash
# Build the benchmark repositories once (cached under your OS cache dir).
cargo run -p rgitui_perf --bin rgitui-perf -- gen-corpus

# Run a scenario against one of them.
RGITUI_PERF=replay:crates/rgitui_perf/scenarios/graph-scroll.json \
  cargo run --profile perf --features perf -p rgitui -- \
  "$(cargo run -q -p rgitui_perf --bin rgitui-perf -- corpus-path medium)"

# Read what happened.
cargo run -p rgitui_perf --bin rgitui-perf -- summarize target/perf-runs/<run>/
```

On PowerShell, set the variable with `$env:RGITUI_PERF = "replay:..."` first.

## Build with `--profile perf`, not `--profile dev`

A debug build optimises so differently that its timings say nothing about the
shipped app. `[profile.perf]` inherits `release` and adds back the line tables
that profilers, dhat backtraces and Tracy need to name a frame. Every report
records which profile produced it, and `compare` refuses to quietly diff a debug
run against a release one.

## The two modes

**Replay** (`RGITUI_PERF=replay:<scenario.json>`) drives the app from a script
and writes its report when the last step finishes. Reproducible, so this is what
you compare across changes.

**Record** (`RGITUI_PERF=record`) instruments a session you drive by hand and
writes its report when you close the window. Use it when you have just felt
something be slow — then turn that session into a repeatable benchmark:

```bash
cargo run -p rgitui_perf --bin rgitui-perf -- \
  trace-to-scenario target/perf-runs/<run>/trace.jsonl my-scenario.json
```

Both modes go through the app's real input path. A replayed keystroke is
resolved by the same keymap as a real one, and a replayed command is built from
gpui's action registry and dispatched at the workspace root — there are no test
doubles anywhere in it.

## What a run produces

In `target/perf-runs/<timestamp>-<scenario>/`:

| File | What it is |
|---|---|
| `report.json` | The digest. Findings, summary, and the top rows of each table. Small enough to read in one pass — read this first. |
| `report.md` | The same thing for humans. |
| `samples.jsonl` | Raw counter series, one line per half-second. |
| `frames.jsonl` | Every frame interval. |
| `trace.jsonl` | Every dispatched command (record mode). |

`report.json` leads with `findings`: threshold checks evaluated in code, so the
file says *what is wrong* rather than leaving you to notice it. A run with no
findings is a run where nothing crossed a threshold.

## Reading a report

```bash
rgitui-perf summarize <run>     # findings, frames, memory, CPU, GPU, marks
rgitui-perf top <run> [N]       # worst task locations, foreground first
rgitui-perf heap <run> [N]      # largest census nodes
rgitui-perf compare <a> <b>     # what moved; exits 1 if anything regressed
```

`compare` holds every metric to a noise floor, because two identical runs never
produce identical numbers and a tool that cries wolf gets ignored. A change has
to be large **in proportion and in absolute terms** before it is called a
regression — the first version checked only the percentage, and two identical
`graph-scroll` runs promptly reported five regressions, among them a mark
spanning one keystroke that moved 8.5ms to 13.8ms (62% worse, and five
milliseconds) and two dropped frames out of 954 (33% worse).

Floors also differ by what is being compared: memory is held tightest, since the
same scenario over the same corpus allocates nearly the same bytes every run;
maxima are held loosest, since an extremum is a single sample. Every metric still
appears in the table whatever its verdict — only the regression call is gated.

Calibrate for your own machine by comparing two unchanged runs. It should report
no regressions; if it does, the floors in `NoiseFloor::default` are too tight for
your hardware.

## What each metric means, and what it does not

**Frame cadence** is the interval between vsync ticks, not the time spent inside
`draw`. Zed's own 120fps work found frame *durations* comfortably under budget
while frames were still being dropped — the cost of drawing and the rate of
presenting are different things, and only the second is what a user perceives.
The harness rides gpui's natural frame clock without forcing redraws, so it also
makes *idle redraws* visible: an app repainting while nothing changed is burning
battery for nothing. That count comes from gpui's own per-frame invalidation
record, so it means "drawn with nothing dirty" literally rather than inferring it
from whether a command was dispatched recently — which filed every hover and
animation tick as an idle repaint.

The refresh interval comes from the OS where it can be asked (`EnumDisplaySettingsW`
on Windows). Only when it cannot does the harness fall back to calibrating from
the run's own ticks, taking the most common interval; that fallback is a guess,
because an idle gpui window throttles its frame callbacks well below the panel's
rate. Every mark and the run summary are judged against the same interval — they
are all aggregated once, at the end, for exactly that reason.

**Task locations** come from gpui's own profiler, which tags every task with the
source location of the `spawn` that created it. Foreground rows are what matter:
time on the frame thread is time the frame clock spent waiting, so a hot
foreground location is a direct explanation for dropped frames.

**Memory** is reported two ways, and they answer different questions.

- The *census* walks rgitui's own structures and reports by feature —
  `tab[0]/caches/diff = 120 MB (200 entries)`. This is the one that tells you
  which bound to change. Shared `Arc`s are deduplicated by pointer, so a
  snapshot held by two views is charged once.
- *dhat* (`--features perf-dhat`) records a backtrace per allocation and catches
  what the census cannot see: gpui's element arena, libgit2's object cache,
  syntect's parser state. It costs roughly **100x runtime** — a scenario that
  opens a 100-commit repository, waits a second and quits takes 6 seconds
  normally and over nine minutes under dhat — so use it in its own run, and
  budget for it. The report marks any dhat run's timings as invalid rather than
  trusting you to remember.

  The cost tracks allocation *count*, and most allocations come from process
  startup and the frame loop rather than from the repository, so a smaller
  corpus barely helps. A dhat run that looks hung usually is not: check whether
  it is burning several cores, which is what the allocator lock looks like from
  outside.

The gap between the two — `unaccounted_bytes` — is worth understanding before
acting on. Measured on the medium corpus it is around 209 MB, while a dhat run
puts the whole Rust heap at a peak of 13.8 MB: almost none of that gap is heap
at all. It is the graphics driver, loaded DLLs and thread stacks. Treat a large
`unaccounted_bytes` as a leak only once a dhat run says the heap agrees.

**GPU** data is per-process VRAM (via DXGI) and engine utilisation (via the same
PDH counters Task Manager reads), sampled about once a second on Windows. That
is enough to catch "this change doubled our VRAM" or "the GPU is saturated while
scrolling". It is **not** enough to attribute cost to a draw call: per-frame GPU
timing would need D3D timestamp queries inside gpui's DirectX renderer, and gpui
is a pinned git dependency rgitui does not patch. When a counter is unavailable
the report says so with a reason rather than showing zero.

**Thread count** is sampled only at census points, not on the counter timer.
Windows offers no cheap way to count a process's threads — it means walking
every thread on the system, about 24 ms on a typical machine — which at 2 Hz
would have the harness burning 5% of a core inside the process it is measuring.

## Scenarios

In `crates/rgitui_perf/scenarios/`:

| Scenario | What it is for |
|---|---|
| `startup` | Cold start alone: repo open, first refresh, first paint. |
| `graph-scroll` | 500 single-row selection moves. The main smoothness benchmark. |
| `diff-browse` | Walking a diff and toggling side-by-side. |
| `panel-tour` | Opens every heavyweight view once, census after each. |
| `staging-churn` | Dirties the tree, then stage/unstage cycles — cache *invalidation*. |
| `scroll-storm` | Continuous wheel scrolling. The only scenario that draws long enough for draw p95/p99 to mean anything. |
| `key-repeat-degradation` | A held arrow key at 30Hz, timed in blocks of 100, to see whether block eight costs what block one did. |
| `idle` | Fifteen seconds of nothing. Any frame drawn here is one too many. |
| `startup-warm` | Second launch, with the caches the first one left. |
| `soak` | Repeated identical cycles; growth across them is a leak. |

A scenario is a JSON list of steps:

```json
{ "mark":     { "mark": "graph-scroll" } }
{ "action":   { "action": "graph::GraphSelectNext", "repeat": 500, "settle": "frame" } }
{ "key":      { "key": "ctrl-shift-p" } }
{ "scroll":   { "x": 400, "y": 300, "delta_y": -120, "repeat": 100 } }
{ "dirty":    { "count": 40 } }
{ "wait_for": { "condition": "operation_idle", "timeout_ms": 30000 } }
{ "snapshot": { "snapshot": "after-scroll" } }
{ "end_mark": { "mark": "graph-scroll" } }
```

Action names are the `namespace::Name` strings from
`crates/rgitui_workspace/src/keymap/registry.rs`. They are checked against the
registry before the first step runs, so a scenario naming a command that no
longer exists fails immediately instead of quietly measuring fewer steps than it
claims.

`"settle": "frame"` waits for the next presented frame before the next step.
`"settle": {"rate": {"hz": 30}}` instead delivers on a fixed clock whether or not
the app kept up — the only mode that reproduces a held key, because the backlog
that builds when the app falls behind is the whole phenomenon, and settling on a
frame is precisely the case where it cannot form.

Per-action latency does not come from either. An action is charged against the
first frame whose *invalidation* happened at or after the dispatch — the first
frame that could contain its effect — measured to the end of that draw. Closing
on the next tick regardless, which is what this used to do, measured the
remainder of the frame period and little else.

`mark`/`end_mark` bracket a named region and give it its own frame statistics
and memory delta. Marks are how you attribute a regression to a phase rather
than to a whole run.

Mouse steps address the window by coordinate, so they are reproducible at a
fixed window size but are **not** independent of layout: move a panel and a
recorded click lands somewhere else. Keystrokes and actions carry the weight of
a scenario for that reason; mouse steps exist mainly for scroll, which has no
keyboard equivalent exercising the same path.

## The corpus

Benchmarks that run against whatever repositories happen to be on the machine
produce numbers nobody else can reproduce. `gen-corpus` builds repositories from
a fixed recipe on a fixed clock, so the same tier is identical on every machine.

| Tier | Shape |
|---|---|
| `tiny` | 200 commits — smoke-tests the harness itself. |
| `small` | 2,000 commits, 20 branches. A personal project. |
| `medium` | 20,000 commits, 200 branches, 5,000 files. Use this for comparisons. |
| `pathological` | Wide merge fans, a 50,000-line single-file diff, CRLF and unicode paths. |
| `large` | 200,000 commits. Slow to build; only made when named explicitly. |

Recipes are fingerprinted into the directory name, so changing one regenerates
rather than silently reusing a repository built to the old shape.

A corpus is restored to its committed state each time it is handed to a run.
Scenarios are allowed to change the working tree — `staging-churn` has to, or it
stages nothing — and without this that would compound: the second run would
append to what the first appended to, and two runs of the same scenario would
stop measuring the same thing. Restoring on the way in rather than on the way
out also covers the run that died partway through. Nothing you leave in a corpus
directory survives, so treat it as the harness's, not yours.

## Interactive profiling with Tracy

`--features perf-tracy` routes gpui's own instrumentation — it already annotates
`draw`, `present` and `dispatch_event`, and emits a frame marker — to a
[Tracy](https://github.com/wolfpld/tracy) server. Use it when a report has told
you *which* scenario is slow and you want to see a specific frame's shape.

## A free first check

gpui itself will print draw-and-present duration per frame with no build changes
at all:

```bash
ZED_MEASUREMENTS=1 cargo run --profile perf -p rgitui -- <repo>
```

Coarse, and it measures draw cost rather than presented cadence, but it costs
nothing to try.

## Adding a type to the census

Implement `rgitui_perf::HeapSize` next to the type it describes, behind that
crate's `perf` feature — see `crates/rgitui_git/src/heap_size.rs`. Two rules
matter:

- Charge **capacity, not length**. A buffer that grew and shrank still holds its
  allocation, and hiding that hides a real cost.
- Route every `Arc` through `Census::visit_shared`. rgitui shares its large
  snapshots deliberately; counting one twice would overstate the heap by more
  than whatever you were investigating.

Then add it to the walk in `crates/rgitui_workspace/src/workspace/census.rs`.
Node labels become report paths and comparisons match on them, so renaming a
label reads as a node that vanished — pick names you can live with.
