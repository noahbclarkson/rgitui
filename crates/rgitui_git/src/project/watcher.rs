use gpui::{AsyncApp, Context};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::refresh::{gather_refresh_data_lightweight_cached, WorktreeStatusCache};
use super::{GitProject, GitProjectEvent};

/// Compute the newest mtime across all critical `.git` sentinel files.
///
/// This covers:
/// - `.git/HEAD` — branch switches, detached HEAD
/// - `.git/index` — staging area changes (`git add`, `git reset`)
/// - `.git/packed-refs` — packed ref updates
/// - `.git/MERGE_HEAD`, `.git/REBASE_HEAD` — in-progress operations
/// - Current branch ref file (parsed from HEAD) — commits on current branch
/// - `.git/refs/heads/` — local branch changes from external operations
/// - `.git/refs/remotes/` — push/fetch ref updates
/// - `.git/refs/tags/` — tag creation/deletion
fn git_state_fingerprint(git_dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut check = |path: &Path| {
        if let Ok(mtime) = path.metadata().and_then(|m| m.modified()) {
            newest = Some(newest.map_or(mtime, |n| n.max(mtime)));
        }
    };

    // Sentinel files
    check(&git_dir.join("HEAD"));
    check(&git_dir.join("index"));
    check(&git_dir.join("packed-refs"));
    check(&git_dir.join("MERGE_HEAD"));
    check(&git_dir.join("REBASE_HEAD"));

    // Current branch ref file (e.g. refs/heads/main from "ref: refs/heads/main\n")
    if let Ok(head_content) = std::fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(ref_rel) = head_content.strip_prefix("ref: ") {
            check(&git_dir.join(ref_rel.trim()));
        }
    }

    // Scan refs directories (local branches, remotes, tags)
    for subdir in &["refs/heads", "refs/remotes", "refs/tags"] {
        scan_refs_mtimes(&git_dir.join(subdir), &mut newest);
    }

    newest
}

/// Recursively scan a directory for the newest file mtime.
fn scan_refs_mtimes(dir: &Path, newest: &mut Option<SystemTime>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_refs_mtimes(&path, newest);
        } else if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
            *newest = Some(newest.map_or(mtime, |n| n.max(mtime)));
        }
    }
}

/// Resolve the per-worktree git dir for a linked worktree under the main repo.
/// A worktree's `.git` is a file containing `gitdir: <path>` pointing into
/// `<main>/.git/worktrees/<name>/`. We read that redirect here so fingerprinting
/// still works when the caller only has the worktree's working-tree path.
fn worktree_git_dir(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    let content = std::fs::read_to_string(&dot_git).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("gitdir:") {
            let p = PathBuf::from(rest.trim());
            return Some(if p.is_absolute() {
                p
            } else {
                worktree_path.join(p)
            });
        }
    }
    None
}

/// Compute a combined fingerprint across the main repo's `.git` and every
/// linked worktree's per-worktree git dir. Returns the newest mtime seen.
fn combined_fingerprint(
    main_git_dir: &Path,
    extra_worktree_paths: &HashSet<PathBuf>,
) -> Option<SystemTime> {
    let mut newest = git_state_fingerprint(main_git_dir);
    for wt in extra_worktree_paths {
        if let Some(git_dir) = worktree_git_dir(wt) {
            if let Some(fp) = git_state_fingerprint(&git_dir) {
                newest = Some(newest.map_or(fp, |n| n.max(fp)));
            }
        }
    }
    newest
}

impl GitProject {
    /// Start watching the repository for filesystem changes.
    ///
    /// Always watches the main repo path. When the `watch_all_worktrees`
    /// setting is enabled, additionally watches every linked worktree's path
    /// and fingerprints its per-worktree git dir. The watched set is
    /// re-evaluated on every poll tick, so toggling the setting or adding /
    /// removing worktrees takes effect without restarting the app.
    pub(super) fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let main_git_dir = repo_path.join(".git");
        let dirty = Arc::new(AtomicBool::new(false));
        let worktree_cache: Arc<Mutex<WorktreeStatusCache>> = self.worktree_status_cache.clone();

        // Keep the notify watcher behind a mutex so the poll loop can add /
        // remove watched paths when the all-worktrees setting flips.
        let watcher_handle: Arc<Mutex<Option<RecommendedWatcher>>> = Arc::new(Mutex::new(None));

        {
            let dirty_flag = dirty.clone();
            let watcher =
                notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        let dominated_by_git = event
                            .paths
                            .iter()
                            .all(|p| p.components().any(|c| c.as_os_str() == ".git"));
                        if dominated_by_git {
                            return;
                        }
                        match event.kind {
                            notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_) => {
                                dirty_flag.store(true, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                    }
                });

