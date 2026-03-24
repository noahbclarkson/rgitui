#[cfg(test)]
use std::time::Duration;
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

    #[cfg(test)]
    pub(crate) fn expired_entry(label: &str, action: UndoAction) -> Self {
        UndoEntry {
            label: label.to_string(),
            action,
            // subtract enough time to make it expired
            created_at: Instant::now()
                .checked_sub(Duration::from_secs(UNDO_EXPIRY_SECS + 2))
                .unwrap_or(Instant::now()),
        }
    }
}

/// Pure undo/redo stack — no GPUI context, fully unit-testable.
///
/// `Workspace` owns one of these; the GPUI-aware methods on `Workspace`
/// delegate to it for all stack mutations and simply call `cx.notify()`
/// and show toasts on top.
#[derive(Debug, Default)]
pub struct UndoStack {
    entries: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an entry, capping at `MAX_UNDO_HISTORY`.
    pub fn push(&mut self, entry: UndoEntry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_UNDO_HISTORY {
            self.entries.remove(0);
        }
    }

    /// Pop the most-recent non-expired entry (purges expired ones first).
    /// Returns `None` when the stack is empty or all entries have expired.
    pub fn pop(&mut self) -> Option<UndoEntry> {
        self.purge_expired();
        self.entries.pop()
    }

    /// Number of non-expired entries remaining.
    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_expired()).count()
    }

    /// Remove entries that have passed their TTL.
    pub fn purge_expired(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !e.is_expired());
        before - self.entries.len()
    }

    /// True when there are no live (non-expired) entries.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Peek at the most-recent non-expired entry label without consuming it.
    #[allow(dead_code)]
    pub fn peek_label(&self) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|e| !e.is_expired())
            .map(|e| e.label.as_str())
    }
}

// ── Workspace glue ────────────────────────────────────────────────────────────

impl Workspace {
    /// Push an undo entry to the history, maintaining max size.
    pub fn push_undo(&mut self, entry: UndoEntry, cx: &mut Context<Self>) {
        self.undo_stack.push(entry);
        self.schedule_undo_expiry(cx);
        cx.notify();
    }

