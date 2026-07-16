## [Unreleased]

### Added

- **DeepSeek AI provider.** Settings now offers DeepSeek with the current
  `deepseek-v4-flash` and `deepseek-v4-pro` models, including repository tool
  calls through DeepSeek's OpenAI-compatible chat API.
- **Merged-branch indicators.** Local branches already reachable from the
  current branch now show a green merge icon, matching `git branch --merged`.
- **Untracked-file filter.** The `?N` count in the Unstaged header is clickable,
  making it easy to hide or restore a large untracked-file backdrop per session.

### Changed

- **Tracked changes use semantic filename colors** while untracked files are
  dimmed, so active modifications stand out without widening the sidebar.

### Fixed

- Auth Preferences no longer collapses horizontally in the settings window.
- Hunk staging now emits complete patches, and hunk unstaging correctly reverses
  the selected change instead of applying the staged direction again.
- Completing a clone opens and refreshes the cloned repository in its own tab;
  it no longer replaces the graph data inside the previously active project tab.

## [0.3.0] - 2026-06-02

A large correctness, safety, and quality release. It closes a cluster of
**data-loss bugs** in the merge / rebase / stash / undo paths, makes
three-way conflict diffs and partial (hunk/line) staging actually work,
adds keyboard accessibility and focus handling across the component library
and every dialog, and removes the dominant per-operation and per-frame
performance waste. Driven by a full engineering audit of the codebase.

> **Upgrade note:** several fixes here change behaviour you may have been
> working around — merge commits now keep both parents, undo no longer
> applies to the wrong repository, and partial line-staging now actually
> stages. If you scripted around any of these, re-check your flow.

### Added

- **Stash with a message.** The toolbar **Stash** button and `Ctrl+Z` now
  open a dialog for an optional stash message instead of stashing
  immediately; an empty message falls back to the default `WIP` stash. The
  message field is focused on open and its length is validated by character
  count (not bytes), so multibyte messages aren't mis-measured.
- **Partial-staging mode indicator.** A **Partial** badge appears next to the
  diff-mode toggle while line-selection staging mode is active (toggled with
  `p`), and the `p` shortcut is now documented in the keyboard-shortcuts help
  overlay.
- **Linux window controls.** On Linux desktops using client-side decorations
  (e.g. GNOME / Wayland) the custom title bar now draws its own
  minimize / maximize / close controls; the close button calls
  `remove_window()` (previously it dispatched an action that was never wired,
  so it did nothing). macOS and Windows are unaffected.

### Fixed

#### Data integrity (merge / rebase / stash / undo)

- **Merge commits dropped their second parent.** Committing during a merge
  wrote an ordinary single-parent commit and left the repository stuck in the
  "Merging" state. It now finalizes a real two-parent merge commit from
  `MERGE_HEAD` and clears the merge state.
- **Amending during a rebase silently aborted the rebase.** It is now refused
  with a clear error instead of throwing away the in-progress rebase.
- **"Continue merge" swept untracked files into the commit.** It ran
  `git add -A` semantics, pulling unrelated untracked/unstaged files into the
  merge commit. It now only finalizes a genuine merge (requires `MERGE_HEAD`)
  and no longer mass-stages.
- **Interactive rebase could corrupt history.** A stale or cross-branch plan
  could be replayed onto the wrong base. The plan is now validated against
  HEAD's real first-parent range, with the base derived from it, and a
  mismatching plan is refused rather than applied.
- **"Stash → branch" branched at the wrong commit.** It created the branch at
  the stash's WIP commit and never checked it out. It now creates the branch
  at the stash's base commit, checks it out, applies the stash, and drops it.
- **Conflicting `stash pop` / `stash apply` failed silently.** A conflict left
  the tree changed but reported nothing. It now refreshes and surfaces the
  conflict so you can resolve it.
- **Undo could run against the wrong repository.** With multiple repository
  tabs open, performing an action in tab A, switching to tab B, then pressing
  Undo ran the reversal against B — recreating a branch or resetting B to a
  commit from A. Undo now routes to the tab the action was performed on (and
  switches to it); if that tab has been closed it refuses with a notice
  instead of touching the wrong repository.
