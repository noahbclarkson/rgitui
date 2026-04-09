# MEMORY.md - Curated Long-Term Memory

## Architecture Notes
- `origin/main` = `3686cfd`. All major GitKraken features implemented (Phase 5).
- Code is structurally sound and tests are green.
- **Branch Author Filtering:** `BranchInfo` now computes `author_email` directly during refresh. This allows local filtering of the sidebar branch list without expensive remote git ops.

## Current Unvalidated Risks
- Large repo test
- Startup profiling
- macOS text system interaction with embedded fonts

## P1 Backlog
- bisect UI progress tracking (shipped in 10e5012)
- PR review/create (GitHub API client exists, UI not wired)
- issue search remain open

## Competitive
- Git health dashboard missing
- Custom theme editor missing

## Recent Architecture Additions
- **Commit Author Filtering:** The `GraphView` now supports a "My Commits" toggle which passes an `--author` flag directly to the `git log` subprocess during `load_more_commits_from_repo`. This pushes the filtering down to `git` itself (which can use commit-graph acceleration), rather than attempting to filter in-memory after loading thousands of commits.
