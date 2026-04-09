# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus

**This cycle (2026-04-09 19:40 UTC):** `origin/main` = `3686cfd`.

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `3686cfd` — feat(sidebar): add filter to only show my branches. Updated `BranchInfo` to compute author emails, laying the foundation for "My Branches" and "My Commits" filters in the sidebar and commit graph.

**Issues:** #9 (investigating macOS font rendering — waiting on user terminal output), #7 (wontfix).

**Next:** Finish the "Filter to my branches/commits" feature by wiring up the UI toggle in the sidebar and adding an `--author` filter to the commit graph query.

---

**Previous cycles:**
See MEMORY.md and daily logs for historical records. All core P0 bugs resolved.

---

## Backlog

### Features
1. **Filter to my branches/commits** — toggle "only branches where I'm HEAD" and "only my commits" (--author filter)
2. **Drag commits in history** → quick rebase (extends existing interactive rebase drag)
3. **Worktree change graph** — pending changes as nodes across all worktrees in graph view
4. **Loading UX** — blank screen → animated splash/progress indicator during initial load

### Performance
5. **5s Linux load investigation** — verify preload is actually working, profile cache behavior

### Visual
6. ~~Discard dialog padding~~ ✅ DONE `ce988c5`

### Audits
7. ~~Comprehensive audit~~ ✅ DONE 2026-04-09
- **Security:** Clean — keychain for secrets, no hardcoded tokens, no unsafe in hot paths
- **Bugs:** Clean — all `unwrap` have fallbacks, `panic!` only in test code
- **Performance:** Clean — git ops on background executor, no N+1 patterns
- **Visual:** Clean — confirmed action buttons have proper padding via row/container level px()

### Issues
- **#9** (macOS font rendering) — investigating, waiting on user terminal output
