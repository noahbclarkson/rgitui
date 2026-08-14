//! Whole-process OS counters: memory footprint, CPU time, handle and thread counts.
//!
//! These are the numbers a user would see in Task Manager. They bound the
//! semantic heap census in [`crate::heap`]: anything the census cannot account
//! for lives in allocator slack, the GPU driver, thread stacks, or a leak.

use serde::{Deserialize, Serialize};

/// One reading of the process's OS-level counters.
///
/// Memory fields are bytes. `None` means the platform did not report that
/// counter rather than that it was zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSample {
    /// Physical memory currently mapped in — Task Manager's "Memory (active)".
    pub working_set_bytes: u64,
    /// High-water mark of `working_set_bytes` for the process lifetime.
    pub peak_working_set_bytes: u64,
    /// Committed private bytes: memory this process cannot share with another.
    /// The most honest single number for "how much does rgitui cost".
    pub private_bytes: u64,
    /// Cumulative user-mode CPU time in microseconds since process start.
    pub cpu_user_micros: u64,
    /// Cumulative kernel-mode CPU time in microseconds since process start.
    pub cpu_kernel_micros: u64,
    /// Open OS handle count, if the platform reports it. A steadily climbing
    /// handle count is a leak that memory counters alone would not reveal.
    pub handle_count: Option<u32>,
    /// Live OS thread count.
    pub thread_count: Option<u32>,
}

impl ProcessSample {
    /// Total CPU microseconds consumed, user plus kernel.
    pub fn cpu_total_micros(&self) -> u64 {
        self.cpu_user_micros.saturating_add(self.cpu_kernel_micros)
    }

    /// Average CPU utilisation between two samples, as a percentage of one core.
    ///
    /// Returns `None` when the samples are not ordered or no wall time passed,
    /// which would otherwise divide by zero and report a meaningless spike.
    pub fn cpu_percent_between(earlier: &Self, later: &Self, wall_micros: u64) -> Option<f64> {
        if wall_micros == 0 {
            return None;
        }
        let cpu = later
            .cpu_total_micros()
            .checked_sub(earlier.cpu_total_micros())?;
        Some((cpu as f64 / wall_micros as f64) * 100.0)
    }
}

/// Reads [`ProcessSample`]s for the current process.
///
/// Holds any platform handles the counters need so that sampling on a timer
/// does not re-open them every tick.
pub struct ProcessSampler {
    inner: platform::Inner,
}

impl ProcessSampler {
    /// Opens the platform counters. Failure is reported rather than swallowed:
    /// a run that silently lost its memory numbers is worse than one that
    /// refuses to start.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            inner: platform::Inner::new()?,
        })
    }

    /// Takes one reading.
    pub fn sample(&mut self) -> anyhow::Result<ProcessSample> {
        self.inner.sample()
    }

    /// Live OS thread count.
    ///
    /// Separate from [`ProcessSampler::sample`] because obtaining it means
    /// walking every thread on the system — several thousand on a typical
    /// machine, ~24 ms — which is far too expensive to sit in a twice-a-second
    /// sampler inside the process being measured.
    ///
    /// Measured on a desktop carrying ~5,500 live threads: ~24ms per call, of
    /// which ~4ms is `CreateToolhelp32Snapshot` and ~20ms is the
    /// `Thread32First`/`Thread32Next` walk, about 4µs for each thread on the
    /// system. Release and debug measure the same because it is all kernel
    /// time, and the cost scales with the machine's thread count rather than
    /// this process's, so a loaded machine is worse.
    ///
    /// Call it where the harness already stops — a heap census, a mark
    /// boundary, the end of a run — and never on the frame path, where a 24ms
    /// stall would land on an arbitrary tick and be reported as a frame-time
    /// outlier against the app.
    ///
    /// [`ProcessSampler::sample`] leaves [`ProcessSample::thread_count`] as
    /// `None`; this is what fills it.
    pub fn sample_thread_count(&mut self) -> Option<u32> {
        self.inner.thread_count()
    }
}

