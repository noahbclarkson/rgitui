//! Executing a scenario against the running app.
//!
//! Every step goes through the same entry point a real user's input does:
//! keystrokes through `Window::dispatch_keystroke` (so the keymap resolves them
//! exactly as it would a real key), commands through gpui's action registry,
//! and mouse input as synthesised `PlatformInput` fed to `Window::dispatch_event`.
//! There is no test harness path anywhere in this — a replayed scenario
//! exercises dispatch, layout, paint and present identically to a person
//! sitting at the keyboard.
//!
//! # The mouse caveat
//!
//! Mouse steps address the window by coordinate, so they are reproducible at a
//! fixed window size and are *not* independent of layout: move a panel and a
//! recorded click lands somewhere else. Keystroke and action steps carry the
//! weight of a scenario for that reason; mouse steps exist for scroll, which
//! has no keyboard equivalent that exercises the same code path.

use std::time::{Duration, Instant};

use gpui::{
    px, AsyncWindowContext, BorrowAppContext as _, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, Point, ScrollDelta,
    ScrollWheelEvent, TouchPhase, Window,
};

use crate::driver::{Condition, Scenario, Settle, Step};
use crate::session::PerfSession;

/// How often [`Condition`] is re-tested while waiting for async git work.
const CONDITION_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How often a frame wait re-checks. Shorter than one 120Hz frame, so a step
/// that settles on a frame is not itself what sets the pace of the scenario.
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(4);

/// Consecutive polls a condition must hold before it counts as settled — half a
/// second at [`CONDITION_POLL_INTERVAL`]. Long enough to see past the gap
/// between two chained git operations, short enough not to pad every step.
const SETTLED_POLLS: u32 = 20;

/// Runs `scenario` to completion, then finishes the session and writes its report.
///
/// Errors abort the run rather than skipping the step: a scenario that half-ran
/// would produce a report whose numbers do not describe anything in particular.
pub async fn run(scenario: Scenario, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    log::info!(
        "replaying scenario {:?} ({} steps)",
        scenario.name,
        scenario.steps.len()
    );

    wait_for_workspace(cx).await?;

    for (index, step) in scenario.steps.iter().enumerate() {
        execute(step, cx)
            .await
            .map_err(|error| anyhow::anyhow!("step {index} ({step:?}) failed: {error}"))?;
    }

    log::info!("scenario {:?} complete", scenario.name);
    cx.update(|_, cx| PerfSession::finish(cx))??;

    // A replayed scenario has a definite end, so the app quits rather than
    // sitting at an idle window. Without this an automated or CI run would hang
    // after producing a perfectly good report.
    cx.update(|_, cx| cx.quit())?;
    Ok(())
}

