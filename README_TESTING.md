# Testing rgitui

## Running the test suite

```bash
cargo test
```

## GPUI view tests (headless, no display needed)

gpui's `test-support` feature unlocks a headless test harness: `gpui::TestApp`
opens real windows against a fake platform (deterministic executor, simulated
keyboard/mouse/clipboard), so view behavior can be tested end to end without a
GPU or display — including on CI.

Write these tests through `rgitui_test_support::ViewTest`, which opens a themed
test app with one window and tears both down in `Drop`. Add it to the crate's
`[dev-dependencies]` alongside `gpui = { workspace = true, features = ["test-support"] }`.

The reference test is `palette_keyboard_navigation_in_test_window` in
`crates/rgitui_workspace/src/command_palette.rs`. The pattern:

```rust
#[test]
fn my_view_test() {
    let mut view = ViewTest::open(|window, cx| MyView::new(cx));

    view.update(|view, window, cx| { /* drive the view */ });
    view.simulate_input("push");           // types into the focused element
    view.simulate_keystroke("down");       // single keystroke, e.g. "ctrl-a"
    view.read(|view, cx| assert!(/* observable state changed */));
}
```

`ViewTest` derefs to `TestAppWindow`, so every `simulate_*`/`update`/`read`
method is available directly; `view.app()` reaches the hosting `TestApp` for
globals, clock control, and spawning.

Notes:

- Windows are re-drawn automatically after every `update`/`simulate_*` call,
  so key listeners registered in `render` are always current.
- **Teardown matters, which is why `ViewTest` owns it.** `test-support` enables
  leak detection, which asserts at shutdown that every entity handle was
  released — a test that opens a window itself must drop it and *then* call
  `cx.shutdown()`, or teardown panics even when the test passed. `ViewTest` does
  this in `Drop`, and skips the shutdown while the thread is panicking so a
  failing assertion is reported instead of being masked by a leak panic.
- Keystrokes dispatch to the focused element and bubble to ancestors, exactly
  like real input. Printable keys get a simulated IME `key_char`.
- `test-support` applies to test builds only; `cargo build`/`cargo run` build
  gpui without it, so the shipped binary is unaffected. `cargo test` and
  `cargo clippy --all-targets` build a second gpui variant with the feature —
  expect one slow build the first time; cargo then caches both variants.
- SVG icons resolve against the embedded assets of the `rgitui` binary and are
  absent in workspace tests; gpui logs and skips them (no panic).
- At the pinned Zed rev, `VisualTestContext` is macOS-only — use `TestApp` /
  `TestAppWindow` (cross-platform) instead.

## Taking a screenshot for visual testing

First, make sure the app is built:
```bash
cargo build
```

Then run the screenshot script:
```bash
./scripts/screenshot.sh
```

This will:
1. Start rgitui with the test repo (default: `~/repos/test-repo`)
2. Wait 3 seconds for the window to render
3. Take a screenshot using `grim` (Wayland) or `scrot` (X11)
4. Save to `test_output/screenshot_TIMESTAMP.png`
5. Kill the app

## Configuration

Copy `.env` and adjust:
```env
RGITUI_TEST_REPO=/path/to/your/test/repo
```

## Making the screenshot script executable

```bash
chmod +x scripts/screenshot.sh
```

## Integration with AI-assisted development

Screenshots in `test_output/` can be loaded using the `Read` tool to visually verify the UI appearance during development.

## Unit test structure

Tests are in `crates/rgitui/tests/integration_test.rs` and test:
- Repository can be opened with git2
- Graph layout algorithm produces valid output for empty and real repos
- Binary artifact exists after build (ignored, run manually)

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `RGITUI_TEST_REPO` | `$HOME/repos/test-repo` | Path to git repo used for integration tests |
