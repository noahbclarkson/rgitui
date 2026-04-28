# Changelog

All notable changes to rgitui are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.8] - 2026-04-28

### Added

- **Custom theme editor** (Ctrl+Shift+T / Alt+9): live color-picker UI for all
  theme fields with hex input and swatch preview. Edited themes are saved as
  JSON files to `~/.config/rgitui/themes/` and reloaded on next launch.
- **JSON theme serialization**: themes can be exported and re-imported as JSON.
  Built-in themes ship as embedded JSON; user themes are loaded from the config
  directory at startup.
- **Branch from stash**: a Branch button on each stash panel row lets you create
  a branch from any stash entry directly, without going through the command
  palette.

### Fixed

- **Theme editor black-on-save regression**: saving a theme no longer
  momentarily resets all colors to black; the editable state is properly
  resynced after save.
- **Theme editor startup popup**: opening the editor no longer flashes an empty
  or stale state on first render.
- **Invalid hex fallback in theme editor**: typing a partial hex value no longer
  silently snaps the swatch to black; the swatch holds the field's current theme
  color until a valid hex is entered.
- **Graph lane-0 gap**: lane 0 is now drawn continuously from the first
  side-branch curve-in to `main_tip`, preventing gaps when the newest commits
  are on side branches.
- **`is_merged_into_main` correctness**: merge detection now uses `merge_base`
  instead of `graph_descendant_of`, fixing false positives on non-linear
  histories.
- **Search output parser**: malformed `git grep` output no longer silently drops
  results; a simple-split fallback preserves partial matches.
