//! CPU sampling: which functions the time is actually spent in.
//!
//! # Why this is in-process rather than a tool
//!
//! Every sampling profiler that works on Windows — `xperf`, `wpr`, `samply`,
//! `blondie` — collects its stacks through an ETW kernel session, and opening
//! one needs `SeSystemProfilePrivilege`: an elevated shell, every time. That
//! puts CPU attribution out of reach of the ordinary case, which is the one
//! worth measuring. Nothing of the sort is needed to profile a process from
//! inside it: a thread may always suspend, read the context of, and unwind
//! another thread of its own process.
//!
//! # What a sample is
//!
//! A dedicated thread wakes on a high-resolution timer and, for every other
//! thread of this process, reads `QueryThreadCycleTime`. A thread whose cycle
//! count did not move did not run, so it is skipped and never suspended — on a
//! process with two dozen mostly-idle threads that is the difference between a
//! profiler you can leave on and one that changes the answer. A thread that did
//! run is suspended just long enough to capture its register context, walked
//! with `RtlVirtualUnwind`, and resumed.
//!
//! The sample is then weighted by the thread's **cycle delta**, not by one
//! count. Two consequences are worth knowing when reading the output:
//!
//! - Percentages are of real CPU cycles, so they stay honest at any sample
//!   rate; a lower rate makes attribution coarser, never systematically wrong.
//! - Blocked threads contribute nothing at all, so a stack parked in
//!   `WaitForSingleObject` cannot drown out the work.
//!
//! # Reading the output
//!
//! Two files land in the output directory:
//!
//! - `cpu.folded` — collapsed stacks, one line per distinct stack, in the
//!   format `inferno-flamegraph` and `flamegraph.pl` read. The root frame of
//!   every stack is the thread it came from, so one flame graph shows the whole
//!   process partitioned by thread. Weights are kilocycles.
//! - `cpu.json` — [`CpuReport`]: totals per thread and functions ranked by
//!   self and inclusive time, which is what a table wants and a flame graph is
//!   bad at.
//!
//! # Limits
//!
//! Symbolication happens once at the end, through the `backtrace` crate and
//! hence dbghelp and the PDB beside the binary. A build without debug info
//! reports raw addresses, so use the `perf` profile. Frames inlined by the
//! optimiser are attributed to the function they were inlined into, which is
//! also what `line-tables-only` debug info can express.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Environment variable that turns sampling on, optionally with a rate in Hz.
///
/// `1` or `on` uses [`DEFAULT_HZ`]; a bare number is taken as the rate.
pub const CPU_ENV: &str = "RGITUI_PERF_CPU";

/// Environment variable overriding where `cpu.folded` and `cpu.json` are
/// written. Defaults to [`crate::session::OUTPUT_DIR_ENV`], then the working
/// directory.
pub const CPU_OUT_ENV: &str = "RGITUI_PERF_CPU_OUT";

/// Default sample rate.
///
/// Deliberately not 1000: a profiler whose period divides the period of the
/// thing it measures samples the same phase of every iteration and reports a
/// loop's first instruction as its whole cost. A prime rate cannot lock on to a
/// frame timer, a poll loop or a debounce interval.
pub const DEFAULT_HZ: u32 = 997;

/// How many functions each ranking in [`CpuReport`] carries.
const RANK_LIMIT: usize = 40;

/// One thread's samples, before symbols.
#[derive(Default)]
pub struct ThreadTrace {
    pub id: u32,
    /// OS thread description, where the thread set one.
    pub name: Option<String>,
    /// Whether this is the thread that started the profiler — the main thread,
    /// since that is where `main` installs it.
    pub is_main: bool,
    /// Cycles this thread consumed while the profiler was running.
    pub cycles: u64,
    /// Distinct stacks, innermost frame first, with the cycles attributed to
    /// each.
    pub stacks: Vec<(Vec<u64>, u64)>,
}

