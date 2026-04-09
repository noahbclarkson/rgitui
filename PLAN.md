# PLAN.md - Forge's Autonomous Work Plan

_Updated by Forge each cron run. Arc reads this to sync._

---

## Current Focus

**This cycle (2026-04-09 19:36 UTC):** `origin/main` = `ce988c5`.

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean.

**Shipped:** `ce988c5` — fix(ui): add right padding to confirm dialog action buttons. Discard/Clean/Delete action buttons now have `.pr(px(4.))` so they are not flush against the modal edge.

**Issues:** #9 (investigating macOS font rendering — waiting on user terminal output), #7 (wontfix).

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean. Pushed `96ff458`.

**Shipped:** Fixed broken build from `entity_by_id` gpui API mismatch. The subscribe callback was trying to use `cx.entity_by_id(input_id)` to look up a `TextInput` by ID — that method doesn't exist in this Zed version. Replaced with direct entity capture in closure. Also fixed `this.update()` result not being dropped and removed unused import.

**Issues:** #9 (investigating, waiting on user feedback), #7 (wontfix). CI running on `96ff458`.

**Build:** ✅ All tests pass, clippy 0 warnings, fmt clean. CI green.

**Shipped:** Investigated font loading to diagnose Issue #9 (missing text on Mac). Verified that the 12 embedded `.ttf` files are correctly packaged and accessible via `Assets::list` and `Assets::load`. The issue is likely upstream in how the macOS text system handles the provided bytes, not a failure to bundle the fonts. Waiting on the user's terminal output from the new diagnostics we added in `57a915f`. 

**Issues:** #9 (investigating), #7 (wontfix macOS Gatekeeper).

**Next:** Need Noah's input on P1 backlog or wait for user log output for #9.

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
7. **Comprehensive audit** — security, bugs, performance, visual issues across full codebase

### Already Implemented (confirm before treating as backlog)
- Stage/unstage all ✅ (Ctrl+S / Ctrl+Shift+S / Ctrl+U / command palette / sidebar buttons)
- Avatar LRU cache ✅ (in-memory + disk persistence, 2000-entry cap)
- Double-click checkout ✅
- Branches containing commit ("Contained in") ✅
- macOS experimental note in README ✅