- **Windows UNC paths (WSL2)**: repository paths beginning with
  `\\server\share\…` are rewritten to `//server/share/…` for libgit2
  compatibility. Extended-length (`\\?\`) and device (`\\.\`) prefixes are left
  unchanged.

## [0.1.7] - 2026-04-18

### Added

- **Dedicated Stashes panel** (Alt+8): lists every stash with its message and
  OID, with inline Apply, Pop, and Drop buttons. Dropping a stash prompts a
  confirmation dialog. A stash-count badge appears in the toolbar. The command
  palette also gains `Git: Create Branch from Stash`, visible when stashes
  exist.
- **Diff line-wrap toggle**: new "Wrap Long Lines in Diff" setting in the Diff
  settings card switches between wrapping and horizontal scrolling. Wrapping
  respects the text layouter so lines break correctly in all diff modes.
- **Diff horizontal scrollbar**: in no-wrap Unified mode, the whole list
  scrolls horizontally with a draggable scrollbar at the bottom. In Split and
  Three-way modes each column keeps its own horizontal scroll area so long
  lines no longer bleed across the column divider. The line-number gutter
  stays pinned in all modes.
- **Diff vertical scrollbar**: a draggable vertical scrollbar now sits next
  to the diff body in every mode.
- **Watch all worktrees** (General settings): when enabled, the filesystem
  watcher tracks every linked worktree, so external changes in any worktree
  trigger a refresh.

### Changed

- **Wrap-mode diff viewer is virtualized**: only the visible rows (plus a
  small overdraw buffer) are rendered per frame, and wrapped-line heights are
  measured once and cached. Large diffs stay responsive while scrolling with
  wrap enabled.
- **Stash detail summary**: stash entries in the detail panel now show a
  clean subject line. The `WIP on <branch>: <sha>` prefix is stripped, with
  graceful fallbacks to the branch name or full message for custom stash
  names.

### Fixed

- **Wrap mode no longer overlaps lines**: wrapped continuation lines used to
  paint on top of the row below because flex-shrink squeezed rows when total
  content exceeded the viewport. Wrapped text now correctly extends the row
  it belongs to.
- **Word-diff noise on reformatted hunks**: word-level highlighting paired
  deletion line *i* with addition line *i* regardless of whether the counts
  matched. On a reformat (e.g. inlining a closure body into a multi-line
  match block) the pairing lined up semantically unrelated rows and every
  coincidental token match produced a misleading highlight. Word-diff is now
  skipped when deletion and addition line counts differ, so reformats render
  as clean red/green blocks.
- **Command palette stale after stash operations**: after Apply / Pop / Drop,
  the palette's `has_stashes`, `has_changes`, and related predicates are
  rebuilt once the active tab finishes refreshing, so the next open reflects
  the actual repo state.
- **Commit graph lane artifacts with worktrees**: fixed a ghost lane-0
  column extending above `main_tip` when the newest commits were on side
  branches, a dangling stub on lane 1 at the main-tip row, and missing
  pass-through strokes across worktree virtual rows for merge-in lanes.
  Worktrees whose HEAD isn't in the visible commit list are now routed to
  orphan slots instead of being silently dropped.
- **Action button clipping in dialogs**: the Discard and Unstage buttons in
  the sidebar file list, and the action buttons in the Create PR dialog,
  were clipped against the right edge on Windows. Right padding now matches
  the rest of the dialog surface.

## [0.1.2] - 2026-04-09

### Added

- **macOS support**: Bundle embedded fonts (IBM Plex Sans, Lilex, JetBrains Mono)
  so text and icons render correctly on macOS. Ad-hoc code sign the .app bundle
  to prevent Gatekeeper "damaged" errors. Generate .icns app icon from PNGs
  during the Mac bundle step.
- **macOS CI**: Add `macos-14` to the CI test matrix alongside Linux and Windows.
- **Configurable commit limit**: New `commit_limit` setting (default 1000) lets
  users control how many commits are loaded per repo.

### Changed

- **Startup performance**: Repos now load during the splash animation instead of
  after it. Active tab refreshes first; inactive tabs load after it completes.
  Avatar disk cache loads on a background thread.
- **Commit walk uses git subprocess**: Replaced libgit2's `revwalk` with
  `git log` subprocess, leveraging commit-graph files for ~100x speedup on large
  repos (Linux kernel: 14.7s to 1.2s).
- **Two-phase commit loading**: First 100 commits load immediately so the graph
  appears fast, remaining commits load in the background.
- **Deferred commit metadata**: GPG signature checking and co-author parsing
  are skipped during the commit walk and computed on-demand when clicking a
  commit, reducing per-commit overhead.
- **Parallel status computation**: Working tree status now runs on a separate
  thread in parallel with stash enumeration and the commit walk.
- **Lightweight initial refresh**: Initial repo load skips ahead/behind
  computation for all branches; it runs in the background after the UI appears.
- **Diff computation**: Removed redundant `diff.stats()` call (~31% CPU savings)
  and intra-diff thread serialization overhead. Diff cache prewarming fires once
  per tab and only for the active tab initially.
- **Diff cache shared across workspace**: The LRU diff cache is now stored in
  `ViewCaches` and shared between project and graph subscriptions.
- **Selected commit priority**: On cache miss, the clicked commit's diff task is
  submitted to the thread pool before neighbor prefetch tasks to ensure it gets
  processed first.

### Fixed

- Resolved clippy warnings in bisect view and workspace events.
- Fixed unnecessary reference creation in events.rs and items-after-test-module
  ordering in bisect.rs.

## [0.1.0] - 2026-04-08

First public release. rgitui is a GPU-accelerated, multi-repo desktop Git
client built with [GPUI](https://github.com/zed-industries/zed). This release
establishes a feature-complete baseline for day-to-day use.

### Added

#### Core git operations
- Stage, unstage, and discard changes at file, hunk, and line granularity
- Commit and amend with a message editor, co-author support, and optional
  AI-generated commit messages (Gemini)
- Branch management: create, checkout, rename, delete, and switch from the
  sidebar or command palette
- Tag management: create annotated and lightweight tags, delete, and checkout
- Stash: save (with optional name), pop, apply, drop, and create branch from
  stash
- Remote operations: fetch, pull, push, and force push with multi-remote
  support and automatic upstream tracking
- Cherry-pick, revert, and reset (hard / soft / mixed)
- Merge with conflict detection and inline "accept ours / accept theirs"
  resolution
- Interactive rebase with pick / squash / reword / fixup / drop actions
- Bisect with start, good, bad, skip, and reset commands
- Worktrees: create, list, and switch
- Submodule initialization, update, and management
- Clean untracked files
- Undo stack for recent local git operations
- Crash recovery: workspace snapshots restored after unclean shutdown

#### Views
- Animated commit graph with lane-based coloring and Bezier-curve edges
- Unified, side-by-side, and three-way conflict diff modes with syntax
  highlighting via syntect
- Blame view with per-line author avatars
- File history view
- Reflog viewer
- Submodule panel
- Global search via `git grep`
- Commit graph search
- Detail panel with commit metadata, file list, and diff stats
- Issues and Pull Requests panels for GitHub repositories

#### UI
- Multi-repo tab bar with drag-resizable sidebar, detail, diff, and commit
  panels
- Toolbar with fetch, pull, push, branch, stash, create PR, refresh, settings,
  search, file explorer, and terminal actions
- Status bar showing branch, ahead/behind counts, staged/unstaged/stash
  counts, repository path, and active operation status
- Command palette (Ctrl+P) with context-aware commands
- Keyboard shortcuts help overlay
- Animated splash screen with skip-on-input
- Catppuccin Mocha (default), Catppuccin Latte, and One Dark themes
- JSON theme loader for user themes
- Toast notifications and confirmation dialogs for destructive actions

#### Integrations
- GitHub device-flow authentication
- Create pull request flow
- Issues and PRs fetched via GitHub API with 60-second TTL caching
- AI commit message generation via Google Gemini (optional)
- Filesystem watcher for external repo changes

#### Platform support
- Windows x86_64 zip archive and Inno Setup installer (adds rgitui to PATH
  optionally, integrates with Add/Remove Programs)
- Linux x86_64 AppImage and tarball
- macOS x86_64 and aarch64 (Apple Silicon) DMG

#### Performance
- Pre-computed trigonometric tables for commit graph edge rendering
- Per-frame memoization of `Utc::now()` for relative timestamps
- LRU caching for styled diff rows, blame, file history, and avatars
- Parallelized diff stat batching and stash / worktree enumeration
- Background git operations via GPUI's background executor to keep the UI
  thread responsive
- Diff prefetching (±25 commits, 200-entry cache) for instant navigation

#### Developer experience
- CI pipeline for Windows and Linux running fmt, clippy, tests, and release
  builds
- Automated release workflow that builds Windows (zip + installer), Linux
  (AppImage + tarball), macOS (x86_64 + aarch64 DMG), computes SHA256 sums,
  and attaches release notes from this CHANGELOG
- Background update checker that notifies when a newer release is available;
  can be disabled in Settings

### Known limitations

- Windows and macOS binaries are not yet code-signed, so SmartScreen and
  Gatekeeper will warn on first run. Choose "Run anyway" / right-click → Open
  to launch. Signing is tracked for a future release.
- Updates are announced in-app but not applied automatically; follow the link
  in the notification to download the new version.
- Only x86_64 Windows and Linux, and x86_64/aarch64 macOS are built by CI.
  Other architectures can be compiled locally with `cargo build --release`.

[Unreleased]: https://github.com/noahbclarkson/rgitui/compare/v0.1.8...HEAD
[0.1.8]: https://github.com/noahbclarkson/rgitui/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/noahbclarkson/rgitui/compare/v0.1.6...v0.1.7
[0.1.2]: https://github.com/noahbclarkson/rgitui/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/noahbclarkson/rgitui/releases/tag/v0.1.0
