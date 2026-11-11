//! Git's autostash, for the operations rgitui concludes or aborts itself.
//!
//! `git pull --autostash` — and a plain pull under `merge.autoStash` or
//! `rebase.autoStash` — stashes uncommitted changes, integrates the remote
//! branch, and reapplies them. When the pull stops on a conflict the stash is
//! *not* in the stash list: git parks the stash commit's id in a file inside
//! the git directory and reapplies it when git itself concludes or aborts the
//! operation.
//!
//! rgitui commits and aborts a merge, and resets, with libgit2, which knows
//! nothing about that file. Left alone, a concluded or aborted merge would
//! leave `MERGE_AUTOSTASH` behind and a reset would delete a rebase's copy with
//! the rest of its state — either way the user's changes would be reachable
//! only by object id. So each of those paths reads the parked stash first with
//! [`pending_autostash`] and then does what git would have done at that point:
//! [`restore_autostash`] after concluding or aborting (apply it, and if that
//! conflicts, put it in the stash list where it can be seen and popped), or
//! [`keep_pending_autostash`] before a reset (straight to the stash list).
//! A rebase is continued and aborted by git itself, which handles its own.

use std::path::{Path, PathBuf};

use anyhow::Result;
use git2::Repository;
use gpui::Context;
use rgitui_i18n::msg;

use crate::types::*;

use super::messages::error;
use super::GitProject;

/// Where git parks an autostash while an operation is stopped, relative to the
/// checkout's git directory. A merge keeps its own file; a rebase keeps one
/// inside whichever state directory its backend uses.
const AUTOSTASH_FILES: [&str; 3] = [
    "MERGE_AUTOSTASH",
    "rebase-merge/autostash",
    "rebase-apply/autostash",
];

/// The message git gives an autostash it had to store in the stash list. Used
/// here too, so the entry reads the same whoever stored it.
const AUTOSTASH_MESSAGE: &str = "autostash";

/// A stash git made before an operation that is now stopped part-way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingAutostash {
    oid: git2::Oid,
    /// The file that records it.
    file: PathBuf,
}

/// How reapplying an autostash went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AutostashRestore {
    /// The changes are back in the working tree.
    Applied,
    /// Reapplying conflicted. The working tree holds the conflicts and the
    /// stash is in the stash list.
    Conflicted,
    /// Neither applying nor storing worked. Nothing is lost — the commit still
    /// exists — but it can only be reached by `commit`.
    Unrecoverable { commit: String },
}

/// The autostash recorded for the operation stopped in `repo`, if there is one.
pub(super) fn pending_autostash(repo: &Repository) -> Option<PendingAutostash> {
    AUTOSTASH_FILES.iter().find_map(|name| {
        let file = repo.path().join(name);
        let contents = std::fs::read_to_string(&file).ok()?;
        let oid = git2::Oid::from_str(contents.trim()).ok()?;
        Some(PendingAutostash { oid, file })
    })
}

/// Reapply `pending` in `worktree_path`, exactly as git does when it concludes
/// the operation itself: `git stash apply`, and on failure `git stash store` so
/// the changes are in the stash list rather than known only by object id.
///
/// Call it only after the operation's own state has been cleared; the apply
/// runs against whatever the working tree holds by then.
pub(super) fn restore_autostash(
    worktree_path: &Path,
    pending: &PendingAutostash,
) -> AutostashRestore {
    let commit = pending.oid.to_string();
    let restore = if run_git(worktree_path, &["stash", "apply", &commit]) {
        AutostashRestore::Applied
    } else if run_git(
        worktree_path,
        &["stash", "store", "-m", AUTOSTASH_MESSAGE, "-q", &commit],
    ) {
        AutostashRestore::Conflicted
    } else {
        // Leave the file where it is: it is the only record of the commit.
        log::error!("could not apply or store autostash {commit}");
        return AutostashRestore::Unrecoverable { commit };
    };
    forget(pending);
    restore
}

/// Move the autostash parked for the operation stopped in `repo` into the
/// stash list, before a reset that discards that operation.
///
/// A mixed or hard reset in libgit2 clears merge and rebase state, the rebase's
/// record of its autostash included, without restoring anything. Git's own
/// reset stores the autostash as a stash entry at that point rather than
/// reapplying it over a tree the user has just asked to reset; this does the
/// same. If the stash cannot be stored the reset must not go ahead.
pub(super) fn keep_pending_autostash(repo: &Repository, worktree_path: &Path) -> Result<()> {
    let Some(pending) = pending_autostash(repo) else {
        return Ok(());
    };
    let commit = pending.oid.to_string();
    if !run_git(
        worktree_path,
        &["stash", "store", "-m", AUTOSTASH_MESSAGE, "-q", &commit],
    ) {
        return Err(error(msg!(
            "git.autostash.restore_failed_detail",
            commit = &commit
        )));
    }
    forget(&pending);
    Ok(())
}

fn run_git(worktree_path: &Path, args: &[&str]) -> bool {
    super::git_command()
        .current_dir(worktree_path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Remove the file recording `pending`, once the stash is safely somewhere
/// else, so git does not reapply it a second time later.
fn forget(pending: &PendingAutostash) {
    // An abort has usually removed a rebase's file along with its directory.
    if let Err(error) = std::fs::remove_file(&pending.file) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!(
                "could not remove {} after restoring the autostash: {error}",
                pending.file.display()
            );
        }
    }
}

