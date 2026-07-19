# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

rgitui is a GPU-accelerated **desktop** Git client (not a TUI, despite the name) built on GPUI — Zed's UI framework, pinned to a specific Zed git rev in the root `Cargo.toml`. Rust toolchain is pinned to 1.94.1 via `rust-toolchain.toml`.

## Commands

```bash
cargo build                                            # debug build
cargo run                                              # run the app (or: cargo xt — alias for run --package rgitui --)
cargo run -- /path/to/repo                             # open a specific repository
cargo fmt --all                                        # format
cargo clippy --workspace --all-targets -- -D warnings  # lint (CI fails on any warning)
cargo test --workspace                                 # all tests
cargo test -p rgitui_git                               # tests for one crate
cargo test -p rgitui_git test_name                     # a single test
```

CI (`.github/workflows/ci.yml`) runs fmt-check, clippy `-D warnings`, and tests on Linux/Windows/macOS. Workspace lints deny `dbg!` and `todo!`. Most tests are plain `#[test]` on pure helper functions and need no display; the one integration test is `crates/rgitui/tests/integration_test.rs`.

## Architecture

Crate dependency direction: `rgitui` (binary) → `rgitui_workspace` → {`rgitui_git`, `rgitui_diff`, `rgitui_graph`, `rgitui_ai`, `rgitui_ui`} → {`rgitui_settings`, `rgitui_theme`}.

### Boot (`crates/rgitui/src/main.rs`)

`main()` builds a `gpui::Application` with embedded assets (RustEmbed), initializes `rgitui_theme::init` and `rgitui_settings::init`, resolves which repos to open (CLI arg → active saved workspace → cwd), and creates the root view `AppRoot`, which shows a splash screen then swaps in the `Workspace` entity. `SettingsState::mark_startup()` detects unclean exits and drives crash recovery via workspace snapshot restore.

### State management (`crates/rgitui_workspace/`)

`Workspace` (`src/workspace/mod.rs`) is the root GPUI entity. It owns `Vec<ProjectTab>` — one per open repo — and each `ProjectTab` holds an `Entity<T>` for every panel: `GitProject`, `GraphView`, `DiffViewer`, `Sidebar`, `DetailPanel`, `CommitPanel`, `Toolbar`, blame/history/reflog/bisect views, plus per-tab LRU `ViewCaches`. Cross-cutting state is grouped in `src/workspace/state.rs` (`LayoutState`, `DialogState`, `OverlayState`, `OperationState`, `FocusState`).

**Communication pattern**: every child entity implements `EventEmitter<XxxEvent>`; the Workspace subscribes in `src/workspace/events.rs` (`subscribe_xxx` functions, wired per-tab in `tabs.rs`), reads children with `.read(cx)`, and pushes updates with `.update(cx, ...)`. Spawned async tasks capture `WeakEntity` handles.

Example flow: `GraphView` emits `CommitSelected(oid)` → `subscribe_graph` spawns `rgitui_git::compute_commit_diff` on the background executor → result applied on the UI thread via `diff_viewer.update(cx, |dv, cx| dv.set_diff(...))`.

### Git layer (`crates/rgitui_git/`)

`GitProject` (`src/project/mod.rs`) is the central entity; it caches a repo snapshot (branches, tags, remotes, stashes, worktrees, status, recent commits) and emits `GitProjectEvent` (`StatusChanged`, `HeadChanged`, `RefsChanged`, `OperationUpdated`, ...). Operations are split across `src/project/{local_ops,network,rebase,bisect,blame,file_history,reflog,submodule,search,diff,refresh,auth,watcher}.rs`.

- **git2 vs CLI**: local operations use git2 (libgit2). Network/auth operations (fetch, pull, push) shell out to the `git` CLI via `run_git_network_command` in `src/project/auth.rs`; `search.rs` shells out to `git grep`.
- **Async pattern** (the threading rule for the whole app): never do git2/FS/HTTP work on the UI thread. Ops call `begin_operation(...)` (monotonic id, emits `OperationUpdated`), do the heavy work in `cx.background_executor().spawn(...)` producing a `Send` `RefreshData` snapshot, then apply it on the UI thread inside `this.update(...)` via `apply_refresh_data`, which bumps `refresh_generation` (stale async results are dropped by comparing generations).
- **File watcher** (`src/project/watcher.rs`): `notify` watcher + a 300ms poll loop fingerprinting `.git` state; on change, debounces 200ms then runs a lightweight cached refresh.
- **Undo**: `UndoStack` in `rgitui_workspace/src/workspace/undo.rs` — pure and unit-tested, 20-entry cap, 60s TTL; each `UndoAction` stamps repo+worktree paths so undo routes to the originating tab.

### Rendering crates

- `rgitui_diff` (`src/lib.rs`): `DiffViewer` receives a `FileDiff` via `set_diff`; precomputes rows per `DiffDisplayMode` (Unified/SideBySide/ThreeWay) into a virtualized list; LRU display cache; syntect highlighting in a process-wide `OnceLock`, with `similar`-based intra-line word-diff merged on top. Emits stage/unstage hunk/line request events back to the workspace.
- `rgitui_graph` (`src/lib.rs`): `GraphView` renders lane layout computed in `rgitui_git::compute_graph` (`src/graph.rs`); rows in a `uniform_list`, edges/nodes drawn with `gpui::canvas` + `PathBuilder` using precomputed trig tables.

### UI, theme, settings, AI

- `rgitui_ui`: flat re-exported component library (Button, Modal, TextInput, Toast, ContextMenu, ...). Layout uses `h_flex()`/`v_flex()` helpers. **Gotcha**: `h_flex()` is `flex_row().items_center()` (`rgitui_theme/src/theme.rs:287`) — the forced vertical centering is a recurring cause of broken scroll containers and misaligned children; use `div().flex().flex_row()` when you don't want centering.
- `rgitui_theme`: `ThemeState` is a GPUI `Global`; access in render code via `cx.theme()`, `cx.colors()`, `cx.status()` (semantic tokens). Built-in themes are Rust-defined in `src/colors.rs`; user themes load from JSON.
- `rgitui_settings`: `SettingsState` global; config at `dirs::config_dir()/rgitui/settings.json`, written by a dedicated writer thread (non-blocking `save()`). Secrets go through the OS keychain (`keyring` crate). Also stores workspace snapshots and the clean-exit flag.
- `rgitui_ai`: `AiGenerator` entity; provider dispatch by settings string (Gemini, OpenAI-compatible, DeepSeek, Anthropic) in `generate_commit_message_with_tools`, with optional agentic tool-calling defined in `tools.rs`.

### Conventions

- Entities are named `XxxView`/`XxxPanel`/`XxxDialog`, each with a sibling `XxxEvent`; workspace subscriptions live in `workspace/events.rs`.
- Errors surface to the user as toasts (`Workspace::show_toast`) via failed `OperationUpdated` events; write user-facing error strings as actionable sentences at the git layer.
- Shared snapshots are `Arc`-wrapped rather than cloned.
- Prefer extracting view logic into pure, GPUI-free helper functions so it can be unit-tested without a display.