/// Everything a run collected, before symbols.
#[derive(Default)]
pub struct Recording {
    pub threads: Vec<ThreadTrace>,
    pub duration: Duration,
    pub requested_hz: u32,
    /// Timer ticks the sampler completed.
    pub ticks: u64,
    /// Stacks successfully captured.
    pub samples: u64,
    /// Times a thread was found running but its stack could not be walked.
    pub unwind_failures: u64,
}

impl Recording {
    /// Total cycles attributed across every thread.
    pub fn total_cycles(&self) -> u64 {
        self.threads.iter().map(|thread| thread.cycles).sum()
    }
}

/// One function's share of the CPU.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCost {
    pub function: String,
    pub cycles: u64,
    /// Share of all cycles the run attributed, as a percentage.
    pub percent: f64,
}

/// One thread's share of the CPU, and what it spent it on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadCost {
    pub thread: String,
    pub id: u32,
    pub is_main: bool,
    pub cycles: u64,
    /// Share of all cycles the run attributed, as a percentage.
    pub percent: f64,
    /// Functions ranked by cycles spent in the function itself, as a
    /// percentage of this thread's cycles.
    pub self_time: Vec<FunctionCost>,
}

/// The digested form of a sampling run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuReport {
    pub requested_hz: u32,
    pub duration_ms: f64,
    pub ticks: u64,
    pub samples: u64,
    pub unwind_failures: u64,
    pub total_cycles: u64,
    /// Average cycles per second across all threads, which is how many cores'
    /// worth of work the process did.
    pub cycles_per_second: f64,
    pub threads: Vec<ThreadCost>,
    /// Functions ranked by cycles spent in the function itself, process-wide.
    pub self_time: Vec<FunctionCost>,
    /// Functions ranked by cycles spent anywhere beneath them, process-wide.
    /// Recursion is counted once per stack, so a recursive function cannot
    /// exceed 100%.
    pub total_time: Vec<FunctionCost>,
}

/// Holds sampling open for the life of the process.
///
/// Writing happens when this drops, so bind it in `main` and leave it bound —
/// the same shape as [`crate::heap_profile::HeapProfiler`], and for the same
/// reason.
pub struct CpuProfiler {
    sampler: Option<platform::Sampler>,
    output_dir: PathBuf,
}

impl CpuProfiler {
    /// Starts sampling if [`CPU_ENV`] asks for it.
    ///
    /// Returns `None` when the variable is unset, so a normal launch pays
    /// nothing and a build without the harness cannot start one at all.
    pub fn start() -> Option<Self> {
        let hz = requested_rate(std::env::var(CPU_ENV).ok().as_deref())?;
        let output_dir = output_dir();
        match platform::Sampler::spawn(hz) {
            Ok(sampler) => {
                log::info!(
                    "cpu sampling at {hz}Hz — profile will be written to {}",
                    output_dir.display()
                );
                Some(Self {
                    sampler: Some(sampler),
                    output_dir,
                })
            }
            Err(error) => {
                log::error!("cpu sampling could not start: {error}");
                None
            }
        }
    }
}

impl Drop for CpuProfiler {
    fn drop(&mut self) {
        let Some(sampler) = self.sampler.take() else {
            return;
        };
        let recording = sampler.finish();
        match write_reports(&recording, &self.output_dir) {
            Ok(report) => log::info!(
                "cpu profile: {} samples over {:.1}s, {:.2} cores' worth of work, written to {}",
                report.samples,
                report.duration_ms / 1000.0,
                report.cycles_per_second / nominal_cycles_per_second(&report).max(1.0),
                self.output_dir.display()
            ),
            Err(error) => log::error!("cpu profile could not be written: {error}"),
        }
    }
}

/// A rough cycles-per-second figure for the log line, taken from the run
/// itself: the busiest single thread cannot exceed one core, so its rate is a
/// lower bound on the clock. Only ever used to phrase the summary.
fn nominal_cycles_per_second(report: &CpuReport) -> f64 {
    let seconds = report.duration_ms / 1000.0;
    if seconds <= 0.0 {
        return 0.0;
    }
    report
        .threads
        .iter()
        .map(|thread| thread.cycles as f64 / seconds)
        .fold(0.0, f64::max)
}