/// Runs one step.
async fn execute(step: &Step, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    match step {
        Step::Action {
            action,
            repeat,
            settle,
        } => {
            for _ in 0..*repeat {
                cx.update(|window, cx| -> anyhow::Result<()> {
                    let built = cx
                        .build_action(action, None)
                        .map_err(|error| anyhow::anyhow!("cannot build {action:?}: {error}"))?;
                    // Deliberately not recorded here. The generated
                    // `attach_actions` handler calls `note_dispatch` for every
                    // command the app handles, so recording it on this side too
                    // would count each replayed step twice — and would also
                    // record steps that never reached a handler at all, which
                    // is exactly the failure this driver needs to stay able to
                    // detect.
                    window.dispatch_action(built, cx);
                    Ok(())
                })??;
                settle_for(*settle, cx).await;
            }
        }

        Step::Key {
            key,
            repeat,
            settle,
        } => {
            let keystroke = Keystroke::parse(key)
                .map_err(|error| anyhow::anyhow!("cannot parse keystroke {key:?}: {error}"))?;
            for _ in 0..*repeat {
                cx.update(|window, cx| {
                    window.dispatch_keystroke(keystroke.clone(), cx);
                })?;
                settle_for(*settle, cx).await;
            }
        }

        Step::Scroll {
            x,
            y,
            delta_y,
            repeat,
            settle,
        } => {
            let position = Point::new(px(*x), px(*y));
            for _ in 0..*repeat {
                cx.update(|window, cx| {
                    window.dispatch_event(
                        PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position,
                            delta: ScrollDelta::Pixels(Point::new(px(0.0), px(*delta_y))),
                            modifiers: Modifiers::default(),
                            touch_phase: TouchPhase::Moved,
                        }),
                        cx,
                    );
                })?;
                settle_for(*settle, cx).await;
            }
        }

        Step::Click { x, y } => {
            let position = Point::new(px(*x), px(*y));
            cx.update(|window, cx| {
                // Moving first matters: hover state and hitbox tracking are
                // established by the move, and a click delivered without one
                // can land on an element that never became interactive.
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }),
                    cx,
                );
            })?;
            next_frame(cx).await;
        }

        Step::Wait { ms } => {
            cx.background_executor()
                .timer(Duration::from_millis(*ms))
                .await;
        }

        Step::WaitFor {
            condition,
            timeout_ms,
        } => {
            wait_for(*condition, Duration::from_millis(*timeout_ms), cx).await?;
        }

        Step::Mark { mark } => {
            cx.update(|_, cx| {
                cx.update_global::<PerfSession, _>(|session, _| session.begin_mark(mark));
            })?;
        }

        Step::EndMark { mark } => {
            cx.update(|_, cx| {
                cx.update_global::<PerfSession, _>(|session, _| session.end_mark(mark));
            })?;
        }

        Step::Dirty { count } => {
            let repo_path = cx.update(|_, cx| crate::session::active_repo_path(cx))?;
            let Some(repo_path) = repo_path else {
                anyhow::bail!(
                    "a `dirty` step needs an open repository, and no tab has one — put it after \
                     the `wait_for` that lets the first refresh finish"
                );
            };
            let count = *count;
            let touched = cx
                .background_executor()
                .spawn(async move { dirty_tracked_files(&repo_path, count) })
                .await?;
            log::info!("dirtied {touched} tracked files");
        }

        Step::Snapshot { snapshot } => {
            cx.update(|_, cx| {
                cx.update_global::<PerfSession, _>(|session, cx| {
                    let bytes = session.take_census(cx);
                    log::info!("census at {snapshot:?}: {} bytes", bytes);
                });
            })?;
        }
    }

    Ok(())
}

/// Waits according to a step's settle policy.
async fn settle_for(settle: Settle, cx: &mut AsyncWindowContext) {
    match settle {
        Settle::None => {}
        Settle::Frame => next_frame(cx).await,
        // Paced off a clock rather than off the app, so a step that the app
        // cannot keep up with builds a backlog exactly as a held key does.
        Settle::Rate { hz } => {
            if hz > 0.0 {
                cx.background_executor()
                    .timer(Duration::from_secs_f64(1.0 / hz))
                    .await;
            }
        }
    }
}

/// How long to wait for the splash to hand over before giving up on a run.
///
/// Generous: the handover waits for the repository, and a cold open of a very
/// large one is allowed to be slow. It is a timeout rather than an unbounded
/// wait so that a run which will never start says so instead of hanging.
const WORKSPACE_TIMEOUT: Duration = Duration::from_secs(120);

/// Blocks until the workspace has rendered, so the first step has something to
/// talk to.
///
/// Without this a scenario begins while the splash is still up: the harness
/// installs as soon as the workspace entity is constructed, which happens well
/// before it is shown. Short scenarios then ran to completion against a window
/// with no workspace in it and reported an idle one, and long ones lost however
/// many steps fell before the handover.
async fn wait_for_workspace(cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let deadline = Instant::now() + WORKSPACE_TIMEOUT;
    loop {
        if cx.update(|_, cx| PerfSession::has_rendered(cx))? {
            return Ok(());
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "the workspace never rendered within {:?} — the splash never handed over, so                  there was nothing for this scenario to drive",
                WORKSPACE_TIMEOUT
            );
        }
        cx.background_executor().timer(FRAME_POLL_INTERVAL).await;
    }
}

/// How long a `settle: frame` step waits before giving up.
///
/// Bounded so a step that never triggers a repaint slows the scenario rather
/// than hanging it forever. Every expiry is counted — see
/// [`PerfSession::note_settle_timeout`].
const SETTLE_TIMEOUT: Duration = Duration::from_millis(500);