- **Undo of a worktree commit reset the main repository.** Undoing a commit
  made while inspecting a linked worktree soft-reset the main repo to an OID
  taken from the main repo's log. It now soft-resets the worktree's own branch
  using that worktree's real pre-commit HEAD.

#### Diffs, conflicts, and partial staging

- **Three-way conflict diffs never rendered.** Selecting a conflicted file
  showed the empty "Select a file" placeholder instead of the conflict view.
  Conflicts now render, and switching back to a normal file clears the stale
  conflict rows.
- **Partial line-staging never applied.** The patches generated for single-line
  stage/unstage lacked a `diff --git` header, so libgit2 rejected every one of
  them — the feature silently did nothing in shipped builds. Patches now carry
  the header and apply correctly.
- **Hunk/line staging ignored the inspected worktree.** Staging a hunk or line
  from the diff viewer always targeted the main repository's index; while
  inspecting a linked worktree it now stages into that worktree's index.
- **Deletions could not be partially staged.** Deleted lines are now selectable
  for partial stage/unstage, and end-of-file-newline markers are preserved in
  the generated patches.
- **Diff viewer robustness.** The highlighted row and scroll-to-line are
  clamped against out-of-bounds positions; wrap-mode scroll position is
  preserved across refreshes; split and three-way views wrap into columns
  instead of clipping horizontally; and the viewer gains a focus ring,
  shortcut hints, and explicit loading/error states.

#### GitHub (Issues & PRs)

- **Panels showed "No GitHub remote configured" until Settings was opened.**
  They were configured at tab-open time, before the repository's remotes
  finished loading asynchronously, and were never reconfigured. They now
  reconfigure when a refresh delivers the remote, picking up the owner/repo
  automatically; configuration is idempotent and clears the stale error once a
  valid remote is known.
- **Richer, more robust panels.** PR and Issue lists now render label colors
  and markdown bodies/comments and support keyboard navigation; a failed
  comment fetch is scoped to the detail view instead of replacing the whole
  panel with an error; and a newly created PR appears immediately rather than
  waiting on the 60-second cache.

#### Accessibility, dialogs, and UI

- **Keyboard and focus support across components.** Buttons, checkboxes, and
  disclosures gain real disabled states, focus rings, and keyboard activation;
  the text input gains disabled/read-only modes, a focus tint, a working
  font-size setter, and multibyte-safe cursor handling, and **Tab** no longer
  submits the field.
- **Dialogs are keyboard-usable on open.** The branch, tag, rename, stash,
  stash-branch, confirm, and clone dialogs focus their input on open; the
  create-PR description field is multiline; and the theme editor is responsive,
  Tab-traversable, and surfaces invalid-hex feedback.
- **Clone dialog gives feedback.** Submitting a clone previously hid the dialog
  immediately with no progress or result. It now stays open in a "Cloning…"
  state, then closes on success (with the repository loaded) or shows the error
  inline on failure so you can fix the URL/path and retry.
- **Tabs and toasts.** A tab's close button no longer activates the tab it
  closes and has a larger hit target; toast severity is no longer conveyed by
  color alone (a severity label and accent bar are added) and toasts can be
  dismissed.
- **Settings window.** The Accounts & Credentials section fills the window and
  resets its scroll position when switching sections; the feedback banner is
  dismissible, auto-clearing, and moved out of the scroll area; a **Save**
  button commits pending edits; the device-flow login can be cancelled and
  honors expiry; and detected-tool lists scroll.
- **Safer text handling.** Commit-message copy is now character-safe (it
  previously sliced on a byte index and could panic on multibyte messages),
  AI diff/context truncation is UTF-8-safe, and a corrupt settings file is
  quarantined rather than crashing the app on load.
- **Sidebar correctness.** Keyboard highlight maps to the correct
  (virtualized) row, file sections get navigation entries, worktrees are
  virtualized, and several panels no longer call `refresh()` from inside
  `render()` (which caused a notify storm).

### Changed

- **Scrollbar drag no longer resets** mid-drag (the drag state is persisted
  across re-renders).