            match watcher {
                Ok(mut w) => {
                    if let Err(e) = w.watch(&repo_path, RecursiveMode::Recursive) {
                        log::warn!(
                            "Failed to watch repository directory for filesystem changes: {}",
                            e
                        );
                    }
                    *watcher_handle.lock().expect("watcher mutex") = Some(w);
                }
                Err(e) => {
                    log::warn!("Failed to create filesystem watcher: {}", e);
                }
            }
        }

        let last_fingerprint: Arc<Mutex<Option<SystemTime>>> = Arc::new(Mutex::new(
            combined_fingerprint(&main_git_dir, &HashSet::new()),
        ));
        let watched_extras: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));

        let watcher_commit_limit = self.commit_limit;
        cx.spawn({
            let watcher_handle = watcher_handle.clone();
            let watched_extras = watched_extras.clone();
            let last_fingerprint = last_fingerprint.clone();
            let dirty = dirty.clone();
            let repo_path = repo_path.clone();
            let main_git_dir = main_git_dir.clone();
            let worktree_cache = worktree_cache.clone();
            async move |weak, cx: &mut AsyncApp| {
                // Keep the watcher handle alive for the lifetime of this task.
                // When the task ends (because the entity was dropped), the Arc
                // refcount hits zero and the notify watcher shuts down.
                let _keepalive = watcher_handle.clone();

                loop {
                    smol::Timer::after(Duration::from_millis(300)).await;
                    log::trace!("watcher: poll tick");

                    // Re-read the all-worktrees setting and current worktree
                    // list. Sync the notify watcher's watched paths and the
                    // fingerprint set to match. `None` signals the entity
                    // has been dropped so we should end the watcher loop.
                    let desired: Option<HashSet<PathBuf>> = cx.update(|app| {
                        let entity = weak.upgrade()?;
                        let watch_all = app
                            .try_global::<rgitui_settings::SettingsState>()
                            .map(|s| s.settings().watch_all_worktrees)
                            .unwrap_or(false);
                        if !watch_all {
                            return Some(HashSet::new());
                        }
                        Some(
                            entity
                                .read(app)
                                .worktrees
                                .iter()
                                .filter(|w| !w.is_current)
                                .map(|w| w.path.clone())
                                .collect::<HashSet<PathBuf>>(),
                        )
                    });
                    let desired = match desired {
                        Some(d) => d,
                        None => break,
                    };

                    {
                        let mut current = watched_extras.lock().expect("watched mutex");
                        if *current != desired {
                            if let Ok(mut guard) = watcher_handle.lock() {
                                if let Some(w) = guard.as_mut() {
                                    for to_remove in current.difference(&desired) {
                                        if let Err(e) = w.unwatch(to_remove) {
                                            log::debug!(
                                                "Failed to unwatch {}: {}",
                                                to_remove.display(),
                                                e
                                            );
                                        }
                                    }
                                    for to_add in desired.difference(&current) {
                                        if let Err(e) = w.watch(to_add, RecursiveMode::Recursive) {
                                            log::warn!(
                                                "Failed to watch worktree {}: {}",
                                                to_add.display(),
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                            *current = desired.clone();
                        }
                    }

                    // Detect git internal state changes across main + watched
                    // worktrees (commits, pushes, branch ops, staging
                    // changes, etc.) that don't produce working-directory
                    // events on the main repo.
                    let fingerprint_changed = {
                        let current = combined_fingerprint(&main_git_dir, &desired);
                        let mut guard = last_fingerprint.lock().expect("fp mutex");
                        if current.is_some() && current != *guard {
                            *guard = current;
                            true
                        } else {
                            false
                        }
                    };

                    if fingerprint_changed {
                        log::debug!("watcher: git state changed, scheduling refresh");
                        dirty.store(true, Ordering::Relaxed);
                    }

                    if dirty.swap(false, Ordering::Relaxed) {
                        smol::Timer::after(Duration::from_millis(200)).await;

                        // Only clear dirty if no new events arrived during the batch wait.
                        // If dirty was re-set, loop back immediately to refresh again without
                        // the 300ms poll interval delay — prevents missed refreshes under
                        // bursty file system activity (e.g. git checkout touching 20 files).
                        if !dirty.load(Ordering::Relaxed) {
                            dirty.store(false, Ordering::Relaxed);
                        }

                        let path = repo_path.clone();
                        let cache = worktree_cache.clone();
                        let data = cx
                            .background_executor()
                            .spawn(async move {
                                gather_refresh_data_lightweight_cached(
                                    &path,
                                    watcher_commit_limit,
                                    &cache,
                                )
                            })
                            .await;

                        let data = match data {
                            Ok(d) => {
                                log::debug!("watcher: lightweight refresh complete");
                                d
                            }
                            Err(_) => continue,
                        };

                        let result = cx.update(|cx| {
                            weak.update(cx, |this, cx| {
                                this.apply_refresh_data(data);
                                cx.emit(GitProjectEvent::StatusChanged);
                                cx.notify();
                            })
                        });

                        if result.is_err() {
                            break;
                        }
                    }
                }
                drop(_keepalive);
            }
        })
        .detach();
    }
}