    /// Get the number of available undo entries.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.count()
    }

    /// Execute the most recent undo operation, if any, and show a result toast.
    pub fn execute_undo(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo_stack.pop() else {
            self.show_toast("Nothing to undo", ToastKind::Info, cx);
            return;
        };

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let project = tab.project.clone();

        let undo_label = undo.label;
        let remaining = self.undo_stack.count();

        let suffix = if remaining > 0 {
            format!(" ({remaining} more)")
        } else {
            String::new()
        };

        match undo.action {
            UndoAction::RecreateBranch { name, oid_hex } => {
                project.update(cx, |proj, cx| {
                    proj.create_branch_at(&name, Some(&oid_hex), cx).detach();
                });
                self.show_toast(
                    format!("Undid: {undo_label}{suffix}"),
                    ToastKind::Success,
                    cx,
                );
            }
            UndoAction::RecreateTag { name, oid_hex } => {
                if let Ok(oid) = git2::Oid::from_str(&oid_hex) {
                    project.update(cx, |proj, cx| {
                        proj.create_tag(&name, oid, cx).detach();
                    });
                    self.show_toast(
                        format!("Undid: {undo_label}{suffix}"),
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
                    format!("Undid: {undo_label}{suffix}"),
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
                        format!("Undid: {undo_label}{suffix}"),
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
                        format!("Undid: {undo_label}{suffix}"),
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
                    format!("Undid: {undo_label}{suffix}"),
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
                    format!("Undid: {undo_label}{suffix}"),
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
                    let removed = this.undo_stack.purge_expired();
                    if removed > 0 {
                        cx.notify();
                    }
                })
                .ok();
            })
        })
        .detach();
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(label: &str) -> UndoEntry {
        UndoEntry {
            label: label.to_string(),
            action: UndoAction::StageFiles {
                paths: vec!["a.rs".to_string()],
            },
            created_at: Instant::now(),
        }
    }

    // ── push / pop basics ─────────────────────────────────────────

    #[test]
    fn push_pop_roundtrip() {
        let mut stack = UndoStack::new();
        stack.push(make_entry("first"));
        stack.push(make_entry("second"));
        assert_eq!(stack.count(), 2);

        let top = stack.pop().unwrap();
        assert_eq!(top.label, "second");
        assert_eq!(stack.count(), 1);

        let next = stack.pop().unwrap();
        assert_eq!(next.label, "first");
        assert!(stack.pop().is_none());
    }

    #[test]
    fn pop_on_empty_returns_none() {
        let mut stack = UndoStack::new();
        assert!(stack.pop().is_none());
    }

    #[test]
    fn is_empty_reflects_count() {
        let mut stack = UndoStack::new();
        assert!(stack.is_empty());
        stack.push(make_entry("x"));
        assert!(!stack.is_empty());
        stack.pop();
        assert!(stack.is_empty());
    }

    // ── capacity cap ─────────────────────────────────────────────

    #[test]
    fn cap_at_max_history() {
        let mut stack = UndoStack::new();
        for i in 0..=(MAX_UNDO_HISTORY + 5) {
            stack.push(make_entry(&format!("op-{i}")));
        }
        // Never exceeds the cap
        assert_eq!(stack.count(), MAX_UNDO_HISTORY);
    }

    #[test]
    fn oldest_entry_evicted_when_over_cap() {
        let mut stack = UndoStack::new();
        // Fill exactly to cap
        for i in 0..MAX_UNDO_HISTORY {
            stack.push(make_entry(&format!("op-{i}")));
        }
        // Add one more — evicts op-0
        stack.push(make_entry("newest"));

        // Check that op-0 is gone
        let labels: Vec<_> = stack.entries.iter().map(|e| e.label.as_str()).collect();
        assert!(!labels.contains(&"op-0"), "op-0 should have been evicted");
        assert!(labels.contains(&"newest"));
        assert_eq!(stack.count(), MAX_UNDO_HISTORY);
    }

    // ── expiry ───────────────────────────────────────────────────

    #[test]
    fn expired_entries_not_counted() {
        let mut stack = UndoStack::new();
        stack.push(make_entry("live"));
        stack.entries.push(UndoEntry::expired_entry(
            "dead",
            UndoAction::StageFiles { paths: vec![] },
        ));
        // 2 raw entries, 1 expired → count = 1
        assert_eq!(stack.entries.len(), 2);
        assert_eq!(stack.count(), 1);
    }

    #[test]
    fn pop_skips_and_removes_expired_entries() {
        let mut stack = UndoStack::new();
        stack.entries.push(UndoEntry::expired_entry(
            "old-expired",
            UndoAction::StageFiles { paths: vec![] },
        ));
        stack.push(make_entry("live"));

        let popped = stack.pop().unwrap();
        assert_eq!(popped.label, "live");
        // expired one should have been purged by pop()
        assert_eq!(stack.entries.len(), 0);
    }

    #[test]
    fn purge_expired_removes_only_stale_entries() {
        let mut stack = UndoStack::new();
        stack.push(make_entry("live-a"));
        stack.push(make_entry("live-b"));
        stack.entries.push(UndoEntry::expired_entry(
            "dead-1",
            UndoAction::ResetTo("abc".into()),
        ));
        stack
            .entries
            .push(UndoEntry::expired_entry("dead-2", UndoAction::PopStash(0)));

        let removed = stack.purge_expired();
        assert_eq!(removed, 2);
        assert_eq!(stack.count(), 2);
        let labels: Vec<_> = stack.entries.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"live-a"));
        assert!(labels.contains(&"live-b"));
    }

    // ── peek_label ───────────────────────────────────────────────

    #[test]
    fn peek_label_returns_most_recent_live_entry() {
        let mut stack = UndoStack::new();
        stack.push(make_entry("alpha"));
        stack.push(make_entry("beta"));
        // add an expired one on top
        stack.entries.push(UndoEntry::expired_entry(
            "expired-top",
            UndoAction::PopStash(0),
        ));

        // peek should skip the expired entry and see "beta"
        assert_eq!(stack.peek_label(), Some("beta"));
    }

    #[test]
    fn peek_label_none_on_empty() {
        let stack = UndoStack::new();
        assert!(stack.peek_label().is_none());
    }

    // ── action variant coverage ──────────────────────────────────

    #[test]
    fn push_pop_all_action_variants() {
        let actions = vec![
            UndoAction::PopStash(3),
            UndoAction::RecreateBranch {
                name: "feat".into(),
                oid_hex: "abc123".into(),
            },
            UndoAction::RecreateTag {
                name: "v1.0".into(),
                oid_hex: "def456".into(),
            },
            UndoAction::ResetTo("oldhead".into()),
            UndoAction::SoftResetHead("prevhead".into()),
            UndoAction::UnstageFiles {
                paths: vec!["b.rs".into()],
            },
            UndoAction::StageFiles {
                paths: vec!["c.rs".into()],
            },
        ];

        let mut stack = UndoStack::new();
        for (i, action) in actions.into_iter().enumerate() {
            stack.push(UndoEntry {
                label: format!("op-{i}"),
                action,
                created_at: Instant::now(),
            });
        }
        assert_eq!(stack.count(), 7);
        // pop all back off without panicking
        for _ in 0..7 {
            assert!(stack.pop().is_some());
        }
        assert!(stack.pop().is_none());
    }
}