- **Layout.** Panel-resize bounds are now unified between dragging the handle
  and keyboard resizing; the welcome screen scrolls; and the center pane keeps
  a minimum width on narrow windows.
- **Theming.** Placeholder text now meets WCAG-AAA contrast, and the dead
  JSON theme loader was removed so themes have a single source of truth.

### Performance

- **Staging no longer triggers a full repository scan.** Stage, unstage,
  discard, and hunk/line operations previously ran the full repo-wide refresh
  — ahead/behind graph walks for every branch plus a fresh `git log` — even
  though none of those can change from staging. They now use a lightweight,
  per-worktree-cached refresh and recompute ahead/behind off the critical path.
- **Less per-frame and per-operation waste.** Graph ancestry is memoized
  (previously O(n²) per render); commit and prefix vectors are shared via
  `Arc` instead of cloned; the detail and sidebar file lists are virtualized
  and no longer clone every row each frame; the branch-filter recompute is
  cached; an O(n²) hunk scan was removed; and the diff viewer only computes its
  longest row when it actually needs it.
- **A coherent refresh/watcher spine.** Commits are ordered topologically;
  pagination is cursor-based so no commit is dropped when refs change between
  pages; the "My Commits" author filter is preserved across watcher- and
  operation-driven refreshes; the watcher no longer zeroes branch ahead/behind
  on every event; a refresh-generation guard drops stale background refreshes;
  and `.git` change detection is content-aware.

### Internal

- Expanded unit-test coverage for the authentication helpers and toolbar
  events.

## [0.2.2] - 2026-05-25

A UI polish and stability release: a crash when opening the theme editor is
fixed, the Settings pages scroll again, and the left sidebar and welcome screen
get layout corrections.

### Fixed

- **Crash when opening the theme editor.** Clicking **Edit Theme** in Settings
  closed the Settings window while the workspace was mid-update, causing a
  reentrant-update panic that aborted the app. The window-close cleanup is now
  deferred until the in-progress update finishes, so the editor opens cleanly.
- **Settings pages could not be scrolled.** The content was vertically centered
  with no bounded height, so longer pages were clipped and the scroll wheel did
  nothing. The settings area now fills the window height and scrolls from the top
  as expected.
- **Theme editor had square corners.** The header and footer bars painted square
  corners over the modal's rounded edges; their outer corners now follow the
  12px radius so the dialog is uniformly rounded.
- **Worktree count hidden in the sidebar.** At the default width the worktree
  count beside **New Worktree** was pushed off-screen. The count is now pinned
  and the button yields space first.

### Changed

- **Sidebar sizing and collapsing.** The default sidebar width is ~15% wider, a
  minimum width is enforced (both when dragging the resize handle and when
  loading a saved width), and branch rows with ahead/behind (`+x -y`) badges now
  truncate and align identically to rows without them.
- **Recent repositories and workspaces on the welcome screen.** Each list now
  shows the five most recent entries with more vertical spacing between rows.


## [0.2.1] - 2026-05-20

### Fixed

- **Settings: Auth page layout broken on Windows 11.** The Auth page was misaligned in the normal window state and only rendered correctly after maximizing the window. Removed a conflicting `.h_full()` CSS property from the settings-page-shell container.

### Changed

- **Rebase: ghost preview for drag-to-reorder.** Visual ghost element follows the cursor when dragging commits to reorder them in the rebase todo list.
- **Diff: clear detail panel on working-tree refresh.** Detail panel properly cleared when working-tree diff refreshes, preventing stale content.
- **Detail panel: Flat/Tree file view toggle.** Folder/File icon button in the toolbar toggles between flat and tree file views.

### Added

- **Rebase: drop indicator line.** Accent-colored drop indicator line appears at the target position during drag-to-reorder.


## [0.2.1] - 2026-05-20

A reliability release centered on **GitHub authentication** and the **diff
viewer**. GitHub tokens now persist across restarts, public repositories no
longer require a token to browse issues and pull requests, and a cluster of
diff-viewer/file-watcher state bugs are fixed. It also lands a real directory
tree for the changed-files list and a drag-to-reorder preview in interactive
rebase.

