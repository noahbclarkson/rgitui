## [Unreleased]

### Added

- **Conflicted files now open a complete three-way resolver.** Each unresolved
  region shows Current, an editable-by-choice Result, and Incoming side by side.
  Regions can take either side, both sides in either order, or be reset before
  the complete byte-preserving result is saved and staged. Whole-file choices
  handle binary, add/add, modify/delete, executable and special-file conflicts;
  an editor handoff supports manual resolutions. Stale resolver views and raw
  conflict markers are rejected instead of silently overwriting or staging
  them, and ordinary Stage / Stage All no longer bypasses the resolver. (#77)

## [0.4.1] - 2026-08-26

A performance, correctness and instrumentation release. Opening a large
repository is several times faster and uses roughly half the memory, an
inspected linked worktree is now the single subject of the window rather than a
mix of two checkouts, and the branch panel stops asserting things it never
computed. Underneath all three sits a new measurement harness — the reason the
numbers in this entry are measured rather than estimated.

> **Upgrade recommended:** keyboard shortcuts did nothing on a fresh launch
> until a panel was clicked, and repositories whose trunk is not `main` or
> `master` had every local branch listed as unmerged — inviting deletion of
> branches that were fully merged.

### Added

- **Performance and memory instrumentation harness** (`rgitui_perf`,
  `docs/PERFORMANCE.md`). Feature-gated off by default and absent from shipped
  binaries. Records frame cadence, per-draw cost, action latency, CPU, GPU and a
  *labelled* memory census, and writes a pre-digested `report.json` whose
  `findings` section states what is wrong rather than only what the numbers
  were. Scenarios drive the real keymap → dispatch → render path, with
  record/replay, deterministic corpus generation, `dhat` allocation-site
  profiling, and `rgitui-perf compare` for before/after. (#76)

### Changed

- **The splash screen hands over on readiness, not on a clock.** It waited a
  flat 1500ms regardless of what the repository needed. It now plays its opening
  animation in full and then hands over as soon as the first refresh has
  finished, keeping the splash up beyond that only for a repository that is
  still loading, to a 2500ms ceiling. A freshly initialised repository — which
  has no commits and never will until the first one — no longer waits out the
  full timeout. (#76)
- **"Go Back to Main" is now "Exit Worktree".** Leaving inspection returns to
  whichever checkout rgitui was opened on, which may be on any branch, or be a
  linked worktree itself — so the old label named a destination that was often
  neither. (#75)
- **Worktree inspection ends only when the worktree is gone.** A worktree that
  merely became clean used to throw the user back to the main checkout with the
  sidebar, commit panel and toolbar all swapping underneath. A status thread
  that failed now reports "unknown" rather than degrading to an empty status,
  which was indistinguishable from a clean worktree and triggered the same exit.
  (#75)
- **gpui and the Rust toolchain are updated** (gpui `f3fb4e04` → `24e25552`,
  toolchain 1.94.1 → 1.97.1). The toolchain move is required: the new gpui uses
  `std::hint::cold_path`. (#76)

### Fixed

#### Branch state

- **Branches are no longer reported as unmerged when nothing asked.** With no
  `main`, `master`, `origin/main` or `origin/master` to walk against, every
  branch fell through to "not merged" and was stored as a definite answer. Any
  repository whose trunk is `develop`, and any fork tracking `upstream/*`, had
  the branch health panel list every local branch as unmerged. "Could not ask"
  is now distinct from "asked, and no", and the same applies to ahead/behind
  counts, whose "not computed" sentinel was `(0, 0)` — which is also what a
  branch level with its upstream reports. (#76)
- **A checkout in a terminal no longer leaves "merged into current branch"
  stale.** The watcher now recomputes the merged flags when an answer is
  missing, and the deferred pass retries when a refresh supersedes it rather
  than leaving the flags unset for as long as anything keeps writing to the
  working tree. Carried-forward answers are keyed on the reference point they
  were derived from, so a checkout invalidates "merged into HEAD" and a push no
  longer carries "ahead 3" onto a branch that is now level. (#76)

#### Linked worktrees

- **An inspected worktree's own state stays on screen.** The displayed diff was
  recomputed against the main repository, where a file modified only inside the
  worktree has no changes — so the diff pane emptied itself on every refresh,
  including while compiling. (#75)
- **Filesystem noise no longer wakes constant refreshes.** The watcher's
  gitignore filter held one repository handle and asked it about every event
  path, so a path inside a linked worktree evaluated to a nonsense relative path
  and came back "not ignored" whatever that checkout's `.gitignore` says. There
  is now one handle per watched checkout. The inspected worktree is also watched
  and fingerprinted regardless of the `watch_all_worktrees` setting; until now
  staging inside it went undetected entirely. (#75)
- **Destructive and network operations follow the worktree on screen.**
  Confirming "discard all" while inspecting a worktree stashed the *main
  repository's* work. Clean, push, pull, fetch, force push, stash apply/pop,
  stash-to-branch, reset, and conflict resolution all targeted the wrong tree,
  from the toolbar, sidebar and command palette alike. Push and pull now resolve
  their branch, upstream and preferred remote from that checkout, and "generate
  commit message" describes the files the commit will actually contain. (#75)
- **Conflicts are reported and resolved in the checkout they happened in.** A
  conflicted merge inside a linked worktree records itself under
  `.git/worktrees/<name>/`, so the banner read the main repository's clean state
  and hid conflicts the user was looking straight at — then offered a Continue
  that refused. Banner state, conflict counts, status-bar and toolbar
  divergence counts, and undo entries all derive from the inspected checkout.
  (#75)
- **Retrying a failed network operation goes back where it failed.** The
  originating worktree is recorded on the operation rather than inferred when
  Retry is clicked, including for failures that happen before the task starts —
  a pull with no upstream, a push with no target. Switching inspection between
  the failure and the retry no longer redirects it. (#75)
- **Worktree discovery is correct when rgitui is opened on a linked worktree.**
  That checkout appeared twice while the main checkout had no row at all. The
  three sources are merged and deduplicated by canonical path. A detached HEAD
  no longer renders a short OID as though it were a branch name; the graph row
  reads "Pending changes on detached HEAD". (#75)

#### Keyboard and interface

- **Keyboard shortcuts work on a fresh launch.** Nothing took focus at startup,
  and GPUI resolves an action along the path from the focused node up to the
  root — so every shortcut landed above the workspace's handlers and did nothing
  until a panel was clicked. (#76)
- **Continue finishes the operation that actually stopped.** The conflict
  banner's Continue always dispatched a merge continuation, which refuses
  anything without `MERGE_HEAD`, so a cherry-pick, revert, rebase or mailbox
  application that stopped on conflicts advertised a recovery path that could
  not work. Continuation now follows the repository state. Bisect, which
  advances by judging the checked-out commit rather than by continuing, no
  longer shows the button.
- **Settings → Auth renders its text correctly on macOS.** Wrapping text under
  the scroll container had no definite width during GPUI's measurement pass,
  which the macOS text system could resolve as a one-glyph wrap. (#74, fixes
  #73 — thanks [@cnsky1103](https://github.com/cnsky1103))

### Performance

Measured against a generated 20,000-commit corpus, same profile, same machine.

- **Opening a large repository: 3,291ms → ~514ms to first commits.** Startup ran
  one libgit2 `merge_base` per local branch, twice — 400 walks for a 200-branch
  repository, 94% of a 3.04s snapshot — to compute two booleans that dim a
  sidebar row. Asking the question the other way round costs two revwalks in
  total, and one where HEAD and the trunk agree. Snapshot time went 3,036ms →
  258ms and startup CPU 104.7% → ~7% of a core. (#76)
- **Memory: private bytes 351.2 MB → 228.9 MB** in the shipping configuration.
  A 20 MB profiling buffer was allocated per dispatcher thread in every build,
  profiling or not; the gpui update makes that storage grow on demand behind a
  feature and a runtime flag. (#76)
- **Merged-flag and ahead/behind walks are off the first-paint path,** deferred
  beside each other, with a refresh carrying forward any value the incoming
  snapshot did not compute — so the sidebar no longer flickers between "merged"
  and "unknown" on every watcher tick. Ahead/behind now reads one
  `git for-each-ref` rather than 200 libgit2 walks. (#76)
- **The file watcher stopped polling.** It fingerprinted every loose ref every
  300ms forever — about 2% of a core per open tab, scaling with ref count — while
  the filesystem watcher was already receiving those events and discarding them.
  It now scans only when something under a git directory changed, with a 5s
  fallback for mounts where notify cannot be trusted. (#76)
- **Diff prefetching stays out of the way.** The queue is capped and drained
  newest-first, so holding a key no longer buries the diff being looked at under
  stale speculative work, and it leaves cores free rather than taking four.
  Syntect's per-line highlighter rebuild is hoisted to one per theme, the
  graph's ancestor cache is bounded on entry count and walk depth, and sidebar
  sorting and author deduplication stop allocating per comparison. (#76)

### Internal

- The `perf` feature is built in CI — roughly 2,000 lines across six crates
  previously compiled on no platform, with four census tests red the whole time.
  A `perf` job now runs clippy and tests with `--features rgitui/perf`. (#76)
- CI and release workflows take the Rust version from `rust-toolchain.toml`
  rather than pinning it separately, with a step that fails if the two drift.
  They had already drifted. (#76)
- Expanded regression coverage for worktree discovery and gitignore filtering,
  the merged-flag carry-forward, sequencer continuation, refresh-readiness
  tracking, frame-cadence calibration, and before/after comparison.

## [0.4.0] - 2026-08-12

A major workflow, safety, and interface release. Keyboard shortcuts are now
fully configurable, selected commits can be prepared for squashing directly
from the graph, and content from commits or stashes can be applied or reverted
at file, hunk, or line level. It also closes command-injection and untracked-file
data-loss paths, prevents historical diffs from staging unrelated working-tree
changes, and makes branch-heavy repositories and commit details easier to
navigate.

> **Upgrade recommended:** repository configuration and commit messages can no
> longer inject options or shell commands into network and interactive-rebase
> operations, and fast-forward merges now preserve colliding untracked files.

### Added

- **User-definable keyboard shortcuts.** A hot-reloaded JSONC `keymap.json`
  beside `settings.json` can rebind or remove every command. Invalid entries are
  isolated, overlapping bindings and unreachable chords produce named warnings,
  and the `?` panel reflects the effective live keymap. Generated keybinding
  documentation and a JSON schema provide command-name completion. (#62, closes
  #63)
- **Squash selected commits from the graph.** `Shift`- or primary-modifier-click
  selects a commit range; `s` or the context menu opens the interactive-rebase
  dialog with a squash plan ready for confirmation. Gaps, cross-branch runs,
  root commits, and unsupported merges are refused before execution. (#62)
- **Apply or revert historical diff content.** Commit and stash diffs now offer
  Apply and Revert for the current hunk, selected lines, or the whole file.
  Defaults are `a` / `r` for the selection and `Alt+A` / `Alt+R` for the file,
  and all four commands are rebindable. A three-way merge preserves unrelated
  working-tree edits without touching the index, while overlapping edits fail
  without writing partial results. (#69, closes #70)
- **Exact undo for applied or reverted content.** The original file contents and
  permissions are recorded. Undo restores them only if the file still matches
  the operation's result, protecting edits made afterward and handling created
  or removed files correctly. (#69)
- **Branch filtering for large repositories.** Local and remote branch sections
  have separate filters for name and branch-tip commit age, with local branches
  also supporting **Only my branches**. Visible rows follow UI density: 12 in
  Compact, 10 in Default, and 8 in Comfortable, with scrollbars for the rest.
  (#72)
- **Collapsible commit panel.** The commit form can collapse to a compact header
  and expand again without losing its previous height, leaving more room for
  commit details and changed files. (#72)

### Changed

- **Shortcuts use the platform's primary modifier consistently.** macOS shows
  and uses `Cmd` where Windows and Linux use `Ctrl`; OS-reserved macOS chords
  retain usable Control alternatives, and the Windows/Linux super key no longer
  acts as Control. Labels now render `Cmd` rather than `Super` on macOS. (#60,
  fixes #61; #67)
- **Shortcut dispatch is focus- and context-aware.** Commands are registered as
  GPUI actions instead of a hand-written key handler. Bare panel keys now act
  only in their focused view, overlays resolve Escape through context depth,
  and typeable bindings stand down while a text input has focus. (#62)
- **Commit metadata is denser and clearer.** Local and matching remote refs on
  the same commit are combined into one compact badge with a remote indicator
  and tooltip; signed commits use a lock beside the SHA; horizontal padding is
  reduced; and duplicate co-author trailers are collapsed case-insensitively.
  The compact ref treatment also applies to the graph. (#72)
- **Commit composition uses less vertical space.** Unnecessary space between
  the description and co-author controls is removed, while compact text inputs
  improve the file-search and branch-filter layouts. (#72)

### Fixed

#### Security and data safety

- **Repository-controlled network arguments can no longer become Git options or
  commands.** Remote and branch names are validated before reaching the CLI,
  including names read from repository configuration, and fetch, pull, and push
  pin Git's standard upload and receive-pack programs. (#64)
- **Interactive-rebase messages no longer pass through shell interpolation.**
  Reword messages are written to files consumed by `git commit --file`, and
  rebase scratch data now lives in private temporary directories. (#64)
- **Search and clone operands are option-safe.** `git grep` receives patterns
  through `-e`, and clone URLs are separated with `--`, so leading-dash input
  cannot be interpreted as a Git option. (#64)
- **Fast-forward merges no longer overwrite colliding untracked files.** A safe
  checkout runs before moving the branch ref, names conflicting paths, and
  leaves HEAD and the working tree unchanged when the merge is refused. (#64)
- **Apply and Revert stay inside the worktree.** Traversal paths, symlink or
  reparse-point parents and targets, submodules, split non-regular type changes,
  write races, and stale undo snapshots are rejected before mutation. (#69)

#### Diffs and working-tree operations

- **Historical diffs no longer masquerade as unstaged content or stage unrelated
  worktree hunks.** Commit and stash diffs are explicitly classified, show
  `Committed` or `Stashed`, and cannot emit stage or unstage requests. Stash
  diffs also remain stable across status refreshes. (#65, fixes #66)
- **Untracked files no longer block everyday Git operations unnecessarily.**
  Checkout, pull, merge, cherry-pick, revert, and rebase reject tracked
  modifications while allowing unrelated untracked files; Git still refuses
  when an operation would overwrite one. (#64)
- **Clean Untracked now performs the requested cleanup.** Its dry run reads
  Git's stdout rather than the empty stderr stream, so the real clean executes
  and reports the correct count. (#64)
- **Literal wildcard-like filenames are diffed correctly.** Paths such as
  `data[1].json` are treated as exact filenames rather than pathspec patterns.
  (#64)
- **Diff hunk actions stay inside the panel.** Apply/Revert and Stage/Unstage
  controls are constrained in unified and split headers instead of overflowing
  into adjacent panels. (#72)

#### Keyboard and interface

- **Keyboard commit submission matches the button.** Primary-modifier+Enter now
  respects the Amend checkbox and the staged-changes guard. (#64)
- **Font Size now affects the interface.** The stored preference scales the
  rem-based UI while preserving the previous pixel size at the default setting.
  (#64)
- **Shortcut and focus regressions are corrected.** Space activates sidebar
  rows; `?` does not open help while typing; Enter no longer both opens a result
  and submits a query; affected lists support Home/End; and toolbar and help
  labels advertise the real Fetch and global-search shortcuts. (#62)
- **Commit-details scrolling is independent and predictable.** Short changed-file
  lists stay fixed, a scrollbar appears only beyond four rows, metadata and file
  lists scroll separately, and wheel input over files no longer moves metadata.
  (#72)
- **Scrollbar dragging tracks the pointer continuously.** Drag events refresh
  during movement and are captured and consumed, preventing lag, snapping, and
  interaction leaking into content behind the scrollbar. (#72)
- **Branch-filter popovers render and interact correctly.** Compact inputs are
  no longer clipped, and the popover occludes pointer movement so hover events
  do not leak into branch rows behind it. (#72)
- **Long commit metadata no longer steals the changed-files scroll target.**
  Metadata is bounded to its own scroll region, leaving the file list
  independently hover-scrollable. File search, margins, badges, and row padding
  are tightened to avoid overflow. (#72)

### Performance

- **Worktree refreshes avoid repeatedly reading entire large modified files.**
  Files over 64 KiB are fingerprinted from bounded samples at both ends. (#64)
- **Git ref polling avoids walking and reading every ref every 300ms.** Directory
  and file metadata detect broad ref changes while HEAD, the checked-out branch,
  and `packed-refs` retain content hashing. (#64)
- **Syntax colors avoid format-and-reparse work for every highlighted span,** and
  unused libgit2 network and TLS default features are no longer built or linked.
  (#64)

### Internal

- Added a cross-platform headless GPUI `ViewTest` harness with deterministic
  keyboard and mouse simulation, safe teardown, and disk-free settings
  initialization. (#59)
- Consolidated temporary Git-repository helpers into `TempRepo`, with
  deterministic commits, `main` as the initial branch, byte-stable line endings,
  and Windows-safe cleanup ordering. (#71)
- Expanded regression coverage for keymap conflicts and context resolution,
  historical diff staging guards, Apply/Revert and exact undo, command-injection
  vectors, safe checkout, branch filters, ref compaction, scrolling, and commit
  panel collapse.
- Updated CI and release actions to Node 24-native major versions, removed
  accidentally tracked Python bytecode and obsolete test scaffolding, and
  rewrote implementation comments around current invariants. (#58, #68)

## [0.3.2] - 2026-07-27

A focused follow-up to 0.3.1. Blame and file history are prepared before you ask
for them and stay honestly disabled when a file has neither, diff text can be
selected with the mouse and copied, the commit graph routes dirty worktrees
without disturbing neighbouring lanes, and the sidebar and detail panel now size
their lists to the space actually available instead of stopping partway down the
panel.

### Added

- **Mouse selection in the diff viewer.** Drag across diff rows, or `Shift`+click
  to extend a range from the current anchor, then copy the selected lines with
  `Ctrl+C`. Both gestures are listed in the keyboard-shortcuts overlay.
- **Blame and file-history prefetching.** Whenever a diff is displayed, blame and
  history for that path load in the background, so switching to those views is
  normally instant. Work is single-flight per repository, path, and diff
  generation — entries are reserved before the background task starts, so a fast
  second click cannot launch the same Git command again.

### Changed

- **The Blame and History tabs reflect what the file actually has.** A path with
  no blame or no history leaves its tab dimmed and unclickable instead of opening
  an empty view, and results that fail or return nothing are retained as
  unavailable so the tab stays disabled.
- **File history is anchored to the selected commit.** Opening history from a
  commit diff stops at that commit rather than including changes made to the file
  later on the current branch.
- **The keyboard-shortcuts overlay adapts to the window.** The modal is clamped to
  the viewport, falls back to a single column below 960px wide, and scrolls
  instead of overflowing its categories.

### Fixed

- **Commit-graph routing for dirty worktrees.** A worktree's pending-changes node
  is now placed as a child of its HEAD commit: it reuses the HEAD lane only when
  that HEAD has no visible incoming child and no earlier worktree has claimed the
  lane, and otherwise takes the first free lane with an explicit edge back to
  HEAD. Real lanes pass through the virtual row instead of being consumed by it,
  worktrees sharing a HEAD render as siblings rather than a chain, and a worktree
  whose HEAD lies outside the loaded window no longer draws an edge to a false
  parent. (#56)
- **Sidebar sections stopped short of the panel bottom.** Expanded sections were
  capped at twelve rows, so a long Unstaged list ended mid-panel with unusable
  space beneath it while its remaining files were reachable only by scrolling
  inside that small box. Section heights are now planned from the measured panel
  height: sections that fit keep their content height, and the rest share what is
  left down to a three-row minimum, so the sections fill the sidebar and still
  virtualize.
- **The changed-files list left dead space in the detail panel.** It was limited
  to a fixed 600px regardless of window size. The list now asks for its full
  content height and is the only part of the panel that gives up space, so it
  ends flush with the bottom of the panel and stops shrinking at four rows.

### Internal

- Added `PRODUCT.md` describing the product's users, purpose, design principles,
  and accessibility expectations.
- `LruCache::peek` reads a cached value without disturbing recency order.
- Added regression coverage for worktree lane layout, mouse-selection ranges,
  file history anchored at a commit, sidebar list-height distribution, and
  detail-panel file-list sizing.

## [0.3.1] - 2026-07-19

This patch release makes large repositories and GitHub-backed workflows feel
substantially more immediate. Commit history can now hydrate from a validated
on-disk cache, Issues and Pull Requests are prefetched through a shared bounded
cache, and the graph, diff, sidebar, detail, markdown, avatar, and search paths
avoid their largest redundant computations and renders. It also fixes stale
async results and several repository-edge cases uncovered by the performance
audit.

### Added

- **Persistent commit-history cache.** Recent commit metadata and graph inputs
  are stored atomically using the repository's common Git directory as its
  identity. Exact HEAD/ref fingerprints, a schema version, corruption fallback,
  and generation pruning ensure stale history is never shown as current.
- **Instant GitHub panel warm-up.** Open Issues and Pull Requests begin loading
  as soon as a repository's GitHub remote is known, so the panels are normally
  ready before they are opened. Requests share a memory-only cache with
  singleflight deduplication, ETag revalidation, pagination, and rate-limit
  reset reporting.
- **DeepSeek AI provider.** Settings now offers DeepSeek with the current
  `deepseek-v4-flash` and `deepseek-v4-pro` models, including repository tool
  calls through DeepSeek's OpenAI-compatible chat API.
- **Merged-branch indicators.** Local branches already reachable from the
  current branch now show a green merge icon, matching `git branch --merged`.
- **Untracked-file filter.** The `?N` count in the Unstaged header is clickable,
  making it easy to hide or restore a large untracked-file backdrop per session.

### Changed

- **Repository refreshes are coherent and worktree-aware.** Related watcher
  signals are consolidated into one repository-change event, linked worktrees
  watch both their worktree and common Git directories, and ref fingerprints
  include loose-ref contents rather than timestamps alone.
- **History edge cases are handled consistently.** Annotated tags are peeled,
  detached HEADs remain usable, `origin/HEAD` is preferred when selecting the
  default branch, author filtering is preserved, and duplicate load-more
  requests are suppressed.
- **Network operations fail sooner when a connection stalls** through bounded
  HTTP low-speed and SSH idle timeouts.
- **Global search materializes results in bounded pages.** The first 250 rows
  render immediately and additional pages are expanded on demand, avoiding a
  frame spike for broad `git grep` queries.
- **Tracked changes use semantic filename colors** while untracked files are
  dimmed, so active modifications stand out without widening the sidebar.

### Fixed

- **Stale background work could overwrite newer UI state.** Commit refresh,
  filtering, pagination, ahead/behind calculation, graph computation, diff
  preparation, GitHub requests, and global search now carry request identity or
  generation guards and discard obsolete completions.
- **Diff display cache collisions and cross-mode reuse.** Cache keys now include
  staged state and a full source fingerprint, validate collisions, retain the
  most complete unified representation, and evict by true LRU order under both
  entry and rendered-row budgets.
- **GitHub panels could show data from the wrong repository or authentication
  context.** Repository, resource/filter/search/comment, and auth identity are
  part of cache and request keys; private payloads and credentials are never
  persisted.
- **Global search could open an invisible panel, route through duplicate
  shortcuts, retain stale results, or paint a blank result area.** The panel is
  now owned per repository tab, renders populated rows reliably, scrolls, and
  restores the Diff panel when dismissed.
- **Graph and detail updates could become internally inconsistent.** Commit and
  graph-row snapshots are applied atomically, selection is resolved at apply
  time, and in-flight computations are deduplicated.
- Auth Preferences no longer collapses horizontally in the settings window.
- Hunk staging now emits complete patches, and hunk unstaging correctly reverses
  the selected change instead of applying the staged direction again.
- Completing a clone opens and refreshes the cloned repository in its own tab;
  it no longer replaces the graph data inside the previously active project tab.

### Performance

- **Commit graph startup avoids a fresh history walk when possible.** A valid
  cache is hydrated synchronously for immediate display while writes and cache
  maintenance stay off the UI thread.
- **Diff preparation is shared and bounded.** Syntax highlighting, word diff,
  longest-row calculation, LCS alignment, and three-way preparation run in the
  background and reuse `Arc` snapshots. Speculative neighbouring-commit diffs
  are singleflight with a four-task concurrency cap and bounded batches.
- **Large lists build only their visible rows.** Graph, Issues, Pull Requests,
  sidebar sections, and changed-file trees use bounded virtual scrolling;
  flattened branch/file/tree data is rebuilt only when its source changes.
- **Per-frame cloning and reparsing are reduced.** Graph search is
  allocation-light, commit and diff snapshots are shared, branch health and
  stash presentation are cached, markdown ASTs use a bounded cache, and avatar
  writes/UI refreshes are coalesced.
- **GitHub data is bounded in memory.** The shared service uses a 60-second TTL,
  128-entry LRU, and 16 MiB raw-payload budget, including successful empty
  responses so repeatedly opening an empty panel remains instant.

### Internal

- Added repository-specific contributor guidance in `CLAUDE.md` covering the
  GPUI entity model, background-work rules, crate boundaries, and CI commands.
- Expanded regression coverage for cache invalidation/corruption, stale async
  generations, worktrees and refs, GitHub pagination/cache identity, diff-cache
  collisions and LRU eviction, search paging, and graph computation.

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

[Unreleased]: https://github.com/noahbclarkson/rgitui/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/noahbclarkson/rgitui/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/noahbclarkson/rgitui/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/noahbclarkson/rgitui/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/noahbclarkson/rgitui/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/noahbclarkson/rgitui/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/noahbclarkson/rgitui/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/noahbclarkson/rgitui/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/noahbclarkson/rgitui/compare/v0.1.8...v0.2.0
[0.1.8]: https://github.com/noahbclarkson/rgitui/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/noahbclarkson/rgitui/compare/v0.1.6...v0.1.7
[0.1.2]: https://github.com/noahbclarkson/rgitui/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/noahbclarkson/rgitui/releases/tag/v0.1.0
