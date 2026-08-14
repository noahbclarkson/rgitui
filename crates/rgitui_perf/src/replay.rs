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
                    PerfSession::note_dispatch(action, None, cx);
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
    }
}

/// Yields until the app has presented at least one more frame.
///
/// Scenario steps that settle this way measure keypress-to-pixels rather than
/// keypress-to-return, which is the latency a user actually experiences.
async fn next_frame(cx: &mut AsyncWindowContext) {
    let before = cx.update(|_, cx| frame_count(cx)).unwrap_or(0);

    // Bounded so a step that never triggers a repaint slows the scenario rather
    // than hanging it forever.
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        cx.background_executor().timer(FRAME_POLL_INTERVAL).await;
        let now = cx.update(|_, cx| frame_count(cx)).unwrap_or(before);
        if now > before || Instant::now() > deadline {
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