#[cfg(windows)]
mod platform {
    //! Win32 counters. `GetProcessMemoryInfo` reports the working set and, via
    //! `PROCESS_MEMORY_COUNTERS_EX`, the private commit that Task Manager shows
    //! as "Commit size"; `GetProcessTimes` reports CPU as 100-nanosecond ticks.
    //!
    //! Thread count is far and away the most expensive counter here, and the
    //! cost is not this code's to optimise away. Win32 exposes no per-process
    //! thread count, so it comes from a ToolHelp snapshot of every thread on
    //! the system, filtered down to the ones this process owns. Measured on a
    //! desktop carrying ~5,500 live threads that is ~24ms per reading — ~4ms
    //! inside `CreateToolhelp32Snapshot` and ~20ms walking the snapshot, the
    //! same in release as in debug because it is all kernel time — against
    //! well under a microsecond for every other counter in this module.
    //!
    //! That is why it is not part of a sample: it is reached through
    //! [`super::ProcessSampler::sample_thread_count`] instead, for the caller
    //! to read at the few points in a run where the harness has already
    //! stopped.

    use super::ProcessSample;
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, FILETIME, HANDLE,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessHandleCount, GetProcessTimes,
    };

    pub(super) struct Inner {
        /// The current-process pseudo-handle. It is a constant rather than a
        /// real handle, so it is never closed and stays valid for the lifetime
        /// of the process.
        process: HANDLE,
    }

    impl Inner {
        pub(super) fn new() -> anyhow::Result<Self> {
            // SAFETY: GetCurrentProcess reads and writes nothing, takes no
            // arguments and cannot fail; it returns a constant pseudo-handle.
            let process = unsafe { GetCurrentProcess() };
            Ok(Self { process })
        }

        pub(super) fn sample(&mut self) -> anyhow::Result<ProcessSample> {
            let memory = self.memory_counters()?;
            let (user_micros, kernel_micros) = self.cpu_micros()?;

            Ok(ProcessSample {
                working_set_bytes: memory.WorkingSetSize as u64,
                peak_working_set_bytes: memory.PeakWorkingSetSize as u64,
                private_bytes: memory.PrivateUsage as u64,
                cpu_user_micros: user_micros,
                cpu_kernel_micros: kernel_micros,
                handle_count: self.handle_count(),
                // Filled by `thread_count`, which costs three orders of
                // magnitude more than everything above it combined.
                thread_count: None,
            })
        }

        pub(super) fn thread_count(&mut self) -> Option<u32> {
            let process_id = std::process::id();
            // A snapshot fails with ERROR_BAD_LENGTH when the system's thread
            // list changes while it is being captured, which is transient and
            // common on a busy machine, so it is worth one retry.
            for _ in 0..2 {
                match snapshot_thread_count(process_id) {
                    Ok(count) => return Some(count),
                    Err(error) if error.code() == HRESULT::from_win32(ERROR_BAD_LENGTH.0) => {
                        continue
                    }
                    Err(_) => return None,
                }
            }
            None
        }

        /// Reads the memory counters in their extended form, which is the only
        /// one that carries `PrivateUsage`.
        fn memory_counters(&self) -> anyhow::Result<PROCESS_MEMORY_COUNTERS_EX> {
            let mut counters = PROCESS_MEMORY_COUNTERS_EX {
                cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                ..PROCESS_MEMORY_COUNTERS_EX::default()
            };

            // SAFETY: the pointer refers to a live, fully initialised
            // PROCESS_MEMORY_COUNTERS_EX on this stack frame. The API is
            // declared over the shorter base struct and distinguishes the two
            // layouts by `cb`, so passing the extended size with a pointer to
            // an extended struct is what selects the extended layout, and the
            // kernel writes exactly `cb` bytes.
            let succeeded = unsafe {
                K32GetProcessMemoryInfo(
                    self.process,
                    (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
                    counters.cb,
                )
            };

            anyhow::ensure!(
                succeeded.as_bool(),
                "GetProcessMemoryInfo failed: {}",
                std::io::Error::last_os_error()
            );
            Ok(counters)
        }

        /// Cumulative user and kernel CPU time, in microseconds.
        fn cpu_micros(&self) -> anyhow::Result<(u64, u64)> {
            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();

            // SAFETY: all four out-parameters are live, initialised FILETIMEs
            // on this stack frame; GetProcessTimes only writes to them.
            unsafe {
                GetProcessTimes(
                    self.process,
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            }
            .map_err(|error| anyhow::anyhow!("GetProcessTimes failed: {error}"))?;

            Ok((filetime_micros(user), filetime_micros(kernel)))
        }

        /// Open handle count, or `None` when the call fails — a missing handle
        /// count is not worth failing an otherwise good memory sample over.
        fn handle_count(&self) -> Option<u32> {
            let mut count = 0u32;
            // SAFETY: `count` is a live u32 on this stack frame and the only
            // thing GetProcessHandleCount writes to.
            unsafe { GetProcessHandleCount(self.process, &mut count) }.ok()?;
            Some(count)
        }
    }

    /// Counts the threads `process_id` owns in a system-wide thread snapshot.
    fn snapshot_thread_count(process_id: u32) -> windows::core::Result<u32> {
        // SAFETY: CreateToolhelp32Snapshot takes no pointer arguments and
        // returns either a valid handle or an error. A process id of 0 with
        // TH32CS_SNAPTHREAD asks for every thread on the system, which is the
        // only form of the thread list Win32 offers.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;

        let walked = walk_threads(snapshot, process_id);

        // SAFETY: `snapshot` is a real kernel handle from the call above, is
        // not used after this point, and is closed exactly once — on the
        // failure path too, since leaking one handle per sample would show up
        // as a climbing `handle_count` in the very next field of this struct.
        let _ = unsafe { CloseHandle(snapshot) };

        walked
    }

    /// Walks an open thread snapshot, counting the entries `process_id` owns.
    fn walk_threads(snapshot: HANDLE, process_id: u32) -> windows::core::Result<u32> {
        let mut entry = THREADENTRY32 {
            // Thread32First rejects an entry that has not declared its size.
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..THREADENTRY32::default()
        };

        // SAFETY: `entry` is a live, initialised THREADENTRY32 on this stack
        // frame whose `dwSize` bounds what the kernel writes into it.
        unsafe { Thread32First(snapshot, &mut entry) }?;

        let mut count = 0;
        loop {
            if entry.th32OwnerProcessID == process_id {
                count += 1;
            }

            // SAFETY: as above — the same entry is reused for each step, which
            // is how the ToolHelp walk is defined.
            match unsafe { Thread32Next(snapshot, &mut entry) } {
                Ok(()) => {}
                // The end of the list, rather than a failure to read it.
                Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => {
                    return Ok(count)
                }
                // A walk cut short would silently undercount, so it is reported
                // as the failure it is.
                Err(error) => return Err(error),
            }
        }
    }

    /// Converts a `FILETIME` duration to microseconds.
    ///
    /// `FILETIME` is a 64-bit count of 100-nanosecond ticks split across two
    /// 32-bit halves. The low half alone wraps after about seven minutes of CPU
    /// time, so both halves have to be recombined before the divide.
    fn filetime_micros(time: FILETIME) -> u64 {
        let ticks = ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
        ticks / 10
    }
}

#[cfg(not(windows))]
mod platform {
    //! `sysinfo` fallback. rgitui's own development happens on Windows, so this
    //! path exists to keep the harness buildable and useful on Linux and macOS
    //! rather than to match Win32's fidelity.
    //!
    //! Three fields are narrower here than on Windows, and are documented at
    //! their assignments below: there is no private-commit equivalent, no peak
    //! working set, and no user/kernel split of CPU time.

    use super::ProcessSample;
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    pub(super) struct Inner {
        system: System,
        pid: Pid,
        /// Largest working set seen across this sampler's own readings, which
        /// stands in for a peak `sysinfo` does not expose. The harness starts
        /// sampling at process start, so it converges on the real high-water
        /// mark within the first tick.
        peak_working_set_bytes: u64,
    }

    impl Inner {
        pub(super) fn new() -> anyhow::Result<Self> {
            let pid = sysinfo::get_current_pid()
                .map_err(|error| anyhow::anyhow!("cannot determine the current pid: {error}"))?;
            Ok(Self {
                system: System::new(),
                pid,
                peak_working_set_bytes: 0,
            })
        }

        pub(super) fn sample(&mut self) -> anyhow::Result<ProcessSample> {
            // Tasks are deliberately not refreshed here: thread count belongs
            // to `thread_count`, which keeps a sample cheap on every platform
            // rather than only on the ones where the count happens to be.
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[self.pid]),
                false,
                ProcessRefreshKind::nothing().with_memory().with_cpu(),
            );

            let process = self
                .system
                .process(self.pid)
                .ok_or_else(|| anyhow::anyhow!("sysinfo did not report process {}", self.pid))?;

            let working_set_bytes = process.memory();
            self.peak_working_set_bytes = self.peak_working_set_bytes.max(working_set_bytes);

            Ok(ProcessSample {
                working_set_bytes,
                peak_working_set_bytes: self.peak_working_set_bytes,
                // `sysinfo` reports virtual size, which measures reserved
                // address space rather than committed private memory. Putting
                // it here would overstate the cost by an order of magnitude, so
                // the field stays zero instead.
                private_bytes: 0,
                // `accumulated_cpu_time` is CPU-milliseconds with no user or
                // kernel split, so the whole total lands in the user field and
                // `cpu_total_micros` stays correct.
                cpu_user_micros: process.accumulated_cpu_time().saturating_mul(1_000),
                cpu_kernel_micros: 0,
                handle_count: None,
                // Filled by `thread_count`, which is where the cost of
                // enumerating threads is paid on every platform.
                thread_count: None,
            })
        }

        pub(super) fn thread_count(&mut self) -> Option<u32> {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[self.pid]),
                false,
                ProcessRefreshKind::nothing().with_tasks(),
            );

            // Tasks are threads, and `sysinfo` only enumerates them on Linux;
            // elsewhere this is `None`.
            self.system
                .process(self.pid)?
                .tasks()
                .map(|tasks| tasks.len() as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_cpu(user_micros: u64, kernel_micros: u64) -> ProcessSample {
        ProcessSample {
            cpu_user_micros: user_micros,
            cpu_kernel_micros: kernel_micros,
            ..ProcessSample::default()
        }
    }

    #[test]
    fn cpu_percent_is_the_share_of_a_single_core() {
        let earlier = with_cpu(0, 0);
        let later = with_cpu(400_000, 100_000);

        assert_eq!(
            ProcessSample::cpu_percent_between(&earlier, &later, 1_000_000),
            Some(50.0)
        );
    }

    #[test]
    fn cpu_percent_can_exceed_one_hundred_across_several_cores() {
        let earlier = with_cpu(0, 0);
        let later = with_cpu(3_000_000, 0);

        assert_eq!(
            ProcessSample::cpu_percent_between(&earlier, &later, 1_000_000),
            Some(300.0)
        );
    }

    #[test]
    fn cpu_percent_refuses_to_divide_by_zero_wall_time() {
        let earlier = with_cpu(0, 0);
        let later = with_cpu(500, 500);

        assert_eq!(
            ProcessSample::cpu_percent_between(&earlier, &later, 0),
            None
        );
    }

    #[test]
    fn cpu_percent_rejects_samples_given_in_the_wrong_order() {
        let earlier = with_cpu(1_000_000, 0);
        let later = with_cpu(10, 0);

        assert_eq!(
            ProcessSample::cpu_percent_between(&earlier, &later, 1_000_000),
            None,
            "CPU counters only climb, so a lower later sample means the pair is out of order"
        );
    }

    #[test]
    fn cpu_total_saturates_instead_of_wrapping() {
        let sample = with_cpu(u64::MAX, 10);
        assert_eq!(sample.cpu_total_micros(), u64::MAX);
    }

    #[cfg(windows)]
    #[test]
    fn sampling_reports_the_live_process_memory() {
        let mut sampler = ProcessSampler::new().unwrap();
        let sample = sampler.sample().unwrap();

        assert!(
            sample.working_set_bytes > 0,
            "a running process always has a working set: {sample:?}"
        );
        assert!(sample.private_bytes > 0, "sample: {sample:?}");
        assert!(
            sample.peak_working_set_bytes >= sample.working_set_bytes,
            "sample: {sample:?}"
        );
        assert!(
            sample.handle_count.is_some_and(|count| count > 0),
            "sample: {sample:?}"
        );
        assert_eq!(
            sample.thread_count, None,
            "a sample is the cheap counters only: {sample:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_thread_count_sees_a_thread_this_test_spawned() {
        let mut sampler = ProcessSampler::new().unwrap();
        let before = sampler.sample_thread_count().unwrap();

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let parked = std::thread::spawn(move || rx.recv());
        let during = sampler.sample_thread_count().unwrap();
        drop(tx);
        parked.join().unwrap().unwrap_err();

        assert!(
            during > before,
            "the spawned thread belongs to this process and should be counted: {before} -> {during}"
        );
    }
}
