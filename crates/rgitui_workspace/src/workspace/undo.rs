use std::time::Instant;

use gpui::Context;

use crate::ToastKind;

use super::Workspace;

/// An undoable operation with the data needed to reverse it.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub label: String,
    pub action: UndoAction,
    pub created_at: Instant,
}

/// The specific reversal strategy for each operation type.
#[derive(Debug, Clone)]
pub enum UndoAction {
    /// Undo discard: pop a hidden stash that was saved before discarding.
    PopStash(usize),
    /// Undo branch delete: recreate the branch at the given OID.
    RecreateBranch { name: String, oid_hex: String },
    /// Undo tag delete: recreate the tag at the given OID.
    RecreateTag { name: String, oid_hex: String },
    /// Undo reset: hard-reset back to the previous HEAD OID.
    ResetTo(String),
    /// Undo commit: soft-reset to the previous HEAD OID (preserves changes in index).
    SoftResetHead(String),
}

impl UndoEntry {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > 10
    }
}

impl Workspace {
    /// Execute the pending undo operation, if any, and show a result toast.
    pub fn execute_undo(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.last_undo.take() else {
            self.show_toast("Nothing to undo", ToastKind::Info, cx);
            return;
        };

        if undo.is_expired() {
            self.show_toast("Undo expired (10s limit)", ToastKind::Warning, cx);
            return;
        }

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let project = tab.project.clone();

        match undo.action {
            UndoAction::RecreateBranch { name, oid_hex } => {
                let branch_name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.create_branch_at(&branch_name, Some(&oid_hex), cx)
                        .detach();
                });
                self.show_toast(format!("Restored branch '{}'", name), ToastKind::Success, cx);
            }
            UndoAction::RecreateTag { name, oid_hex } => {
                let tag_name = name.clone();
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.create_tag(&tag_name, oid, cx).detach();
                    });
                    self.show_toast(format!("Restored tag '{}'", name), ToastKind::Success, cx);
                } else {
                    self.show_toast("Failed to restore tag: invalid OID", ToastKind::Error, cx);
                }
            }
            UndoAction::PopStash(index) => {
                project.update(cx, |proj, cx| {
                    proj.stash_pop(index, cx).detach();
                });
                self.show_toast("Restored discarded changes", ToastKind::Success, cx);
            }
            UndoAction::SoftResetHead(oid_hex) => {
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.reset_soft(oid, cx).detach();
                    });
                    self.show_toast(
                        "Undid last commit (changes preserved in index)",
                        ToastKind::Success,
                        cx,
                    );
                } else {
                    self.show_toast(
                        "Failed to undo commit: invalid OID",
                        ToastKind::Error,
                        cx,
                    );
                }
            }
            UndoAction::ResetTo(oid_hex) => {
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.reset_to_commit(oid, cx).detach();
                    });
                    self.show_toast("Undid reset", ToastKind::Success, cx);
                } else {
                    self.show_toast("Failed to undo reset: invalid OID", ToastKind::Error, cx);
                }
            }
        }
    }

    /// Schedule auto-expiry of the undo entry after 10 seconds.
    pub(super) fn schedule_undo_expiry(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(11))
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if let Some(entry) = &this.last_undo {
                        if entry.is_expired() {
                            this.last_undo = None;
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
        })
        .detach();
    }
}
