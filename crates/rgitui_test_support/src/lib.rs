//! Helpers for driving rgitui views in a headless GPUI app.
//!
//! GPUI's `test-support` feature enables leak detection, which asserts at
//! shutdown that every entity handle has been released. A test that opens a
//! window must therefore drop it and then shut the app down, in that order, or
//! teardown panics even when the test itself passed. [`ViewTest`] owns both and
//! does this in `Drop`, so there is nothing to remember and nothing to get
//! wrong.
//!
//! ```no_run
//! # use rgitui_test_support::ViewTest;
//! # struct MyView;
//! # impl gpui::Render for MyView {
//! #     fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement { gpui::div() }
//! # }
//! let mut view = ViewTest::open(|_window, _cx| MyView);
//! view.simulate_input("hello");
//! view.simulate_keystroke("escape");
//! view.read(|my_view, _cx| { /* assert on state */ });
//! ```

use std::ops::{Deref, DerefMut};

use gpui::{Context, Render, TestApp, TestAppWindow, Window};

/// A themed headless app with one open window, torn down on drop.
///
/// Derefs to the underlying [`TestAppWindow`], so `simulate_keystroke`,
/// `simulate_input`, `update`, and `read` are called directly on it.
pub struct ViewTest<V: Render + 'static> {
    app: TestApp,
    // `Option` so `Drop` can drop the window before shutting the app down.
    window: Option<TestAppWindow<V>>,
}

impl<V: Render + 'static> ViewTest<V> {
    /// Opens `build_view` in a maximised window of a new test app, with
    /// `rgitui_theme` initialised so views may call `cx.colors()`.
    pub fn open(build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V) -> Self {
        let mut app = TestApp::new();
        app.update(rgitui_theme::init);
        let window = app.open_window(build_view);
        Self {
            app,
            window: Some(window),
        }
    }

    /// The app hosting the window, for globals, clock control, and spawning.
    pub fn app(&mut self) -> &mut TestApp {
        &mut self.app
    }
}

impl<V: Render + 'static> Deref for ViewTest<V> {
    type Target = TestAppWindow<V>;

    fn deref(&self) -> &Self::Target {
        self.window
            .as_ref()
            .expect("window dropped before teardown")
    }
}

impl<V: Render + 'static> DerefMut for ViewTest<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.window
            .as_mut()
            .expect("window dropped before teardown")
    }
}

impl<V: Render + 'static> Drop for ViewTest<V> {
    fn drop(&mut self) {
        drop(self.window.take());

        // A failing test is already unwinding; shutting down here would report
        // leaked handles on top of the real failure, or abort on double panic.
        if std::thread::panicking() {
            return;
        }

        self.app.update(|cx| cx.shutdown());
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use gpui::{div, InteractiveElement, IntoElement};

    use super::*;

    struct Probe {
        keys: Vec<String>,
        focus_handle: gpui::FocusHandle,
    }

    impl Probe {
        fn focused(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let focus_handle = cx.focus_handle();
            focus_handle.focus(window, cx);
            Self {
                keys: Vec::new(),
                focus_handle,
            }
        }
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .track_focus(&self.focus_handle)
                .on_key_down(cx.listener(|probe, event: &gpui::KeyDownEvent, _, cx| {
                    probe.keys.push(event.keystroke.key.clone());
                    cx.notify();
                }))
        }
    }

    #[test]
    fn window_is_usable_and_torn_down_without_leaking_handles() {
        let mut probe = ViewTest::open(Probe::focused);
        probe.simulate_keystroke("escape");
        probe.read(|probe, _| assert_eq!(probe.keys, ["escape"]));
        // Dropping here must not panic; that is the whole guarantee.
    }

    #[test]
    fn a_failing_test_reports_its_own_panic_not_a_leak() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _probe = ViewTest::open(Probe::focused);
            panic!("assertion from the test body");
        }));

        let panic = result.expect_err("body should have panicked");
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
            .unwrap_or_default();
        assert_eq!(message, "assertion from the test body");
    }
}
