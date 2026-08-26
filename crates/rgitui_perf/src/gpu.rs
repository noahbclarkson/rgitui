//! GPU memory and engine utilisation for the current process.
//!
//! # What this can and cannot tell you
//!
//! These counters are sampled at roughly 1 Hz for the process as a whole. They
//! are enough to catch "this change doubled our VRAM" or "the GPU is saturated
//! while scrolling", and they are **not** enough to attribute cost to a draw
//! call or a frame. Per-frame GPU timing would need D3D timestamp queries
//! inside gpui's DirectX renderer, and gpui is a pinned git dependency that
//! rgitui does not patch.
//!
//! Every field is optional and every failure is recorded as a reason string:
//! GPU counters are the most likely part of the harness to be unavailable on a
//! given machine, and a report that quietly showed zeros would be misleading.

use serde::{Deserialize, Serialize};

/// One reading of GPU counters, or the reason there wasn't one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuSample {
    /// Bytes of dedicated (on-card) video memory this process is using,
    /// from DXGI's `QueryVideoMemoryInfo` for the local memory segment.
    pub dedicated_bytes: Option<u64>,
    /// Bytes of shared (system) memory used as video memory — the non-local
    /// segment. Large values here often mean textures spilled out of VRAM.
    pub shared_bytes: Option<u64>,
    /// The driver's current memory budget for this process, in bytes. Usage
    /// approaching the budget is what precedes eviction and stutter.
    pub budget_bytes: Option<u64>,
    /// Percentage utilisation summed across this process's GPU engines.
    /// May exceed 100 when several engines are busy at once.
    pub utilisation_percent: Option<f64>,
    /// Why a field above is `None`, when the cause is known. Empty when
    /// everything was collected.
    pub unavailable: Vec<String>,
}

impl GpuSample {
    /// Whether this sample carries any usable data at all.
    pub fn is_available(&self) -> bool {
        self.dedicated_bytes.is_some()
            || self.shared_bytes.is_some()
            || self.utilisation_percent.is_some()
    }
}

/// Reads [`GpuSample`]s for the current process.
///
/// Construction never fails: an unavailable GPU counter is a degraded report,
/// not a reason to abandon a run that still has frame and memory data. The
/// reason is carried through to the report instead.
pub struct GpuSampler {
    inner: platform::Inner,
}

impl GpuSampler {
    /// Opens whatever GPU counters this machine offers.
    pub fn new() -> Self {
        Self {
            inner: platform::Inner::new(),
        }
    }

    /// Takes one reading.
    pub fn sample(&mut self) -> GpuSample {
        self.inner.sample()
    }
}

impl Default for GpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
mod platform {
    //! DXGI supplies per-process video memory; PDH supplies engine
    //! utilisation through the same counters Task Manager reads.
    //!
    //! The two sources are independent: a machine can report video memory and
    //! no utilisation, or the reverse, so each carries its own reason for being
    //! missing and neither failure suppresses the other.

