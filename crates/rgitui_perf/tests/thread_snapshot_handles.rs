//! That counting threads does not leak the snapshot handle it opens.
//!
//! # Why this is not a unit test
//!
//! The only evidence available is `handle_count`, which is a *process-global*
//! counter. Inside the crate's unit-test binary this test would share a process
//! with everything else — the corpus generator opening git repositories, the
//! GPU sampler opening DXGI and PDH handles — all running concurrently and all
//! moving the number it reads. It passed alone and failed in the suite, which
//! is the worst failure mode a test has: it trains you to ignore it.
//!
//! Cargo gives each integration test file its own process, so here the counter
//! moves only for the reason under test.

#[cfg(windows)]
#[test]
fn counting_threads_does_not_leak_the_snapshot_handle() {
    use rgitui_perf::process::ProcessSampler;

    let mut sampler = ProcessSampler::new().expect("process counters");

    // Each call opens one ToolHelp snapshot. If any path failed to close it,
    // the handle count would climb by exactly this many.
    const CALLS: u32 = 20;

    let before = sampler
        .sample()
        .expect("baseline sample")
        .handle_count
        .expect("Windows reports a handle count");

    for _ in 0..CALLS {
        sampler
            .sample_thread_count()
            .expect("a live process always has threads");
    }

    let after = sampler
        .sample()
        .expect("final sample")
        .handle_count
        .expect("Windows reports a handle count");

    assert!(
        after < before + CALLS,
        "one unclosed snapshot per call would show up here: {before} -> {after}"
    );
}
