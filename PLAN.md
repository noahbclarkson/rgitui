# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus (2026-04-10 07:21 UTC)

**Build:** ✅ `4ee179f` builds clean, all tests pass, clippy 0 warnings.

**Shipped:** `4ee179f` — fix(avatar): stop treating random strings as GitHub usernames.
The app was extracting the local-part of git emails or commit author names and hitting `github.com/name.png`. If it accidentally matched a username, it loaded their avatar; if not, GitHub often redirects `.png` to a default placeholder. This has been removed. Gravatar, exact noreply emails, and the official GitHub API search remain in place.

**Issues:**
- **#10** (Placeholder/random avatars) — FIXED and closed.
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

### Issues
- **#9** (macOS text missing) — FIXED in `e5d6285` (font-kit + Lilex fallback font). Noah confirms.
- **#7** (Gatekeeper) — OS-level, not code.

---

## Release Tracker

- **v0.1.3** — Released! (fixes #9 macOS text rendering, includes GPG badge).