/// Yields until the app has presented at least one more frame.
///
/// Scenario steps that settle this way measure keypress-to-pixels rather than
/// keypress-to-return, which is the latency a user actually experiences.
async fn next_frame(cx: &mut AsyncWindowContext) {
    let before = cx.update(|_, cx| frame_count(cx)).unwrap_or(0);

    // Bounded so a step that never triggers a repaint slows the scenario rather
    // than hanging it forever.
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    loop {
        cx.background_executor().timer(FRAME_POLL_INTERVAL).await;
        let now = cx.update(|_, cx| frame_count(cx)).unwrap_or(before);
        if now > before {
            return;
        }
        if Instant::now() > deadline {
            // Counted, not swallowed. `wait_for` treats its timeout as an
            // error; this one cannot, because a step that legitimately draws
            // nothing should not fail a run — but a report that stayed silent
            // about it would present the timeout as the app's response time.
            cx.update(|_, cx| PerfSession::note_settle_timeout(cx)).ok();
            return;
        }
    }
}

/// Frames presented so far, or 0 when no session is installed.
fn frame_count(cx: &gpui::App) -> usize {
    cx.try_global::<PerfSession>()
        .map(PerfSession::frame_count)
        .unwrap_or(0)
}

/// Blocks until `condition` has held continuously for [`SETTLED_POLLS`] polls,
/// failing the run on timeout.
///
/// Sustained rather than instantaneous, because an instantaneous check is
/// satisfied by the wrong things. At startup nothing has been queued yet, so the
/// app is idle for the moment before it becomes busy — a single check there
/// returns immediately and the step measures nothing, which is exactly what the
/// first run of `startup.json` did. The same applies between two operations,
/// where a momentary gap would be read as the work being finished.
///
/// A timeout is an error rather than a shrug: the alternative is a report that
/// quietly measured a half-finished operation.
async fn wait_for(
    condition: Condition,
    timeout: Duration,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut consecutive = 0;

    loop {
        let satisfied = cx.update(|_, cx| match condition {
            Condition::OperationIdle => crate::session::is_idle(cx),
        })?;

        consecutive = if satisfied { consecutive + 1 } else { 0 };
        if consecutive >= SETTLED_POLLS {
            return Ok(());
        }

        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out after {:?} waiting for {condition:?}",
            timeout
        );
        cx.background_executor()
            .timer(CONDITION_POLL_INTERVAL)
            .await;
    }
}

/// Starts a scenario on the window's foreground task queue.
///
/// A failed scenario is logged rather than propagated: by the time it runs the
/// app is already up, and tearing the window down would destroy the partial
/// data that explains the failure.
pub fn spawn(scenario: Scenario, window: &mut Window, cx: &mut gpui::App) {
    window
        .spawn(cx, async move |cx| {
            let Err(error) = run(scenario, cx).await else {
                return;
            };
            log::error!("perf scenario failed: {error}");

            // Salvage the partial run. The data up to the failure is usually
            // what explains it, and a failed run that also hangs at an idle
            // window would leave an automated caller waiting forever.
            let _ = cx.update(|_, cx| {
                if let Err(error) = PerfSession::finish(cx) {
                    log::error!("cannot write the partial perf report: {error}");
                }
                cx.quit();
            });
        })
        .detach();
}

/// Appends a line to up to `count` tracked files, returning how many changed.
///
/// Reads the HEAD tree rather than the working directory so it only ever
/// touches files git already knows about — an untracked scratch file would
/// change what `git status` reports without exercising the staging path a
/// scenario is trying to measure. Paths are taken in tree order, which is
/// stable for a given corpus, so two runs of the same scenario dirty the same
/// files.
fn dirty_tracked_files(repo_path: &std::path::Path, count: usize) -> anyhow::Result<usize> {
    let repo = git2::Repository::open(repo_path)
        .map_err(|error| anyhow::anyhow!("cannot open {}: {error}", repo_path.display()))?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("a bare repository has no working tree to dirty"))?
        .to_path_buf();
    let tree = repo.head()?.peel_to_commit()?.tree()?;

    let mut paths = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        if paths.len() >= count {
            return git2::TreeWalkResult::Abort;
        }
        if entry.kind() == Some(git2::ObjectType::Blob) {
            if let Some(name) = entry.name() {
                paths.push(format!("{dir}{name}"));
            }
        }
        git2::TreeWalkResult::Ok
    })?;

    let mut touched = 0;
    for path in paths {
        let full = workdir.join(&path);
        // Appending rather than rewriting keeps the change small, so the diff a
        // scenario then stages is a realistic one rather than a whole-file
        // replacement.
        match std::fs::OpenOptions::new().append(true).open(&full) {
            Ok(mut file) => {
                use std::io::Write as _;
                writeln!(file, "// touched by the perf harness")?;
                touched += 1;
            }
            Err(error) => log::warn!("could not dirty {}: {error}", full.display()),
        }
    }
    Ok(touched)
}
