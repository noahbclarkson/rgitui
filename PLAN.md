# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus (2026-04-10 08:05 UTC)

**Build:** ✅ `bdca9ba` builds clean, all tests pass, clippy 0 warnings.

**Shipped:** `bdca9ba` — feat(health): add branch health dashboard showing stale, diverged, unmerged branches.
Added a new `BranchHealthPanel` in the right tab group to help users quickly clean up their repository. It computes `last_commit_time` and `is_merged_into_main` in the background git worker during branch refresh. Filters allow users to view:
- **Unmerged**: Local branches not yet merged into `main`/`master`.
- **Stale**: Branches with no new commits in 30+ days.
- **Diverged**: Branches that have commits ahead AND behind their upstream.

**Issues:**
- **#7** (Gatekeeper) — OPEN. Wontfix, OS-level.

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

---

## Release Tracker

- **v0.1.3** — Released! (fixes #9 macOS text rendering, includes GPG badge).
