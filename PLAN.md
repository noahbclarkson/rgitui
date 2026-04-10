# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus (2026-04-10 04:50 UTC)

**Build:** ✅ `19cff25` builds clean, 337+ tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `19cff25` — feat(rebase): add Interactive Rebase to graph context menu.
The interactive rebase UI and git backend logic were implemented but inaccessible from the graph view. Right-clicking a commit now correctly calculates the range from HEAD to the selected commit and triggers the overlay.

**Issues:**
- **#9** (macOS text missing) — CLOSED. Was fixed in v0.1.3.

**Next:** Drag commits in history to initiate interactive rebase.

---

## Backlog

### Features
1. ✅ **Filter to my branches/commits** — done
2. **Drag commits in history** → quick rebase (initiate interactive rebase by dragging in graph)
3. ✅ **Interactive Rebase Context Menu** — done
4. **Worktree change graph** — pending changes as nodes across all worktrees in graph view
5. ✅ **Loading UX** — animated splash screen already implemented and working

### Performance
5. **5s Linux load investigation** — verify preload is actually working, profile cache behavior

### Issues
- **#9** (macOS text missing) — FIXED in `e5d6285` (font-kit + Lilex fallback font). Noah confirms.
- **#7** (Gatekeeper) — OS-level, not code.

---

## Release Tracker

- **v0.1.3** — Released! (fixes #9 macOS text rendering, includes GPG badge).