impl GitProject {
    /// Report how reapplying git's autostash went, as an operation of its own.
    ///
    /// It is reported apart from the operation that triggered it because the
    /// two outcomes differ: a merge can be committed, or an operation aborted,
    /// perfectly well while the changes stashed before it come back with
    /// conflicts — and that needs to stay on screen after the success fades.
    pub(super) fn report_autostash_restore(
        &mut self,
        restore: AutostashRestore,
        branch_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let id = self.allocate_operation_id();
        match restore {
            AutostashRestore::Applied => self.complete_op(
                id,
                GitOperationKind::Stash,
                msg!("git.autostash.restored"),
                (None, None, branch_name),
                cx,
            ),
            AutostashRestore::Conflicted => self.fail_op(
                id,
                GitOperationKind::Stash,
                msg!("git.autostash.conflict"),
                msg!("git.autostash.conflict_detail"),
                (None, branch_name, false),
                cx,
            ),
            AutostashRestore::Unrecoverable { commit } => self.fail_op(
                id,
                GitOperationKind::Stash,
                msg!("git.autostash.restore_failed"),
                msg!("git.autostash.restore_failed_detail", commit = &commit),
                (None, branch_name, false),
                cx,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgitui_test_support::TempRepo;

    fn git(repo: &TempRepo, args: &[&str]) -> String {
        let output = super::super::git_command()
            .current_dir(repo.path())
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A repository with one commit, and an uncommitted edit to `notes.txt`
    /// taken out of the working tree into a stash commit that is in no list —
    /// the state git leaves an autostash in.
    fn repo_with_parked_stash(parked_in: &str) -> (TempRepo, PendingAutostash) {
        let repo = TempRepo::init();
        repo.commit_file("notes.txt", "one\n", "add notes");
        repo.write_file("notes.txt", "one\nmine\n");
        let commit = git(&repo, &["stash", "create"]);
        git(&repo, &["checkout", "--", "notes.txt"]);

        let file = repo.repo().path().join(parked_in);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("{commit}\n")).unwrap();

        let pending = pending_autostash(repo.repo()).expect("the parked stash is found");
        assert_eq!(pending.oid.to_string(), commit);
        (repo, pending)
    }

    #[test]
    fn nothing_is_pending_in_an_ordinary_repository() {
        let repo = TempRepo::init();
        repo.commit_file("notes.txt", "one\n", "add notes");
        assert_eq!(pending_autostash(repo.repo()), None);
    }

    #[test]
    fn a_stash_parked_by_a_merge_or_either_rebase_backend_is_found() {
        for parked_in in AUTOSTASH_FILES {
            let (_repo, pending) = repo_with_parked_stash(parked_in);
            assert!(pending.file.ends_with(parked_in));
        }
    }

    #[test]
    fn a_file_that_does_not_hold_an_object_id_is_ignored() {
        let repo = TempRepo::init();
        repo.commit_file("notes.txt", "one\n", "add notes");
        std::fs::write(repo.repo().path().join("MERGE_AUTOSTASH"), "not an oid\n").unwrap();
        assert_eq!(pending_autostash(repo.repo()), None);
    }

    #[test]
    fn restoring_puts_the_changes_back_and_forgets_the_parked_stash() {
        let (repo, pending) = repo_with_parked_stash("MERGE_AUTOSTASH");

        assert_eq!(
            restore_autostash(repo.path(), &pending),
            AutostashRestore::Applied
        );

        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
            "one\nmine\n"
        );
        assert!(
            !pending.file.exists(),
            "git would reapply the stash a second time at the next commit"
        );
        assert_eq!(git(&repo, &["stash", "list"]), "");
    }

    #[test]
    fn a_conflicting_restore_keeps_the_changes_in_the_stash_list() {
        let (repo, pending) = repo_with_parked_stash("MERGE_AUTOSTASH");
        // The same line changed underneath the stash, as a pulled commit would.
        repo.commit_file("notes.txt", "one\ntheirs\n", "their change");

        assert_eq!(
            restore_autostash(repo.path(), &pending),
            AutostashRestore::Conflicted
        );

        let stashes = git(&repo, &["stash", "list"]);
        assert!(
            stashes.contains(AUTOSTASH_MESSAGE),
            "the changes must be recoverable from the stash list, got {stashes:?}"
        );
        assert_eq!(
            git(&repo, &["rev-parse", "refs/stash"]),
            pending.oid.to_string()
        );
        assert!(!pending.file.exists());
    }

    #[test]
    fn keeping_a_pending_autostash_moves_it_to_the_stash_list_untouched() {
        let (repo, pending) = repo_with_parked_stash("rebase-merge/autostash");

        keep_pending_autostash(repo.repo(), repo.path()).unwrap();

        assert_eq!(
            git(&repo, &["rev-parse", "refs/stash"]),
            pending.oid.to_string()
        );
        assert!(!pending.file.exists());
        // Stored, not applied: the tree is about to be reset.
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn keeping_nothing_is_not_an_error() {
        let repo = TempRepo::init();
        repo.commit_file("notes.txt", "one\n", "add notes");
        keep_pending_autostash(repo.repo(), repo.path()).unwrap();
        assert_eq!(git(&repo, &["stash", "list"]), "");
    }

    #[test]
    fn a_stash_that_cannot_be_applied_or_stored_is_left_recorded() {
        let (repo, mut pending) = repo_with_parked_stash("MERGE_AUTOSTASH");
        // An id no object has: both `stash apply` and `stash store` refuse it.
        pending.oid = git2::Oid::from_str("1234567890123456789012345678901234567890").unwrap();

        assert_eq!(
            restore_autostash(repo.path(), &pending),
            AutostashRestore::Unrecoverable {
                commit: pending.oid.to_string()
            }
        );
        assert!(
            pending.file.exists(),
            "the file is the only record of where the changes are"
        );
    }
}