    use super::GpuSample;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, IDXGIFactory4, DXGI_MEMORY_SEGMENT_GROUP,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO,
    };
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W,
        PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PDH_NO_DATA,
    };

    pub(super) struct Inner {
        /// The primary adapter, kept alive across samples so a tick costs one
        /// `QueryVideoMemoryInfo` per segment rather than a full adapter
        /// enumeration, or the reason there is no adapter to query.
        adapter: Result<IDXGIAdapter3, String>,
        /// The engine utilisation query, or the reason PDH could not open one.
        utilisation: Result<EngineUtilisation, String>,
    }

    impl Inner {
        pub(super) fn new() -> Self {
            Self {
                adapter: open_adapter(),
                utilisation: EngineUtilisation::open(),
            }
        }

        pub(super) fn sample(&mut self) -> GpuSample {
            let mut sample = GpuSample::default();

            match &self.adapter {
                Ok(adapter) => {
                    match video_memory(adapter, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, "local") {
                        Ok(local) => {
                            sample.dedicated_bytes = Some(local.CurrentUsage);
                            sample.budget_bytes = Some(local.Budget);
                        }
                        Err(reason) => sample.unavailable.push(reason),
                    }
                    match video_memory(adapter, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, "non-local") {
                        Ok(non_local) => sample.shared_bytes = Some(non_local.CurrentUsage),
                        Err(reason) => sample.unavailable.push(reason),
                    }
                }
                Err(reason) => sample.unavailable.push(reason.clone()),
            }

            match &mut self.utilisation {
                Ok(counter) => match counter.sample() {
                    Ok(percent) => sample.utilisation_percent = Some(percent),
                    Err(reason) => sample.unavailable.push(reason),
                },
                Err(reason) => sample.unavailable.push(reason.clone()),
            }

            sample
        }
    }

    /// Finds the primary adapter as an `IDXGIAdapter3`, the interface that
    /// carries `QueryVideoMemoryInfo`.
    fn open_adapter() -> Result<IDXGIAdapter3, String> {
        // SAFETY: CreateDXGIFactory1 writes only the interface pointer it
        // returns, and the type parameter supplies the IID matching the type
        // that pointer is bound to. Asking for IDXGIFactory4 also establishes
        // that this is a Windows 10 or newer DXGI, which is what makes the
        // IDXGIAdapter3 cast below worth attempting.
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }
            .map_err(|error| format!("DXGI video memory unavailable: {error}"))?;

        // Adapter enumeration lives on IDXGIFactory1, which IDXGIFactory4
        // derives from; the conversion reinterprets an interface that already
        // implements it rather than issuing a QueryInterface.
        let factory: &IDXGIFactory1 = (&factory).into();

        // SAFETY: `factory` is live for the duration of the call, and
        // EnumAdapters1 returns a new reference-counted adapter or an error.
        let adapter = unsafe { factory.EnumAdapters1(0) }
            .map_err(|error| format!("DXGI reported no primary adapter: {error}"))?;

        adapter
            .cast::<IDXGIAdapter3>()
            .map_err(|error| format!("primary adapter has no IDXGIAdapter3: {error}"))
    }

    /// Reads one memory segment group of the adapter.
    fn video_memory(
        adapter: &IDXGIAdapter3,
        group: DXGI_MEMORY_SEGMENT_GROUP,
        label: &str,
    ) -> Result<DXGI_QUERY_VIDEO_MEMORY_INFO, String> {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();

        // SAFETY: `adapter` is a live interface and `info` is a live,
        // initialised out-parameter on this stack frame. Node 0 is the only
        // node on a single-GPU machine and the first node of a linked set.
        unsafe { adapter.QueryVideoMemoryInfo(0, group, &mut info) }
            .map_err(|error| format!("QueryVideoMemoryInfo failed for {label} memory: {error}"))?;

        Ok(info)
    }

    /// A PDH query over this process's GPU engine instances.
    ///
    /// The driver publishes one instance per engine — 3D, copy, video decode
    /// and so on — named `pid_<PID>_luid_..._engtype_<kind>`, so the path is a
    /// wildcard and the reading is their sum, exactly as Task Manager does it.
    struct EngineUtilisation {
        query: PDH_HQUERY,
        /// `None` until the process has submitted enough GPU work for the
        /// instances to exist. The harness starts before the first frame is
        /// drawn, so the counter is attached lazily rather than once.
        counter: Option<PDH_HCOUNTER>,
    }

    impl EngineUtilisation {
        fn open() -> Result<Self, String> {
            let mut query = PDH_HQUERY::default();

            // SAFETY: `query` is a live out-parameter on this stack frame, and
            // a null data source names the live registry rather than a log file.
            let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
            if status != ERROR_SUCCESS.0 {
                return Err(format!(
                    "GPU utilisation unavailable: PdhOpenQueryW failed (0x{status:08X})"
                ));
            }

            let mut this = Self {
                query,
                counter: None,
            };
            // Failing here only means the engine instances do not exist yet,
            // which `sample` retries; the query itself is open and usable.
            let _ = this.attach();
            Ok(this)
        }

        /// Adds the wildcard counter and primes it with the first collection.
        ///
        /// The counter is recorded as soon as PDH accepts the path, before the
        /// priming collection is attempted: a query with no data yet is a
        /// counter that will start reporting once the process draws something,
        /// and re-adding it every tick would pile duplicate counters into the
        /// query for as long as the process stays idle.
        fn attach(&mut self) -> Result<(), String> {
            // The underscore after the PID is part of the instance name —
            // `pid_1234_luid_..._engtype_3D` — and anchoring on it is what
            // stops `pid_1234*` also matching process 12345, whose engine
            // utilisation would then be summed into this process's sample.
            let path = wide(&format!(
                r"\GPU Engine(pid_{}_*)\Utilization Percentage",
                std::process::id()
            ));
            let mut counter = PDH_HCOUNTER::default();

            // SAFETY: `self.query` is open, `path` is a null-terminated UTF-16
            // buffer that outlives the call, and `counter` is a live
            // out-parameter on this stack frame. The English entry point is
            // what makes a hardcoded counter name work on a localised Windows,
            // where the displayed names are translated.
            let status = unsafe {
                PdhAddEnglishCounterW(self.query, PCWSTR::from_raw(path.as_ptr()), 0, &mut counter)
            };
            if status != ERROR_SUCCESS.0 {
                return Err(format!(
                    "GPU utilisation unavailable: no \\GPU Engine counter for this process (0x{status:08X})"
                ));
            }
            self.counter = Some(counter);

            // PDH derives a rate counter from two collections separated in
            // time, so this one only establishes the baseline.
            self.collect()
        }

        /// Collects one set of raw values into the query.
        fn collect(&self) -> Result<(), String> {
            // SAFETY: `self.query` is open and owns every counter added to it.
            let status = unsafe { PdhCollectQueryData(self.query) };
            if status == ERROR_SUCCESS.0 {
                return Ok(());
            }
            if status == PDH_NO_DATA {
                return Err(
                    "GPU utilisation unavailable: this process has no \\GPU Engine instances, which means it has submitted no GPU work"
                        .to_string(),
                );
            }
            Err(format!(
                "GPU utilisation unavailable: PdhCollectQueryData failed (0x{status:08X})"
            ))
        }

        /// Collects and sums the engine instances.
        fn sample(&mut self) -> Result<f64, String> {
            let counter = match self.counter {
                Some(counter) => {
                    self.collect()?;
                    counter
                }
                None => {
                    self.attach()?;
                    return Err("GPU utilisation pending: the engine counter was attached on this tick and a rate needs two collections".to_string());
                }
            };

            let mut buffer_bytes = 0u32;
            let mut item_count = 0u32;

            // SAFETY: both out-parameters are live u32s on this stack frame,
            // and a null item buffer is how PDH is asked for the size it needs.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_bytes,
                    &mut item_count,
                    None,
                )
            };
            if status == ERROR_SUCCESS.0 {
                // The counter resolved to no instances at all, which is a real
                // reading of an idle process rather than a missing counter.
                return Ok(0.0);
            }
            if status != PDH_MORE_DATA {
                return Err(format!(
                    "GPU utilisation unavailable: PdhGetFormattedCounterArrayW could not size the instance array (0x{status:08X})"
                ));
            }

            // PDH lays the instance array out first and packs the instance name
            // strings after it, so the buffer is sized in bytes but allocated as
            // items to satisfy the array's alignment.
            let item_size = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let capacity = (buffer_bytes as usize).div_ceil(item_size);
            let mut items = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); capacity];
            buffer_bytes = (capacity * item_size) as u32;

            // SAFETY: the buffer is `buffer_bytes` long, correctly aligned for
            // the item type, and PDH writes no further than the size it is
            // given; both counts are live u32s on this stack frame.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_bytes,
                    &mut item_count,
                    Some(items.as_mut_ptr()),
                )
            };
            if status != ERROR_SUCCESS.0 {
                return Err(format!(
                    "GPU utilisation unavailable: PdhGetFormattedCounterArrayW failed (0x{status:08X})"
                ));
            }

            let filled = (item_count as usize).min(items.len());
            let mut total = 0.0;
            for item in &items[..filled] {
                if item.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA
                    && item.FmtValue.CStatus != PDH_CSTATUS_NEW_DATA
                {
                    continue;
                }
                // SAFETY: the union member read here matches the PDH_FMT_DOUBLE
                // format requested above, which is what PDH filled it with.
                total += unsafe { item.FmtValue.Anonymous.doubleValue };
            }
            Ok(total)
        }
    }

    impl Drop for EngineUtilisation {
        fn drop(&mut self) {
            // SAFETY: `query` came from PdhOpenQueryW and is closed exactly
            // once, here; closing it also releases the counter it owns.
            unsafe { PdhCloseQuery(self.query) };
        }
    }

    /// Null-terminated UTF-16, as the `W` entry points expect.
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    //! No portable per-process GPU counters exist outside Windows, so this
    //! reports unavailability with a reason rather than pretending otherwise.

    use super::GpuSample;

    pub(super) struct Inner {
        _private: (),
    }

    impl Inner {
        pub(super) fn new() -> Self {
            Self { _private: () }
        }

        pub(super) fn sample(&mut self) -> GpuSample {
            GpuSample {
                unavailable: vec![
                    "per-process GPU counters are only implemented for Windows".to_string()
                ],
                ..GpuSample::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_with_no_counters_is_not_available() {
        let sample = GpuSample {
            unavailable: vec!["no adapter".to_string()],
            ..GpuSample::default()
        };
        assert!(!sample.is_available());
    }

    #[test]
    fn any_one_counter_makes_a_sample_available() {
        for sample in [
            GpuSample {
                dedicated_bytes: Some(1),
                ..GpuSample::default()
            },
            GpuSample {
                shared_bytes: Some(1),
                ..GpuSample::default()
            },
            GpuSample {
                utilisation_percent: Some(0.0),
                ..GpuSample::default()
            },
        ] {
            assert!(sample.is_available(), "sample: {sample:?}");
        }
    }

    #[test]
    fn a_budget_on_its_own_is_not_a_reading() {
        let sample = GpuSample {
            budget_bytes: Some(4 << 30),
            ..GpuSample::default()
        };
        assert!(
            !sample.is_available(),
            "the budget describes the driver's allowance, not this process's usage"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_sample_either_reports_counters_or_says_why_not() {
        let mut sampler = GpuSampler::new();
        let sample = sampler.sample();

        assert!(
            sample.is_available() || !sample.unavailable.is_empty(),
            "a sample with neither numbers nor a reason would read as a silent zero: {sample:?}"
        );
    }
}
