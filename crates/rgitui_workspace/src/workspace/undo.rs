use std::time::Instant;

use gpui::Context;

use crate::ToastKind;

use super::Workspace;

/// Maximum number of undo entries to keep in history.
const MAX_UNDO_HISTORY: usize = 20;

/// Undo entry expiry time in seconds.
const UNDO_EXPIRY_SECS: u64 = 60;

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
    /// Undo stage: unstage the file paths.
    UnstageFiles { paths: Vec<String> },
    /// Undo unstage: stage the file paths.
    StageFiles { paths: Vec<String> },
}

impl UndoEntry {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > UNDO_EXPIRY_SECS
    }
}

impl Workspace {
    /// Push an undo entry to the history, maintaining max size.
    pub fn push_undo(&mut self, entry: UndoEntry, cx: &mut Context<Self>) {
        self.undo_history.push(entry);
        // Trim to max size, keeping most recent entries
        if self.undo_history.len() > MAX_UNDO_HISTORY {
            self.undo_history.remove(0);
        }
        self.schedule_undo_expiry(cx);
        cx.notify();
    }

    /// Get the number of available undo entries.
    pub fn undo_count(&self) -> usize {
        self.undo_history.iter().filter(|e| !e.is_expired()).count()
    }

    /// Execute the most recent undo operation, if any, and show a result toast.
    pub fn execute_undo(&mut self, cx: &mut Context<Self>) {
        // Clean expired entries first
        self.undo_history.retain(|e| !e.is_expired());

        let Some(undo) = self.undo_history.pop() else {
            self.show_toast("Nothing to undo", ToastKind::Info, cx);
            return;
        };

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let project = tab.project.clone();

        let undo_label = undo.label;
        let remaining = self.undo_history.len();

        match undo.action {
            UndoAction::RecreateBranch { name, oid_hex } => {
                let branch_name = name;
                project.update(cx, |proj, cx| {
                    proj.create_branch_at(&branch_name, Some(&oid_hex), cx)
                        .detach();
                });
                self.show_toast(
                    format!(
                        "Undid: {undo_label}{}",
                        if remaining > 0 {
                            format!(" ({remaining} more)", remaining = remaining)
                        } else {
                            "".into()
                        }
                    ),
                    ToastKind::Success,
                    cx,
                );
            }
            UndoAction::RecreateTag { name, oid_hex } => {
                let tag_name = name;
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.create_tag(&tag_name, oid, cx).detach();
                    });
                    self.show_toast(
                        format!(
                            "Undid: {undo_label}{}",
                            if remaining > 0 {
                                format!(" ({remaining} more)", remaining = remaining)
                            } else {
                                "".into()
                            }
                        ),
                        ToastKind::Success,
                        cx,
                    );
                } else {
                    self.show_toast("Failed to undo: invalid OID", ToastKind::Error, cx);
                }
            }
            UndoAction::PopStash(index) => {
                project.update(cx, |proj, cx| {
                    proj.stash_pop(index, cx).detach();
                });
                self.show_toast(
                    format!(
                        "Undid: {undo_label}{}",
                        if remaining > 0 {
                            format!(" ({remaining} more)", remaining = remaining)
                        } else {
                            "".into()
                        }
                    ),
                    ToastKind::Success,
                    cx,
                );
            }
            UndoAction::SoftResetHead(oid_hex) => {
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.reset_soft(oid, cx).detach();
                    });
                    self.show_toast(
                        format!(
                            "Undid: {undo_label}{}",
                            if remaining > 0 {
                                format!(" ({remaining} more)", remaining = remaining)
                            } else {
                                "".into()
                            }
                        ),
                        ToastKind::Success,
                        cx,
                    );
                } else {
                    self.show_toast("Failed to undo: invalid OID", ToastKind::Error, cx);
                }
            }
            UndoAction::ResetTo(oid_hex) => {
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.reset_to_commit(oid, cx).detach();
                    });
                    self.show_toast(
                        format!(
                            "Undid: {undo_label}{}",
                            if remaining > 0 {
                                format!(" ({remaining} more)", remaining = remaining)
                            } else {
                                "".into()
                            }
                        ),
                        ToastKind::Success,
                        cx,
                    );
                } else {
                    self.show_toast("Failed to undo: invalid OID", ToastKind::Error, cx);
                }
            }
            UndoAction::UnstageFiles { paths } => {
                let paths: Vec<std::path::PathBuf> =
                    paths.iter().map(std::path::PathBuf::from).collect();
                project.update(cx, |proj, cx| {
                    proj.unstage_files(&paths, cx).detach();
                });
                self.show_toast(
                    format!(
                        "Undid: {undo_label}{}",
                        if remaining > 0 {
                            format!(" ({remaining} more)", remaining = remaining)
                        } else {
                            "".into()
                        }
                    ),
                    ToastKind::Success,
                    cx,
                );
            }
            UndoAction::StageFiles { paths } => {
                let paths: Vec<std::path::PathBuf> =
                    paths.iter().map(std::path::PathBuf::from).collect();
                project.update(cx, |proj, cx| {
                    proj.stage_files(&paths, cx).detach();
                });
                self.show_toast(
                    format!(
                        "Undid: {undo_label}{}",
                        if remaining > 0 {
                            format!(" ({remaining} more)", remaining = remaining)
                        } else {
                            "".into()
                        }
                    ),
                    ToastKind::Success,
                    cx,
                );
            }
        }
    }

    /// Schedule periodic cleanup of expired undo entries.
    pub(super) fn schedule_undo_expiry(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(UNDO_EXPIRY_SECS + 1))
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    let before = this.undo_history.len();
                    this.undo_history.retain(|e| !e.is_expired());
                    if this.undo_history.len() != before {
                        cx.notify();
                    }
                })
                .ok();
            })
        })
        .detach();
    }
}
