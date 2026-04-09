# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus

**This cycle (2026-04-09 21:11 UTC):**

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `5c2ea27` — feat(sidebar): wire up 'My Branches' filter toggle. The sidebar now correctly filters local branches based on the user's `user.email` from git config when the user clicks the "My Branches" filter icon.

**Issues:** #9 (investigating macOS font rendering — waiting on user terminal output), #7 (wontfix).

**Next:** Add `--author` filter to the commit graph query so users can filter commits down to their own.

---

**Previous cycles:**
See MEMORY.md and daily logs for historical records. All core P0 bugs resolved.

---

## Backlog

### Features
1. **Filter to my branches/commits** — toggle "only branches where I'm HEAD" (Done) and "only my commits" (--author filter)
2. **Drag commits in history** → quick rebase (extends existing interactive rebase drag)
3. **Worktree change graph** — pending changes as nodes across all worktrees in graph view
4. **Loading UX** — blank screen → animated splash/progress indicator during initial load

### Performance
5. **5s Linux load investigation** — verify preload is actually working, profile cache behavior

### Issues
- **#9** (macOS font rendering) — investigating, waiting on user terminal output