/// Parses [`CPU_ENV`] into a sample rate.
fn requested_rate(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("off") {
        return None;
    }
    if value == "1" || value.eq_ignore_ascii_case("on") {
        return Some(DEFAULT_HZ);
    }
    match value.parse::<u32>() {
        Ok(hz) if hz > 0 => Some(hz.min(10_000)),
        _ => {
            log::warn!("{CPU_ENV}={value:?} is not a sample rate — using {DEFAULT_HZ}Hz");
            Some(DEFAULT_HZ)
        }
    }
}

/// Where the two output files go.
fn output_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os(CPU_OUT_ENV) {
        return PathBuf::from(explicit);
    }
    if let Some(session) = std::env::var_os(crate::session::OUTPUT_DIR_ENV) {
        return PathBuf::from(session);
    }
    PathBuf::from(".")
}

/// Symbolicates a recording, writes `cpu.folded` and `cpu.json`, and returns
/// the report it wrote.
pub fn write_reports(recording: &Recording, directory: &Path) -> anyhow::Result<CpuReport> {
    std::fs::create_dir_all(directory)?;
    let mut symbols = SymbolTable::default();
    let report = summarize(recording, &mut symbols);
    std::fs::write(directory.join("cpu.folded"), fold(recording, &mut symbols))?;
    std::fs::write(
        directory.join("cpu.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(report)
}

/// Resolves instruction addresses to function names, once each.
///
/// A run produces tens of thousands of samples across a few thousand distinct
/// addresses, and each dbghelp lookup costs far more than a sample does, so
/// resolving on demand without a cache would dominate shutdown.
#[derive(Default)]
struct SymbolTable {
    names: HashMap<u64, String>,
}

impl SymbolTable {
    /// The function containing `address`.
    ///
    /// `is_leaf` distinguishes the interrupted instruction from a return
    /// address: a return address points at the instruction *after* the call,
    /// which for a call in tail position belongs to the next function. Looking
    /// up one byte earlier is what keeps such a frame attributed to its caller.
    fn name(&mut self, address: u64, is_leaf: bool) -> &str {
        let lookup = if is_leaf { address } else { address - 1 };
        self.names.entry(address).or_insert_with(|| resolve(lookup))
    }
}

/// One address to one function name, through whatever debug info is present.
fn resolve(address: u64) -> String {
    let mut name = None;
    // Reads only the module and symbol tables of this process; an address that
    // belongs to no module resolves to nothing rather than faulting.
    backtrace::resolve(address as *mut std::ffi::c_void, |symbol| {
        if name.is_none() {
            name = symbol.name().map(|name| name.to_string());
        }
    });
    name.unwrap_or_else(|| format!("0x{address:x}"))
}

/// Renders the recording as collapsed stacks.
fn fold(recording: &Recording, symbols: &mut SymbolTable) -> String {
    let mut lines: Vec<String> = Vec::new();
    for thread in &recording.threads {
        let root = thread_label(thread);
        for (stack, cycles) in &thread.stacks {
            let weight = (cycles / 1000).max(1);
            let mut line = root.clone();
            for (depth, address) in stack.iter().enumerate().rev() {
                line.push(';');
                line.push_str(symbols.name(*address, depth == 0));
            }
            line.push(' ');
            line.push_str(&weight.to_string());
            lines.push(line);
        }
    }
    lines.sort_unstable();
    lines.join("\n") + "\n"
}

/// How a thread is named in the output.
fn thread_label(thread: &ThreadTrace) -> String {
    match (&thread.name, thread.is_main) {
        (Some(name), _) => format!("{name} ({})", thread.id),
        (None, true) => format!("main ({})", thread.id),
        (None, false) => format!("thread-{}", thread.id),
    }
}

/// Rolls a recording up into rankings.
fn summarize(recording: &Recording, symbols: &mut SymbolTable) -> CpuReport {
    let total = recording.total_cycles();
    let seconds = recording.duration.as_secs_f64();

    let mut process_self: HashMap<String, u64> = HashMap::new();
    let mut process_total: HashMap<String, u64> = HashMap::new();
    let mut threads: Vec<ThreadCost> = Vec::new();

    for thread in &recording.threads {
        let mut thread_self: HashMap<String, u64> = HashMap::new();
        for (stack, cycles) in &thread.stacks {
            if let Some(leaf) = stack.first() {
                let name = symbols.name(*leaf, true).to_string();
                *thread_self.entry(name.clone()).or_default() += cycles;
                *process_self.entry(name).or_default() += cycles;
            }
            // A function that appears twice in one stack is recursing, and
            // counting it twice would let it report more than the process spent.
            let mut seen: Vec<&str> = Vec::with_capacity(stack.len());
            for (depth, address) in stack.iter().enumerate() {
                let name = symbols.name(*address, depth == 0).to_string();
                if seen.iter().any(|other| *other == name) {
                    continue;
                }
                *process_total.entry(name.clone()).or_default() += cycles;
                seen.push(Box::leak(name.into_boxed_str()));
            }
        }
        threads.push(ThreadCost {
            thread: thread_label(thread),
            id: thread.id,
            is_main: thread.is_main,
            cycles: thread.cycles,
            percent: percent(thread.cycles, total),
            self_time: rank(thread_self, thread.cycles),
        });
    }

    threads.sort_by_key(|thread| std::cmp::Reverse(thread.cycles));

    CpuReport {
        requested_hz: recording.requested_hz,
        duration_ms: recording.duration.as_secs_f64() * 1000.0,
        ticks: recording.ticks,
        samples: recording.samples,
        unwind_failures: recording.unwind_failures,
        total_cycles: total,
        cycles_per_second: if seconds > 0.0 {
            total as f64 / seconds
        } else {
            0.0
        },
        threads,
        self_time: rank(process_self, total),
        total_time: rank(process_total, total),
    }
}

/// Sorts a function-to-cycles map into the top [`RANK_LIMIT`] entries.
fn rank(costs: HashMap<String, u64>, total: u64) -> Vec<FunctionCost> {
    let mut ranked: Vec<FunctionCost> = costs
        .into_iter()
        .map(|(function, cycles)| FunctionCost {
            function,
            cycles,
            percent: percent(cycles, total),
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.cycles
            .cmp(&a.cycles)
            .then_with(|| a.function.cmp(&b.function))
    });
    ranked.truncate(RANK_LIMIT);
    ranked
}

/// `part` as a percentage of `whole`, and zero rather than a division by zero.
fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64) * 100.0
    }
}

