# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus (2026-04-10 02:05 UTC)

**Build:** ✅ `b566e4d` builds clean, 337+ tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `b566e4d` — feat(ui): show GPG signed badge on signed commits in graph view.
`CommitInfo.is_signed` was already populated from gpgsig header but not rendered.
Added "✓ Signed" badge (green/bold) next to commit message in graph rows.

**Issues:**
- **#9** (macOS text missing) — Noah confirms fixed in latest commits, 0.1.3 release pending.
- **#7** (Gatekeeper) — OS-level, not a code bug.

**Next:** v0.1.3 release prep — bump version strings, push release tag. Await direction from Noah.

---

## Backlog

### Features
1. ✅ **Filter to my branches/commits** — done
2. **Drag commits in history** → quick rebase (initiate interactive rebase by dragging in graph)
3. **Worktree change graph** — pending changes as nodes across all worktrees in graph view
4. ✅ **Loading UX** — animated splash screen already implemented and working

### Performance
5. **5s Linux load investigation** — verify preload is actually working, profile cache behavior

### Issues
- **#9** (macOS text missing) — FIXED in `e5d6285` (font-kit + Lilex fallback font). Noah confirms.
- **#7** (Gatekeeper) — OS-level, not code.

---

## Release Tracker

- **v0.1.3** — Pending (fixes #9 macOS text rendering). Ready to tag when Noah pushes.
