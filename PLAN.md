# PLAN.md — rgitui Roadmap

## Build Status
- **Tests:** 6/6 passing, 1 ignored (binary launch)
- **Clippy:** clean (no warnings)
- **Release build:** clean (~2m08s)
- **Last release:** v0.1.0 (2026-04-08) — first public release

---

## P0 — Stability & Polish (Before v0.2)

### Bugs / Crash Fixes
- [ ] **Crash recovery edge case:** workspace snapshot restored but state may be inconsistent
- [ ] **Graph row click:** occasionally selects wrong commit when scrolled far down (suspect off-by-one in index calculation)
- [ ] **Rebase conflict:** interactive rebase shows conflict but cancel/abort doesn't fully restore state

### UX Polish
- [ ] **Keyboard shortcut overlay:** press `?` to show all shortcuts (GitKraken-style) — major quality-of-life
- [ ] **Command palette fuzzy search:** current implementation is exact match — fuzzy would improve discoverability
- [ ] **Graph column resize:** user should be able to drag-resize the graph/author/date columns
- [ ] **Toast notifications:** failed operations (push, pull, fetch) should show descriptive error toasts, not just status bar

---

## P1 — Feature Parity

### Git Operations
- [ ] **Reflog view:** exists per CHANGELOG but not accessible from UI — wire up the panel
- [ ] **Git bisect UI:** logic exists (events fire) but no dedicated panel — need to show progress
- [ ] **Stash pop --index:** apply specific stash index, not just top
- [ ] **Worktree delete:** can create worktrees, but can't delete them from UI

### GitHub Integration
- [ ] **PR review:** browse PR diff and comments in-app, not just list
- [ ] **PR create:** existing per CHANGELOG, but needs testing against real repos
- [ ] **Issue search:** full-text search within issues, not just title filter

---

## P2 — Performance & Scale

- [ ] **Large repo test:** try rgitui on a repo with 50k+ commits — may need virtualized graph lanes
- [ ] **Search performance:** commit search loads more commits when no matches found — this could be expensive on large repos
- [ ] **Diff rendering:** syntect syntax highlighting on very large files (10k+ lines) may stutter
- [ ] **Startup time:** measure cold start, profile the splash screen phase

---

## P3 — Missing from Competitive Set

| Feature | GitKraken | Lazygit | Tower | rgitui |
|---------|-----------|---------|-------|--------|
| Keyboard shortcut overlay | ✅ | ✅ | ✅ | ❌ |
| Fuzzy command palette | ✅ | ✅ | ✅ | ❌ (exact match) |
| Resizable columns | ✅ | ✅ | ✅ | ❌ |
| Submodule depth view | ✅ | partial | ✅ | ⚠️ basic |
| Git health dashboard | ✅ | ❌ | ✅ | ❌ |
| Partial hunk staging UX | ✅ | ✅ | ✅ | ⚠️ exists, needs polish |
| Custom theme editor | ✅ | ❌ | ✅ | ❌ |

---

## Notes
- v0.1.0 was the first public release (2026-04-08)
- GPUI (Zed's framework) is the rendering backbone — dependency on Zed ecosystem
- Windows AppImage replaced with `.msi` in CI per 7b4d2f7 — need to verify Windows build
- macOS DMG now has Applications symlink (02ed6b3) for drag-to-install