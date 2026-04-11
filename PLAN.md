# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus (2026-04-10 23:30 UTC)

**Build:** ✅ `6468aad` builds clean, all 592 tests pass, clippy 0 warnings.

**Shipped:** `04648d2` — feat(keyboard): Alt+5/6/7 shortcuts for Issues, PRs, Branch Health panels.
- Alt+5 → Issues (auto-fetches on first open)
- Alt+6 → Pull Requests (auto-fetches on first open)
- Alt+7 → Branch Health (auto-refreshes)
- All registered in command palette and shortcuts help.

**Cleanup:** `38f9395` — removed leftover `.patch`/`.orig` artifacts from avatar_cache merge.

**Released:** v0.1.5 — includes Push All/Pull All, Alt+5/6/7 shortcuts, artifact cleanup.

**Issues:**
- **#7** (Gatekeeper) — OPEN. Wontfix, OS-level.

**Next:** Theme editor rewrite (needs GPUI API audit), PR review/create UI wiring, startup profiling.

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

---

## Release Tracker

- **v0.1.5** — Released! (Alt+5/6/7 shortcuts, Push All/Pull All, artifact cleanup)
- **v0.1.4** — Released! (fixes #9 macOS text rendering, includes GPG badge).
