# Testing rgitui

## Running the test suite

```bash
cargo test
```

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
