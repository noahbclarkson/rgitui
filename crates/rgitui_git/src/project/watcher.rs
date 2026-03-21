use gpui::{AsyncApp, Context};
use notify::{RecursiveMode, Watcher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::refresh::gather_refresh_data;
use super::{GitProject, GitProjectEvent};

impl GitProject {
    /// Start watching the repository for filesystem changes.
    pub(super) fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let repo_path = self.repo_path.clone();
        let dirty = Arc::new(AtomicBool::new(false));
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

        if let Ok(mut watcher) = watcher {
            if let Err(e) = watcher.watch(&repo_path, RecursiveMode::Recursive) {
                log::warn!(
                    "Failed to watch repository directory for filesystem changes: {}",
                    e
                );
            }
            self._watcher = Some(watcher);

            let watcher_repo_path = repo_path.clone();
            cx.spawn(async move |weak, cx: &mut AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(300)).await;

                if dirty.swap(false, Ordering::Relaxed) {
                    smol::Timer::after(Duration::from_millis(200)).await;
                    dirty.store(false, Ordering::Relaxed);

                    let path = watcher_repo_path.clone();
                    let data = cx
                        .background_executor()
                        .spawn(async move { gather_refresh_data(&path) })
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
