use gpui::{AsyncApp, Context};
use notify::{RecursiveMode, Watcher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::refresh::gather_refresh_data_lightweight;
use super::{GitProject, GitProjectEvent};

impl GitProject {
    /// Start watching the repository for filesystem changes.
    pub(super) fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let dirty = Arc::new(AtomicBool::new(false));

        // Track .git/index mtime to detect git index-only changes (e.g. `git add`,
        // `git reset HEAD`) which do not produce working-directory filesystem events.
        let index_path = repo_path.join(".git/index");
        let last_index_mtime = Arc::new(std::sync::Mutex::new(
            index_path.metadata().ok().and_then(|m| m.modified().ok()),
        ));

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
            let index_path = index_path.clone();
            let index_mtime = last_index_mtime.clone();
            cx.spawn(async move |weak, cx: &mut AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(300)).await;

                // Detect git index-only changes that don't produce working-directory events.
                let index_changed = index_mtime
                    .lock()
                    .ok()
                    .and_then(|guard| {
                        let current = index_path.metadata().ok()?.modified().ok();
                        let changed = current.is_some() && current != *guard;
                        if changed {
                            Some(current)
                        } else {
                            None
                        }
                    })
                    .flatten();

                if let Some(new_mtime) = index_changed {
                    if let Ok(mut guard) = index_mtime.lock() {
                        *guard = Some(new_mtime);
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
