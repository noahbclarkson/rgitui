# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus

**This cycle (2026-04-09 21:11 UTC):**

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `1c5fdfa` — feat(graph): add filter to only show my commits. The commit graph now has a "My Commits" toggle (user icon) in the header. When clicked, it adds an `--author` filter to the underlying `git log` command so users can quickly see just their own history.

**Issues:** #9 (investigating macOS font rendering — waiting on user terminal output), #7 (wontfix).

**Next:** Look into allowing users to drag commits in the history for a quick interactive rebase.

---

**Previous cycles:**
See MEMORY.md and daily logs for historical records. All core P0 bugs resolved.

---

## Backlog

### Features
1. **Filter to my branches/commits** — toggle "only branches where I'm HEAD" and "only my commits" (DONE)
2. **Drag commits in history** → quick rebase (extends existing interactive rebase drag)
3. **Worktree change graph** — pending changes as nodes across all worktrees in graph view
4. **Loading UX** — blank screen → animated splash/progress indicator during initial load

### Performance
5. **5s Linux load investigation** — verify preload is actually working, profile cache behavior

### Issues
- **#9** (macOS font rendering) — investigating, waiting on user terminal output
