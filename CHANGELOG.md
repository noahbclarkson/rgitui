# Changelog

All notable changes to rgitui are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/noahbclarkson/rgitui/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/noahbclarkson/rgitui/releases/tag/v0.1.0