### Added

- **Flat/tree toggle for the changed-files list** in the detail panel. Switch
  between a flat list (full paths) and a real directory tree with collapsible
  folders and single-child-chain compaction (e.g. `crates/foo/src` renders as one
  node). Press `v` or use the toolbar button; the toggle is disabled during file
  search or when there are no changes. (#24)
- **Ghost preview when reordering commits** in the interactive rebase modal — a
  floating, theme-matched copy of the dragged row tracks the cursor. (#37)

### Fixed

- **GitHub tokens persist on first connect.** A newly added provider — including
  the first device-flow sign-in — had its token silently dropped on save while
  Settings still reported "Connected", so Issues/PRs showed "GitHub token
  required" and it never survived a restart. Tokens are now written to the OS
  keyring unconditionally, the auth runtime is resolved from the keyring on every
  startup, and the Settings "Connected" badge reflects the actually-resolved
  token rather than a stale flag.
- **Public repositories no longer require a token** to view issues and pull
  requests. The panels now fetch unauthenticated when no token is configured and
  only prompt for sign-in on a genuine authentication/visibility failure; that
  prompt notes organization repos may need a fine-grained PAT approved by an org
  owner.
- **Diff content is no longer wiped by unrelated repository changes.** The
  working-tree refresh is gated to working-tree diffs (commit and three-way diffs
  are left intact) and ignores gitignored paths, and a generation guard prevents a
  slow background refresh from clobbering a newer selection or its detail panel.
- **Stale diff previews after an in-place edit.** The diff viewer compares hunk
  content, not just hunk/line counts, so a same-shape edit is detected; it clears
  when a displayed working-tree file loses all its changes (e.g. after discard).
- **The wrap-mode diff scrollbar keeps a constant thumb size** while scrolling
  instead of resizing as rows are measured, and stays grabbable.
- **Background refreshes no longer truncate commits paged in via "load more."**
- Removed `#[allow(clippy::too_many_arguments)]` attributes in favour of params
  structs, and made GitHub provider host-matching case-insensitive.

## [0.2.0] - 2026-05-12

This release moves the **Settings UI into its own OS window**, adds **clone-from-URL**
support with a folder picker, ships a new **Cream & Blue** light theme, and delivers
a wave of polish across the command palette, dialogs, and graph view. A new
per-worktree refresh cache cuts background CPU during file-system churn.

### Added

- **Clone repositories from a URL.** The Repo Opener now has a **Clone** button
  that opens a dedicated dialog with URL and Path fields. The dialog includes a
  **Browse** button that opens an OS folder picker and auto-appends the repo
  name parsed from the URL (so picking `~/Github` with
  `https://github.com/user/repo.git` fills `~/Github/repo`). Clone tries
  in-process `git2` first and transparently falls back to system `git` with the
  configured HTTPS credentials. Resolves #32.
- **Settings now opens in a dedicated OS window** (`Ctrl+,`), not an overlay.
  Window position and size are persisted across sessions, and the workspace and
  settings windows talk over a cross-window action channel so theme changes and
  settings updates apply live.
- **Cream & Blue light theme.** A warm near-paper light theme with cool-grey
  element layers and a Xero-style cyan-blue accent, intended as a more formal
  alternative to Catppuccin Latte.
- **Detail-panel view-mode toggle button.** A Folder/File icon in the detail
  panel toolbar toggles between Flat and Tree file views, alongside the
  existing `v` keyboard shortcut. The `v` shortcut is now documented in the
  Navigation section of the in-app shortcuts help.
- **Inline descriptions in the command palette.** Every command now shows a
  short description next to its label. Long descriptions truncate with an
  ellipsis and a hover tooltip shows the full text. The palette itself is
  wider (480 → 720 px) so descriptions fit on common displays.
- **Debian packaging.** The release pipeline now builds a `.deb` alongside the
  AppImage / tarball, with the necessary `.desktop` and AppStream metadata.

### Changed

- **Command palette redesign.** Widened to 720 px, the heavy focused-blue
  header underline replaced with a muted separator (the input renders its own
  focus outline), and the navigate / select / dismiss footer switched from a
  fixed 32 px row to padding-driven sizing so the labels aren't clipped at
  small font metrics. Inter-label gap and modal max-height tuned to match.
- **Settings secret fields commit on Enter, not on every keystroke.** The 10
  fields that round-trip through the OS keychain (GitHub token, AI API keys,
  Git provider tokens) no longer write to the keychain per character. Live UI
  still updates as you type; the actual save happens on submit.
- **Create-PR base branch defaults to the detected default branch.** Reads
  `refs/remotes/origin/HEAD` and falls back to `main` only when unavailable, so
  repositories whose default branch is `master` (or anything else) no longer
  get a stuck "main" placeholder.
- **Fuzzy file-search results are sorted by relevance.** The Ctrl+P-style fuzzy
  filter previously returned matches in repo order; it now ranks by fuzzy
  score, surfacing tighter character-position matches first in both flat and
  tree views.
- **CI is pinned to Rust 1.94.1** (matching `rust-toolchain.toml`) so a stable
  bump no longer fails fmt checks on otherwise-green commits.

### Fixed

- **Diff preview now refreshes after stage / unstage / discard.** The diff
  viewer was holding the pre-action contents until you re-selected the file.
  It now subscribes to `GitProjectEvent::StatusChanged` and recomputes the
  diff for the currently displayed file when its status changes. Fixes #31.
- **Theme editor / clone / stash-branch dialogs render with a repo open.**
  These three dialogs were only attached to the no-tabs welcome layout, so
  clicking "Edit Theme" or "Clone" while a repo was open flipped their
  internal `visible` flag but rendered nothing. They are now in the tab-open
  render tree too.
- **Clone dialog is no longer hidden behind the Repo Opener.** Welcome-screen
  z-order was inverted so the clone dialog sits above the opener it was
  launched from.
- **Settings window closes when "Edit Theme" is clicked.** The theme editor
  lives in the workspace window; previously the settings window stayed on
  top, hiding the editor. Settings now closes when the editor is requested.
- **Graph column headers align with their data rows.** The right-side header
  hosts both the "my-commits" filter and the display-settings gear (52 px
  total), but the trailing row spacer was still sized for a single button,
  so Author / Date were pushed left of their columns. Hash / Message also
  ignored the per-row 16 px drag-to-rebase grip. Both spacers are now in the
  header, and the Author / Date drag-resize math is updated accordingly.
- **Graph display-settings popover no longer leaks hover** into the commit
  rows behind it. The popover only stopped left-click propagation, so
  mouse-move still reached the underlying rows.
- **Dialog buttons no longer overflow on narrow modals.** Branch / Confirm /
  Create-PR / Stash-Branch / Tag dialogs lay their button row out below the
  keybinding hint instead of squeezing it onto a single horizontal strip.
  Branch dialog gains `flex_nowrap` to prevent button truncation. Fixes #30.
- **GitHub 403 errors are now actionable.** OAuth-App access restrictions and
  SAML SSO enforcement messages are rewritten into a single readable
  sentence, and long error text wraps inside a 480 px container instead of
  pushing the layout horizontally in the Issues / Pull Requests panels.
- **GitHub OAuth device-flow polling cancels with the window.** Previously the
  polling future was detached and could run for ~15 minutes after the
  settings UI was dismissed; it is now bound to the settings entity.
- **Settings window bounds save only on close**, not on every render frame
  during a drag.
- **Splash screen top corners squared** so they align with the title bar.

### Performance

- **Per-worktree diff-stat cache.** `GitProject::refresh()` and the watcher
  loop now share a `WorktreeStatusCache` keyed on path + status flags +
  staged blob OIDs + workdir mtime/size for modified files. Watcher-triggered
  refreshes skip the expensive `batch_diff_stats()` scan when the fingerprint
  is unchanged, cutting CPU during background file-system churn while typing
  in another editor.

### Internal

- **Test coverage push.** Workspace event enums and dialog logic gained unit
  tests across the board: `BranchDialog`, `ReflogView`, `SubmoduleView`,
  `BisectView`, `Toolbar`, `ShortcutsHelp`, `SettingsView`, `RepoOpener`,
  `Workspace`, `StashBranchDialog`, `CreatePrDialog` (including PR JSON
  request/response parsing), `ConfirmDialog`, `BlameView`, `CommandPalette`
  (`CommandContext` / `PaletteCommand`), `CommitPanel` co-author trailer
  formatting, and fuzzy file-search ordering. Integration tests gained a
  merge-commit graph test and an ignored headless GPU smoke test, and were
  made robust to CI environments that lack a configured `init.defaultBranch`.

## [0.1.8] - 2026-04-28

### Added

- **Custom theme editor** (Ctrl+Shift+T / Alt+9): live color-picker UI for all
  theme fields with hex input and swatch preview. Edited themes are saved as
  JSON files to `~/.config/rgitui/themes/` and reloaded on next launch.
- **JSON theme serialization**: themes can be exported and re-imported as JSON.
  Built-in themes ship as embedded JSON; user themes are loaded from the config
  directory at startup.
- **Branch from stash**: a Branch button on each stash panel row lets you create
  a branch from any stash entry directly, without going through the command
  palette.

### Fixed

- **Theme editor black-on-save regression**: saving a theme no longer
  momentarily resets all colors to black; the editable state is properly
  resynced after save.
- **Theme editor startup popup**: opening the editor no longer flashes an empty
  or stale state on first render.
- **Invalid hex fallback in theme editor**: typing a partial hex value no longer
  silently snaps the swatch to black; the swatch holds the field's current theme
  color until a valid hex is entered.
- **Graph lane-0 gap**: lane 0 is now drawn continuously from the first
  side-branch curve-in to `main_tip`, preventing gaps when the newest commits
  are on side branches.
- **`is_merged_into_main` correctness**: merge detection now uses `merge_base`
  instead of `graph_descendant_of`, fixing false positives on non-linear
  histories.
- **Search output parser**: malformed `git grep` output no longer silently drops
  results; a simple-split fallback preserves partial matches.
- **Windows UNC paths (WSL2)**: repository paths beginning with
  `\\server\share\…` are rewritten to `//server/share/…` for libgit2
  compatibility. Extended-length (`\\?\`) and device (`\\.\`) prefixes are left
  unchanged.

## [0.1.7] - 2026-04-18

### Added

- **Dedicated Stashes panel** (Alt+8): lists every stash with its message and
  OID, with inline Apply, Pop, and Drop buttons. Dropping a stash prompts a
  confirmation dialog. A stash-count badge appears in the toolbar. The command
  palette also gains `Git: Create Branch from Stash`, visible when stashes
  exist.
- **Diff line-wrap toggle**: new "Wrap Long Lines in Diff" setting in the Diff
  settings card switches between wrapping and horizontal scrolling. Wrapping
  respects the text layouter so lines break correctly in all diff modes.
- **Diff horizontal scrollbar**: in no-wrap Unified mode, the whole list
  scrolls horizontally with a draggable scrollbar at the bottom. In Split and
  Three-way modes each column keeps its own horizontal scroll area so long
  lines no longer bleed across the column divider. The line-number gutter
  stays pinned in all modes.
- **Diff vertical scrollbar**: a draggable vertical scrollbar now sits next
  to the diff body in every mode.
- **Watch all worktrees** (General settings): when enabled, the filesystem
  watcher tracks every linked worktree, so external changes in any worktree
  trigger a refresh.

### Changed

- **Wrap-mode diff viewer is virtualized**: only the visible rows (plus a
  small overdraw buffer) are rendered per frame, and wrapped-line heights are
  measured once and cached. Large diffs stay responsive while scrolling with
  wrap enabled.
- **Stash detail summary**: stash entries in the detail panel now show a
  clean subject line. The `WIP on <branch>: <sha>` prefix is stripped, with
  graceful fallbacks to the branch name or full message for custom stash
  names.

### Fixed

- **Wrap mode no longer overlaps lines**: wrapped continuation lines used to
  paint on top of the row below because flex-shrink squeezed rows when total
  content exceeded the viewport. Wrapped text now correctly extends the row
  it belongs to.
- **Word-diff noise on reformatted hunks**: word-level highlighting paired
  deletion line *i* with addition line *i* regardless of whether the counts
  matched. On a reformat (e.g. inlining a closure body into a multi-line
  match block) the pairing lined up semantically unrelated rows and every
  coincidental token match produced a misleading highlight. Word-diff is now
  skipped when deletion and addition line counts differ, so reformats render
  as clean red/green blocks.
- **Command palette stale after stash operations**: after Apply / Pop / Drop,
  the palette's `has_stashes`, `has_changes`, and related predicates are
  rebuilt once the active tab finishes refreshing, so the next open reflects
  the actual repo state.
- **Commit graph lane artifacts with worktrees**: fixed a ghost lane-0
  column extending above `main_tip` when the newest commits were on side
  branches, a dangling stub on lane 1 at the main-tip row, and missing
  pass-through strokes across worktree virtual rows for merge-in lanes.
  Worktrees whose HEAD isn't in the visible commit list are now routed to
  orphan slots instead of being silently dropped.
- **Action button clipping in dialogs**: the Discard and Unstage buttons in
  the sidebar file list, and the action buttons in the Create PR dialog,
  were clipped against the right edge on Windows. Right padding now matches
  the rest of the dialog surface.

## [0.1.2] - 2026-04-09

### Added

- **macOS support**: Bundle embedded fonts (IBM Plex Sans, Lilex, JetBrains Mono)
  so text and icons render correctly on macOS. Ad-hoc code sign the .app bundle
  to prevent Gatekeeper "damaged" errors. Generate .icns app icon from PNGs
  during the Mac bundle step.
- **macOS CI**: Add `macos-14` to the CI test matrix alongside Linux and Windows.
- **Configurable commit limit**: New `commit_limit` setting (default 1000) lets
  users control how many commits are loaded per repo.

### Changed

- **Startup performance**: Repos now load during the splash animation instead of
  after it. Active tab refreshes first; inactive tabs load after it completes.
  Avatar disk cache loads on a background thread.
- **Commit walk uses git subprocess**: Replaced libgit2's `revwalk` with
  `git log` subprocess, leveraging commit-graph files for ~100x speedup on large
  repos (Linux kernel: 14.7s to 1.2s).
- **Two-phase commit loading**: First 100 commits load immediately so the graph
  appears fast, remaining commits load in the background.
- **Deferred commit metadata**: GPG signature checking and co-author parsing
  are skipped during the commit walk and computed on-demand when clicking a
  commit, reducing per-commit overhead.
- **Parallel status computation**: Working tree status now runs on a separate
  thread in parallel with stash enumeration and the commit walk.
- **Lightweight initial refresh**: Initial repo load skips ahead/behind
  computation for all branches; it runs in the background after the UI appears.
- **Diff computation**: Removed redundant `diff.stats()` call (~31% CPU savings)
  and intra-diff thread serialization overhead. Diff cache prewarming fires once
  per tab and only for the active tab initially.
- **Diff cache shared across workspace**: The LRU diff cache is now stored in
  `ViewCaches` and shared between project and graph subscriptions.
- **Selected commit priority**: On cache miss, the clicked commit's diff task is
  submitted to the thread pool before neighbor prefetch tasks to ensure it gets
  processed first.

### Fixed

- Resolved clippy warnings in bisect view and workspace events.
- Fixed unnecessary reference creation in events.rs and items-after-test-module
  ordering in bisect.rs.

## [0.1.0] - 2026-04-08

First public release. rgitui is a GPU-accelerated, multi-repo desktop Git
client built with [GPUI](https://github.com/zed-industries/zed). This release
establishes a feature-complete baseline for day-to-day use.

### Added

#### Core git operations
- Stage, unstage, and discard changes at file, hunk, and line granularity
- Commit and amend with a message editor, co-author support, and optional
  AI-generated commit messages (Gemini)
- Branch management: create, checkout, rename, delete, and switch from the
  sidebar or command palette
- Tag management: create annotated and lightweight tags, delete, and checkout
- Stash: save (with optional name), pop, apply, drop, and create branch from
  stash
- Remote operations: fetch, pull, push, and force push with multi-remote
  support and automatic upstream tracking
- Cherry-pick, revert, and reset (hard / soft / mixed)
- Merge with conflict detection and inline "accept ours / accept theirs"
  resolution
- Interactive rebase with pick / squash / reword / fixup / drop actions
- Bisect with start, good, bad, skip, and reset commands
- Worktrees: create, list, and switch
- Submodule initialization, update, and management
- Clean untracked files
- Undo stack for recent local git operations
- Crash recovery: workspace snapshots restored after unclean shutdown

#### Views
- Animated commit graph with lane-based coloring and Bezier-curve edges
- Unified, side-by-side, and three-way conflict diff modes with syntax
  highlighting via syntect
- Blame view with per-line author avatars
- File history view
- Reflog viewer
- Submodule panel
- Global search via `git grep`
- Commit graph search
- Detail panel with commit metadata, file list, and diff stats
- Issues and Pull Requests panels for GitHub repositories

#### UI
- Multi-repo tab bar with drag-resizable sidebar, detail, diff, and commit
  panels
- Toolbar with fetch, pull, push, branch, stash, create PR, refresh, settings,
  search, file explorer, and terminal actions
- Status bar showing branch, ahead/behind counts, staged/unstaged/stash
  counts, repository path, and active operation status
- Command palette (Ctrl+P) with context-aware commands
- Keyboard shortcuts help overlay
- Animated splash screen with skip-on-input
- Catppuccin Mocha (default), Catppuccin Latte, and One Dark themes
- JSON theme loader for user themes
- Toast notifications and confirmation dialogs for destructive actions

#### Integrations
- GitHub device-flow authentication
- Create pull request flow
- Issues and PRs fetched via GitHub API with 60-second TTL caching
- AI commit message generation via Google Gemini (optional)
- Filesystem watcher for external repo changes

#### Platform support
- Windows x86_64 zip archive and Inno Setup installer (adds rgitui to PATH
  optionally, integrates with Add/Remove Programs)
- Linux x86_64 AppImage and tarball
- macOS x86_64 and aarch64 (Apple Silicon) DMG

#### Performance
- Pre-computed trigonometric tables for commit graph edge rendering
- Per-frame memoization of `Utc::now()` for relative timestamps
- LRU caching for styled diff rows, blame, file history, and avatars
- Parallelized diff stat batching and stash / worktree enumeration
- Background git operations via GPUI's background executor to keep the UI
  thread responsive
- Diff prefetching (±25 commits, 200-entry cache) for instant navigation

#### Developer experience
- CI pipeline for Windows and Linux running fmt, clippy, tests, and release
  builds
- Automated release workflow that builds Windows (zip + installer), Linux
  (AppImage + tarball), macOS (x86_64 + aarch64 DMG), computes SHA256 sums,
  and attaches release notes from this CHANGELOG
- Background update checker that notifies when a newer release is available;
  can be disabled in Settings

### Known limitations

- Windows and macOS binaries are not yet code-signed, so SmartScreen and
  Gatekeeper will warn on first run. Choose "Run anyway" / right-click → Open
  to launch. Signing is tracked for a future release.
- Updates are announced in-app but not applied automatically; follow the link
  in the notification to download the new version.
- Only x86_64 Windows and Linux, and x86_64/aarch64 macOS are built by CI.
  Other architectures can be compiled locally with `cargo build --release`.

[Unreleased]: https://github.com/noahbclarkson/rgitui/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/noahbclarkson/rgitui/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/noahbclarkson/rgitui/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/noahbclarkson/rgitui/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/noahbclarkson/rgitui/compare/v0.1.8...v0.2.0
[0.1.8]: https://github.com/noahbclarkson/rgitui/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/noahbclarkson/rgitui/compare/v0.1.6...v0.1.7
[0.1.2]: https://github.com/noahbclarkson/rgitui/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/noahbclarkson/rgitui/releases/tag/v0.1.0