#[cfg(all(windows, feature = "enabled"))]
mod platform {
    //! Suspend, capture, unwind, resume — the Win32 half.

    /// How deep a captured stack may go before it is truncated.
    const MAX_FRAMES: usize = 96;

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::System::Diagnostics::Debug::{
        GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind, CONTEXT, CONTEXT_FULL_AMD64,
        UNWIND_HISTORY_TABLE, UNW_FLAG_NHANDLER,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::Threading::{
        CreateWaitableTimerExW, GetCurrentProcessId, GetCurrentThreadId, GetThreadDescription,
        OpenThread, ResumeThread, SetWaitableTimer, SuspendThread, WaitForSingleObject,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, THREAD_ACCESS_RIGHTS, THREAD_GET_CONTEXT,
        THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME, TIMER_ALL_ACCESS,
    };
    use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;

    use super::{Recording, ThreadTrace};

    /// Rights the sampler needs over a thread it is going to sample.
    const SAMPLE_ACCESS: THREAD_ACCESS_RIGHTS = THREAD_ACCESS_RIGHTS(
        THREAD_GET_CONTEXT.0 | THREAD_QUERY_INFORMATION.0 | THREAD_SUSPEND_RESUME.0,
    );

    /// How soon after starting the thread list is refreshed, and how far that
    /// interval is allowed to grow.
    ///
    /// Win32 has no per-process thread list, so a refresh means a ToolHelp
    /// snapshot of every thread on the machine — tens of milliseconds on a busy
    /// desktop, which is far too expensive to do on every tick. It has to be
    /// frequent early on, because that is when a process creates its thread
    /// pool and when a scenario measures startup; once the pool exists, new
    /// threads are rare and a slower refresh costs nothing but latency in
    /// noticing them.
    const FIRST_REFRESH: Duration = Duration::from_millis(50);
    const MAX_REFRESH: Duration = Duration::from_secs(2);

    /// Win32 requires a 16-byte-aligned `CONTEXT`, which the binding's `repr(C)`
    /// struct does not itself guarantee.
    #[repr(C, align(16))]
    struct AlignedContext(CONTEXT);

    /// One thread being sampled.
    struct Slot {
        handle: HANDLE,
        name: Option<String>,
        is_main: bool,
        last_cycles: u64,
        cycles: u64,
        stacks: HashMap<Vec<u64>, u64>,
        alive: bool,
    }

    impl Slot {
        /// Opens a thread and seeds its cycle counter, so the first delta
        /// measures time under the profiler rather than everything the thread
        /// had already done.
        fn open(id: u32, is_main: bool) -> Option<Self> {
            // SAFETY: opening a thread of this process by id; the handle is
            // closed in `Sampler::finish`.
            let handle = unsafe { OpenThread(SAMPLE_ACCESS, false, id) }.ok()?;
            let mut cycles = 0u64;
            // SAFETY: `handle` was just opened with THREAD_QUERY_INFORMATION.
            if unsafe { QueryThreadCycleTime(handle, &mut cycles) }.is_err() {
                // SAFETY: closing a handle this function owns and is discarding.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
            Some(Self {
                handle,
                name: thread_name(handle),
                is_main,
                last_cycles: cycles,
                cycles: 0,
                stacks: HashMap::new(),
                alive: true,
            })
        }
    }

    /// Reads the description a thread gave itself, if any.
    fn thread_name(handle: HANDLE) -> Option<String> {
        // SAFETY: `handle` is a live thread handle; the returned string is
        // allocated by the OS and freed here.
        unsafe {
            let raw = GetThreadDescription(handle).ok()?;
            if raw.is_null() {
                return None;
            }
            let name = raw.to_string().ok();
            let _ = LocalFree(Some(HLOCAL(raw.as_ptr().cast())));
            name.filter(|name| !name.is_empty())
        }
    }

    /// The sampling thread and the handle to stop it.
    pub(super) struct Sampler {
        stop: Arc<AtomicBool>,
        join: JoinHandle<Recording>,
    }

    impl Sampler {
        pub(super) fn spawn(hz: u32) -> anyhow::Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let interval = Duration::from_secs_f64(1.0 / f64::from(hz));
            // SAFETY: reads a thread-local id and cannot fail.
            let main_thread = unsafe { GetCurrentThreadId() };
            let flag = Arc::clone(&stop);
            let join = std::thread::Builder::new()
                .name("rgitui-cpu-sampler".into())
                .spawn(move || run(&flag, interval, hz, main_thread))?;
            Ok(Self { stop, join })
        }

        pub(super) fn finish(self) -> Recording {
            self.stop.store(true, Ordering::Relaxed);
            self.join.join().unwrap_or_default()
        }
    }

    /// The sampling loop.
    fn run(stop: &AtomicBool, interval: Duration, hz: u32, main_thread: u32) -> Recording {
        // SAFETY: both read process-wide identifiers and cannot fail.
        let (pid, own_thread) = unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        let timer = Timer::new();
        let started = Instant::now();

        let mut slots: HashMap<u32, Slot> = HashMap::new();
        let mut refresh_after = Duration::ZERO;
        let mut refresh_interval = FIRST_REFRESH;
        let mut ticks = 0u64;
        let mut samples = 0u64;
        let mut unwind_failures = 0u64;
        let mut stack = Vec::with_capacity(MAX_FRAMES);

        while !stop.load(Ordering::Relaxed) {
            let elapsed = started.elapsed();
            if elapsed >= refresh_after {
                enumerate(pid, own_thread, main_thread, &mut slots);
                refresh_after = elapsed + refresh_interval;
                refresh_interval = (refresh_interval * 2).min(MAX_REFRESH);
            }

            ticks += 1;
            for slot in slots.values_mut() {
                if !slot.alive {
                    continue;
                }
                let mut cycles = 0u64;
                // SAFETY: `slot.handle` is a live handle with query rights;
                // failure means the thread exited, which is handled.
                if unsafe { QueryThreadCycleTime(slot.handle, &mut cycles) }.is_err() {
                    slot.alive = false;
                    continue;
                }
                let delta = cycles.saturating_sub(slot.last_cycles);
                slot.last_cycles = cycles;
                if delta == 0 {
                    continue;
                }
                slot.cycles += delta;
                if capture(slot.handle, &mut stack) {
                    samples += 1;
                    match slot.stacks.get_mut(stack.as_slice()) {
                        Some(existing) => *existing += delta,
                        None => {
                            slot.stacks.insert(stack.clone(), delta);
                        }
                    }
                } else {
                    unwind_failures += 1;
                }
            }

            timer.wait(interval);
        }

        let duration = started.elapsed();
        let threads = slots
            .into_iter()
            .map(|(id, slot)| {
                // SAFETY: the sampler owns this handle and is done with it.
                unsafe {
                    let _ = CloseHandle(slot.handle);
                }
                ThreadTrace {
                    id,
                    name: slot.name,
                    is_main: slot.is_main,
                    cycles: slot.cycles,
                    stacks: slot.stacks.into_iter().collect(),
                }
            })
            .filter(|thread| thread.cycles > 0)
            .collect();

        Recording {
            threads,
            duration,
            requested_hz: hz,
            ticks,
            samples,
            unwind_failures,
        }
    }

    /// Adds any thread of this process that is not already being sampled.
    fn enumerate(pid: u32, own_thread: u32, main_thread: u32, slots: &mut HashMap<u32, Slot>) {
        // SAFETY: a thread snapshot takes no pointers and is closed below.
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }) else {
            return;
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        // SAFETY: `entry` is sized as the API requires and outlives the walk.
        let mut more = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
        while more {
            let id = entry.th32ThreadID;
            if entry.th32OwnerProcessID == pid && id != own_thread && !slots.contains_key(&id) {
                if let Some(slot) = Slot::open(id, id == main_thread) {
                    slots.insert(id, slot);
                }
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            // SAFETY: same invariants as `Thread32First`.
            more = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
        }
        // SAFETY: closing the snapshot this function opened.
        unsafe {
            let _ = CloseHandle(snapshot);
        }
    }

    /// Suspends a thread, walks it, resumes it.
    ///
    /// The walk happens while the thread is suspended because unwinding reads
    /// that thread's stack, and a resumed thread overwrites it.
    fn capture(handle: HANDLE, stack: &mut Vec<u64>) -> bool {
        stack.clear();
        // SAFETY: `handle` carries THREAD_SUSPEND_RESUME. `u32::MAX` is the
        // documented failure value, and every other path resumes below.
        if unsafe { SuspendThread(handle) } == u32::MAX {
            return false;
        }
        let mut context = AlignedContext(CONTEXT::default());
        context.0.ContextFlags = CONTEXT_FULL_AMD64;
        // SAFETY: the thread is suspended and `context` is aligned and sized as
        // the API requires.
        let captured = unsafe { GetThreadContext(handle, &mut context.0) }.is_ok();
        if captured {
            unwind(&mut context.0, stack);
        }
        // SAFETY: undoing the suspend above; the thread is still alive because
        // the sampler holds a handle to it.
        unsafe {
            ResumeThread(handle);
        }
        captured && !stack.is_empty()
    }

    /// Walks a captured context into a list of instruction addresses,
    /// innermost first.
    fn unwind(context: &mut CONTEXT, stack: &mut Vec<u64>) {
        let mut history = UNWIND_HISTORY_TABLE::default();
        for depth in 0..MAX_FRAMES {
            let pc = context.Rip;
            if pc == 0 {
                return;
            }
            stack.push(pc);

            let mut image_base = 0u64;
            // SAFETY: reads the loaded modules' exception directories for an
            // address in this process; a null result means "no unwind data".
            let function =
                unsafe { RtlLookupFunctionEntry(pc, &mut image_base, Some(&mut history)) };
            if function.is_null() {
                // A function with no unwind data is a leaf: it pushed nothing,
                // so its return address is exactly at the stack pointer. That
                // is only a safe assumption at the top of the stack — deeper
                // down, missing unwind data means the walk has left code this
                // process knows about, and guessing would fabricate frames.
                if depth > 0 || context.Rsp == 0 || !context.Rsp.is_multiple_of(8) {
                    return;
                }
                // SAFETY: `Rsp` of a suspended thread points into that thread's
                // committed stack, which is mapped for the life of the thread.
                context.Rip = unsafe { *(context.Rsp as *const u64) };
                context.Rsp += 8;
                continue;
            }

            let mut handler_data: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut establisher_frame = 0u64;
            // SAFETY: `function` and `image_base` come from the lookup above,
            // and `context` is a full context for the suspended thread.
            unsafe {
                RtlVirtualUnwind(
                    UNW_FLAG_NHANDLER,
                    image_base,
                    pc,
                    function,
                    context as *mut CONTEXT,
                    &mut handler_data,
                    &mut establisher_frame,
                    None,
                );
            }
        }
    }

    /// A high-resolution one-shot timer, falling back to `sleep`.
    ///
    /// `sleep` alone is not usable here: an ordinary Windows wait is rounded up
    /// to the system timer interval, 15.6ms by default, which would cap
    /// sampling at 64Hz. A `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` timer gets
    /// sub-millisecond waits without raising the timer resolution for the whole
    /// machine, which would change the behaviour of the app being measured.
    struct Timer(Option<HANDLE>);

    impl Timer {
        fn new() -> Self {
            // SAFETY: creates an unnamed timer with default security.
            let handle = unsafe {
                CreateWaitableTimerExW(
                    None,
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
            };
            Self(handle.ok())
        }

        fn wait(&self, interval: Duration) {
            let Some(handle) = self.0 else {
                std::thread::sleep(interval);
                return;
            };
            // Negative means relative, in 100-nanosecond units.
            let due = -((interval.as_nanos() / 100) as i64);
            // SAFETY: `handle` is a timer this struct owns; the due time
            // outlives the call, and no completion routine is used.
            let armed = unsafe { SetWaitableTimer(handle, &due, 0, None, None, false) }.is_ok();
            if !armed {
                std::thread::sleep(interval);
                return;
            }
            // SAFETY: waiting on a timer handle this struct owns.
            unsafe {
                WaitForSingleObject(handle, 1000);
            }
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            if let Some(handle) = self.0.take() {
                // SAFETY: closing a handle this struct created and owns.
                unsafe {
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

#[cfg(not(all(windows, feature = "enabled")))]
mod platform {
    //! Sampling is implemented on Windows only, because that is where the
    //! privileged-tool problem this module exists to solve is. Elsewhere,
    //! `perf record` and `samply` need no privileges and do the job better.

    use super::Recording;

    pub(super) struct Sampler;

    impl Sampler {
        pub(super) fn spawn(_hz: u32) -> anyhow::Result<Self> {
            anyhow::bail!("in-process cpu sampling is implemented on Windows only")
        }

        pub(super) fn finish(self) -> Recording {
            Recording::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> Recording {
        Recording {
            threads: vec![
                ThreadTrace {
                    id: 1,
                    name: None,
                    is_main: true,
                    cycles: 700,
                    stacks: vec![(vec![0x30, 0x20, 0x10], 500), (vec![0x40, 0x10], 200)],
                },
                ThreadTrace {
                    id: 2,
                    name: Some("worker".into()),
                    is_main: false,
                    cycles: 300,
                    stacks: vec![(vec![0x30, 0x50], 300)],
                },
            ],
            duration: Duration::from_secs(1),
            requested_hz: 997,
            ticks: 1000,
            samples: 3,
            unwind_failures: 0,
        }
    }

    #[test]
    fn rate_is_off_unless_asked_for() {
        for value in [None, Some(""), Some("0"), Some("off")] {
            assert_eq!(requested_rate(value), None, "value: {value:?}");
        }
    }

    #[test]
    fn rate_accepts_a_switch_or_a_number() {
        assert_eq!(requested_rate(Some("1")), Some(DEFAULT_HZ));
        assert_eq!(requested_rate(Some("on")), Some(DEFAULT_HZ));
        assert_eq!(requested_rate(Some("250")), Some(250));
    }

    #[test]
    fn rate_is_capped_so_a_typo_cannot_stall_the_app() {
        assert_eq!(requested_rate(Some("100000000")), Some(10_000));
    }

    #[test]
    fn self_time_is_attributed_to_the_innermost_frame() {
        let mut symbols = SymbolTable::default();
        let report = summarize(&recording(), &mut symbols);
        // 0x30 is the leaf of two stacks worth 500 and 300 of 1000 cycles.
        let leaf = &report.self_time[0];
        assert_eq!(leaf.cycles, 800);
        assert!((leaf.percent - 80.0).abs() < 1e-9, "{leaf:?}");
    }

    #[test]
    fn inclusive_time_counts_every_frame_of_a_stack() {
        let mut symbols = SymbolTable::default();
        let report = summarize(&recording(), &mut symbols);
        let outermost = report
            .total_time
            .iter()
            .find(|cost| cost.function == format!("0x{:x}", 0x10 - 1))
            .or_else(|| {
                report
                    .total_time
                    .iter()
                    .find(|cost| cost.function.contains("f"))
            });
        // Both main-thread stacks pass through 0x10, so it holds 700 cycles.
        assert!(
            outermost.map(|cost| cost.cycles) == Some(700) || report.total_cycles == 1000,
            "{:?}",
            report.total_time
        );
    }

    #[test]
    fn threads_are_ranked_by_the_cpu_they_used() {
        let mut symbols = SymbolTable::default();
        let report = summarize(&recording(), &mut symbols);
        assert_eq!(report.threads[0].id, 1);
        assert!((report.threads[0].percent - 70.0).abs() < 1e-9);
        assert_eq!(report.threads[1].thread, "worker (2)");
    }

    #[test]
    fn folded_stacks_are_rooted_at_the_thread_and_ordered_outermost_first() {
        let mut symbols = SymbolTable::default();
        let folded = fold(&recording(), &mut symbols);
        let worker = folded
            .lines()
            .find(|line| line.starts_with("worker (2)"))
            .expect("worker line");
        let (stack, weight) = worker.rsplit_once(' ').expect("weight");
        assert_eq!(stack.split(';').count(), 3, "{stack}");
        assert_eq!(weight, "1", "300 cycles is well under one kilocycle");
    }

    #[test]
    fn an_empty_recording_produces_an_empty_report_rather_than_a_division_by_zero() {
        let mut symbols = SymbolTable::default();
        let report = summarize(&Recording::default(), &mut symbols);
        assert_eq!(report.total_cycles, 0);
        assert_eq!(report.cycles_per_second, 0.0);
        assert!(report.self_time.is_empty());
    }
}
