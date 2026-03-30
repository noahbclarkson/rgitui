use gpui::{AsyncApp, Context};
use notify::{RecursiveMode, Watcher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::refresh::gather_refresh_data_lightweight;
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

impl GitProject {
    /// Start watching the repository for filesystem changes.
    pub(super) fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let dirty = Arc::new(AtomicBool::new(false));

        let git_dir = repo_path.join(".git");
        let last_git_fingerprint = Arc::new(std::sync::Mutex::new(git_state_fingerprint(&git_dir)));

        let watcher = {
            let dirty_flag = dirty.clone();
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
            })
        };

        if let Ok(mut watcher) = watcher {
            if let Err(e) = watcher.watch(&repo_path, RecursiveMode::Recursive) {
                log::warn!(
                    "Failed to watch repository directory for filesystem changes: {}",
                    e
                );
            }
            self._watcher = Some(watcher);

            let watcher_repo_path = repo_path.clone();
            let dirty_flag = dirty.clone();
            let git_fingerprint = last_git_fingerprint.clone();
            cx.spawn(async move |weak, cx: &mut AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(300)).await;

                // Detect git internal state changes (commits, pushes, branch ops,
                // staging changes, etc.) that don't produce working-directory events.
                let git_dir = watcher_repo_path.join(".git");
                let fingerprint_changed = git_fingerprint
                    .lock()
                    .ok()
                    .and_then(|guard| {
                        let current = git_state_fingerprint(&git_dir);
                        if current.is_some() && current != *guard {
                            Some(current)
                        } else {
                            None
                        }
                    })
                    .flatten();

                if let Some(new_fp) = fingerprint_changed {
                    if let Ok(mut guard) = git_fingerprint.lock() {
                        *guard = Some(new_fp);
                    }
                    dirty_flag.store(true, Ordering::Relaxed);
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

                    let path = watcher_repo_path.clone();
                    let data = cx
                        .background_executor()
                        .spawn(async move { gather_refresh_data_lightweight(&path) })
                        .await;

                    let data = match data {
                        Ok(d) => d,
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
            })
            .detach();
        }
    }
}
