# PLAN.md — rgitui Roadmap

## Build Status
- **Tests:** 6/6 passing, 1 ignored (binary launch)
- **Clippy:** clean (no warnings)
- **Release build:** clean (~2m08s)
- **Last release:** v0.1.1 (2026-04-08) — deferred ahead/behind, double-click checkout, graph header fixes
- **Current: `29ea11d` + `docs: bisect note` — cargo test + clippy + fmt clean
- **CI:** green on `c5ed151`

---

## P0 — Stability & Polish (Before v0.2)

### Bugs / Crash Fixes
- [x] **Crash recovery edge case:** ✅ workspace snapshot restored but state may be inconsistent
- [x] **Graph row click:** ✅ occasionally selects wrong commit when scrolled far down (suspect off-by-one in index calculation)
- [x] **Rebase conflict:** ✅ interactive rebase shows conflict but cancel/abort doesn't fully restore state

### UX Polish
- [x] **Keyboard shortcut overlay:** ✅ implemented. Press `?` → `ShortcutsHelp` modal with 4 categories (~24 shortcuts). Exists at `crates/rgitui_workspace/src/shortcuts_help.rs`.
- [x] **Command palette fuzzy search:** ✅ implemented. `fuzzy_score` function (char-by-char sequential match with position-weighted scoring) added in `9ca61f2`. Tested with edge cases in `a17dc62`. Already shipped in v0.1.0.
- [x] **Graph column resize:** ✅ user can drag-resize the author and date columns in the graph view.
- [x] **Toast notifications:** ✅ `GitOperationState::Failed` path shows `ToastKind::Error` with full failure message (summary + details). Shipped — working as designed.

---

## P1 — Feature Parity

### Git Operations
- [x] **Reflog view:** ✅ wired up via command palette ("View: Reflog") and bottom panel mode switcher. Available since earlier releases.
- [ ] **Git bisect UI:** logic exists (need dedicated panel) (events fire) but no dedicated panel — need to show progress
- [x] **Stash pop --index:** ✅ per-index pop button added to sidebar stash items. `SidebarEvent::StashPop(usize)` wired to `proj.stash_pop(index, cx)`.
- [x] **Contained in:** ✅ Commit detail panel now shows local branches that contain the selected commit (using `git2` merge-base check in a background task).

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
| Keyboard shortcut overlay | ✅ | ✅ | ✅ | ✅ |
| Fuzzy command palette | ✅ | ✅ | ✅ | ✅ |
| Resizable columns | ✅ | ✅ | ✅ | ✅ |
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