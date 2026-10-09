//! Localizable user-facing git operation errors.
//!
//! High-frequency operation failures (checkout, pull/push, stash, rebase,
//! reset, and friends) surface to the user as toasts via failed
//! `OperationUpdated` events. Their templates live as [`TrKey`]s in
//! `rgitui_settings` so the English rendering stays byte-identical to the
//! legacy `format!` strings while Chinese renders alongside.
//!
//! Dynamic values (branch names, SHAs, paths, counts) are only ever
//! substituted into `{}` placeholders — never translated. Anything not
//! modelled here (raw `git` CLI output, libgit2 internals, filesystem
//! errors) keeps its English text as the fallback.

use gpui::Context;
use rgitui_settings::{Language, SettingsState, TrKey};

use super::GitProject;

/// A user-facing git error template plus its placeholder arguments.
///
/// `Display` always renders the English template so logs, `Result` chains,
/// and the English UI stay byte-identical to the legacy `format!` strings.
/// The toast path renders through [`render_err_text`] with the current
/// interface language instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalizedOpError {
    pub key: TrKey,
    pub args: Vec<String>,
}

impl std::fmt::Display for LocalizedOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            render_op_template(Language::English, self.key, &self.args)
        )
    }
}

impl std::error::Error for LocalizedOpError {}

/// Build an [`anyhow::Error`] carrying a localizable template.
///
/// Use this wherever the code previously did
/// `anyhow::bail!("... {} ...", value)` for a user-facing failure.
pub(crate) fn op_error(key: TrKey, args: Vec<String>) -> anyhow::Error {
    anyhow::Error::new(LocalizedOpError { key, args })
}

/// Substitute `{}` placeholders in order. Mirrors the existing UI pattern of
/// chained `.replacen("{}", value, 1)` calls.
pub(crate) fn render_op_template(lang: Language, key: TrKey, args: &[String]) -> String {
    let mut out = lang.tr(key).to_string();
    for arg in args {
        let Some(pos) = out.find("{}") else {
            break;
        };
        out.replace_range(pos..pos + 2, arg);
    }
    out
}

/// The interface language for operation messages, defaulting to English when
/// no settings state is installed (e.g. bare entity tests).
pub(crate) fn op_lang(cx: &Context<GitProject>) -> Language {
    cx.try_global::<SettingsState>()
        .map(|s| s.settings().language)
        .unwrap_or_default()
}

/// Localize a failure summary or static detail template at the call site.
pub(crate) fn op_msg(cx: &Context<GitProject>, key: TrKey, args: Vec<String>) -> String {
    render_op_template(op_lang(cx), key, &args)
}

/// Localize an [`anyhow::Error`] for a failure toast.
///
/// A [`LocalizedOpError`] anywhere in the chain renders in the current
/// language; anything else (raw `git` output, libgit2/filesystem internals)
/// falls back to its English text unchanged.
pub(crate) fn op_err_text(cx: &Context<GitProject>, error: &anyhow::Error) -> String {
    render_err_text(op_lang(cx), error)
}

/// Pure core of [`op_err_text`], testable without a GPUI context.
pub(crate) fn render_err_text(lang: Language, error: &anyhow::Error) -> String {
    if let Some(localized) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<LocalizedOpError>())
    {
        return render_op_template(lang, localized.key, &localized.args);
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en(key: TrKey, args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        render_op_template(Language::English, key, &owned)
    }

    fn zh(key: TrKey, args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        render_op_template(Language::SimplifiedChinese, key, &owned)
    }

    #[test]
    fn english_failure_summaries_match_legacy_format_strings() {
        assert_eq!(en(TrKey::ErrStageFailed, &[]), "Stage failed");
        assert_eq!(en(TrKey::ErrUnstageFailed, &[]), "Unstage failed");
        assert_eq!(en(TrKey::ErrStageAllFailed, &[]), "Stage all failed");
        assert_eq!(en(TrKey::ErrUnstageAllFailed, &[]), "Unstage all failed");
        assert_eq!(en(TrKey::ErrCommitFailed, &[]), "Commit failed");
        assert_eq!(en(TrKey::ErrAmendFailed, &[]), "Amend failed");
        assert_eq!(
            en(TrKey::ErrCheckoutBranchFailedFmt, &["master"]),
            "Checkout of 'master' failed"
        );
        assert_eq!(
            en(TrKey::ErrCheckoutCommitFailedFmt, &["a1b2c3d"]),
            "Checkout of a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrCheckoutTagFailedFmt, &["v1.0"]),
            "Checkout of tag 'v1.0' failed"
        );
        assert_eq!(
            en(TrKey::ErrBranchCreateFailedFmt, &["feat"]),
            "Branch 'feat' could not be created"
        );
        assert_eq!(
            en(TrKey::ErrBranchDeleteFailedFmt, &["feat"]),
            "Delete branch 'feat' failed"
        );
        assert_eq!(
            en(TrKey::ErrBranchRenameFailedFmt, &["old"]),
            "Rename branch 'old' failed"
        );
        assert_eq!(
            en(TrKey::ErrTagCreateFailedFmt, &["v2"]),
            "Tag 'v2' could not be created"
        );
        assert_eq!(
            en(TrKey::ErrTagDeleteFailedFmt, &["v2"]),
            "Delete tag 'v2' failed"
        );
        assert_eq!(en(TrKey::ErrStashSaveFailed, &[]), "Save stash failed");
        assert_eq!(
            en(TrKey::ErrStashPopFailedFmt, &["0"]),
            "Pop stash #0 failed"
        );
        assert_eq!(
            en(TrKey::ErrStashApplyFailedFmt, &["1"]),
            "Apply stash #1 failed"
        );
        assert_eq!(
            en(TrKey::ErrStashDropFailedFmt, &["2"]),
            "Drop stash #2 failed"
        );
        assert_eq!(
            en(TrKey::ErrStashBranchFailedFmt, &["0"]),
            "Create branch from stash #0 failed"
        );
        assert_eq!(en(TrKey::ErrDiscardFailed, &[]), "Discard changes failed");
        assert_eq!(en(TrKey::ErrCleanFailed, &[]), "Clean failed");
        assert_eq!(en(TrKey::ErrResetHeadFailed, &[]), "Reset to HEAD failed");
        assert_eq!(
            en(TrKey::ErrResetFailedFmt, &["a1b2c3d"]),
            "Reset to a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrResetSoftFailedFmt, &["a1b2c3d"]),
            "Soft reset to a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrResetMixedFailedFmt, &["a1b2c3d"]),
            "Mixed reset to a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrRevertConflictFmt, &["a1b2c3d"]),
            "Revert of a1b2c3d needs conflict resolution"
        );
        assert_eq!(
            en(TrKey::ErrRevertFailedFmt, &["a1b2c3d"]),
            "Revert of a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrRevertGuide, &[]),
            "Resolve the conflicts in the working tree, then commit the revert manually."
        );
        assert_eq!(
            en(TrKey::ErrCherryPickConflictFmt, &["a1b2c3d"]),
            "Cherry-pick of a1b2c3d needs conflict resolution"
        );
        assert_eq!(
            en(TrKey::ErrCherryPickFailedFmt, &["a1b2c3d"]),
            "Cherry-pick of a1b2c3d failed"
        );
        assert_eq!(
            en(TrKey::ErrCherryPickGuide, &[]),
            "Resolve the conflicts in the working tree, then commit the cherry-pick manually."
        );
        assert_eq!(
            en(TrKey::ErrAbortFailedFmt, &["merge"]),
            "Failed to abort merge"
        );
        assert_eq!(
            en(TrKey::ErrContinueFailedFmt, &["rebase"]),
            "Could not continue rebase"
        );
        assert_eq!(
            en(TrKey::ErrMergeConflictFmt, &["feat"]),
            "Merge conflicts in 'feat'"
        );
        assert_eq!(
            en(TrKey::ErrMergeFailedFmt, &["feat"]),
            "Merge of 'feat' failed"
        );
        assert_eq!(
            en(TrKey::ErrRemoveRemoteFailed, &[]),
            "Removing remote failed"
        );
        assert_eq!(en(TrKey::ErrCloneFailed, &[]), "Clone failed");
        assert_eq!(
            en(TrKey::ErrBisectStartFailed, &[]),
            "Failed to start bisect"
        );
        assert_eq!(
            en(TrKey::ErrBisectGoodFailedFmt, &["a1b2c3d"]),
            "Failed to mark a1b2c3d as good"
        );
        assert_eq!(
            en(TrKey::ErrBisectBadFailedFmt, &["a1b2c3d"]),
            "Failed to mark a1b2c3d as bad"
        );
        assert_eq!(en(TrKey::ErrBisectExhausted, &[]), "Bisect exhausted");
        assert_eq!(
            en(TrKey::ErrBisectSkipFailedFmt, &["HEAD"]),
            "Failed to skip HEAD"
        );
        assert_eq!(
            en(TrKey::ErrBisectResetFailed, &[]),
            "Failed to reset bisect"
        );
        assert_eq!(
            en(TrKey::ErrWorktreeCreateFailedFmt, &["wt"]),
            "Create worktree 'wt' failed"
        );
        assert_eq!(
            en(TrKey::ErrWorktreeRemoveFailedFmt, &["/tmp/wt"]),
            "Remove worktree '/tmp/wt' failed"
        );
        assert_eq!(
            en(TrKey::ErrFetchFailedFmt, &["origin"]),
            "Fetch from 'origin' failed"
        );
        assert_eq!(
            en(TrKey::ErrPullConflictFmt, &["origin"]),
            "Pull from 'origin' has conflicts"
        );
        assert_eq!(
            en(TrKey::ErrPullFailedFmt, &["origin"]),
            "Pull from 'origin' failed"
        );
        assert_eq!(
            en(TrKey::ErrPushFailedFmt, &["origin"]),
            "Push to 'origin' failed"
        );
        assert_eq!(en(TrKey::ErrFetchNotStarted, &[]), "Fetch could not start");
        assert_eq!(en(TrKey::ErrPullNotStarted, &[]), "Pull could not start");
        assert_eq!(en(TrKey::ErrPushNotStarted, &[]), "Push could not start");
        assert_eq!(
            en(TrKey::ErrRebasePaused, &[]),
            "Rebase paused due to conflicts"
        );
        assert_eq!(en(TrKey::ErrRebaseFailed, &[]), "Interactive rebase failed");
        assert_eq!(en(TrKey::ErrStageHunkFailed, &[]), "Stage hunk failed");
        assert_eq!(en(TrKey::ErrUnstageHunkFailed, &[]), "Unstage hunk failed");
        assert_eq!(en(TrKey::ErrStageLinesFailed, &[]), "Stage lines failed");
        assert_eq!(
            en(TrKey::ErrUnstageLinesFailed, &[]),
            "Unstage lines failed"
        );
        assert_eq!(
            en(TrKey::ErrConflictResolveFailed, &[]),
            "Conflict resolution failed"
        );
    }

    #[test]
    fn english_error_details_match_legacy_format_strings() {
        assert_eq!(
            en(TrKey::ErrAlreadyOnBranchFmt, &["master"]),
            "Already on branch 'master'."
        );
        assert_eq!(
            en(TrKey::ErrCleanWorktreeFmt, &["Checkout"]),
            "Checkout requires a clean working tree. Commit, stash, or discard your changes to tracked files first."
        );
        assert_eq!(
            en(TrKey::ErrHeadDetached, &[]),
            "HEAD is detached. Switch to a branch before running this operation."
        );
        assert_eq!(
            en(TrKey::ErrBranchNameUnknown, &[]),
            "Failed to determine the current branch name"
        );
        assert_eq!(
            en(TrKey::ErrNoRemotes, &[]),
            "No remotes configured. Add one with: git remote add origin <url>"
        );
        assert_eq!(
            en(TrKey::ErrNoUsableRemotes, &[]),
            "No usable git remotes are configured."
        );
        assert_eq!(
            en(TrKey::ErrNoStagedChanges, &[]),
            "There are no staged changes to commit."
        );
        assert_eq!(
            en(TrKey::ErrCannotAmendRebase, &[]),
            "Cannot amend during a rebase. Continue or abort the rebase first."
        );
        assert_eq!(
            en(TrKey::ErrOverwriteLocalFmt, &["Checkout"]),
            "Checkout would overwrite local changes. Commit, stash, or discard them first."
        );
        assert_eq!(
            en(TrKey::ErrOverwritePathOneFmt, &["Checkout", "a.txt"]),
            "Checkout would overwrite a.txt. Commit, stash, move, or delete it first."
        );
        assert_eq!(
            en(
                TrKey::ErrOverwritePathManyFmt,
                &["Merge", "a.txt, b.txt and 1 more"]
            ),
            "Merge would overwrite a.txt, b.txt and 1 more. Commit, stash, move, or delete them first."
        );
        assert_eq!(
            en(TrKey::ErrStagingConflictFmt, &["a.txt"]),
            "'a.txt' has unresolved conflicts. Open the conflict resolver before staging it."
        );
        assert_eq!(
            en(TrKey::ErrBranchNotFoundFmt, &["nope"]),
            "Branch 'nope' not found as a local or remote branch. Try fetching to update remote refs."
        );
        assert_eq!(
            en(TrKey::ErrLocalBranchExistsFmt, &["feat"]),
            "A local branch named 'feat' already exists. Please delete or rename it first."
        );
        assert_eq!(
            en(TrKey::ErrInvalidRemoteBranchFmt, &["origin"]),
            "Invalid remote branch name 'origin'. Expected 'remote/branch' format."
        );
        assert_eq!(
            en(TrKey::ErrNotMergeState, &[]),
            "Repository is not in a merge state (no MERGE_HEAD to continue)."
        );
        assert_eq!(
            en(TrKey::ErrUnresolvedConflicts, &[]),
            "There are still unresolved conflicts. Resolve all conflicts before continuing."
        );
        assert_eq!(
            en(TrKey::ErrNothingToContinueFmt, &["merge"]),
            "There is no merge to continue."
        );
        assert_eq!(
            en(TrKey::ErrNoRebaseEntries, &[]),
            "No entries provided for interactive rebase"
        );
        assert_eq!(
            en(TrKey::ErrRebasePlanMismatchFmt, &["3"]),
            "Interactive rebase plan does not match the current branch's history (the selected commits are not exactly the last 3 first-parent commits of HEAD). Refresh and try again."
        );
        assert_eq!(
            en(TrKey::ErrStashIndexOutOfRangeFmt, &["9"]),
            "Stash index 9 out of range"
        );
        assert_eq!(
            en(TrKey::ErrBaseNotCommitFmt, &["nope"]),
            "'nope' does not resolve to a commit"
        );
    }

    #[test]
    fn chinese_renders_keep_dynamic_values_untranslated() {
        let rendered = zh(TrKey::ErrCheckoutBranchFailedFmt, &["master"]);
        assert_eq!(rendered, "检出 'master' 失败");
        assert!(!rendered.contains("{}"));

        let rendered = zh(TrKey::ErrAlreadyOnBranchFmt, &["master"]);
        assert_eq!(rendered, "已位于分支 'master'。");

        let rendered = zh(TrKey::ErrCleanWorktreeFmt, &["Checkout"]);
        assert!(rendered.contains("Checkout"), "{rendered}");

        let rendered = zh(TrKey::ErrRebasePlanMismatchFmt, &["3"]);
        assert!(rendered.contains('3'), "{rendered}");
        assert!(!rendered.contains("{}"), "{rendered}");

        let rendered = zh(TrKey::ErrOverwritePathManyFmt, &["Merge", "a.txt, b.txt"]);
        assert!(rendered.contains("Merge"), "{rendered}");
        assert!(rendered.contains("a.txt, b.txt"), "{rendered}");

        // Static templates render without placeholders too.
        assert!(!zh(TrKey::ErrHeadDetached, &[]).is_empty());
        assert!(!zh(TrKey::ErrRebaseFailed, &[]).contains("{}"));
    }

    #[test]
    fn localized_error_displays_english_and_renders_current_language() {
        let error = op_error(TrKey::ErrAlreadyOnBranchFmt, vec!["master".to_string()]);
        assert_eq!(error.to_string(), "Already on branch 'master'.");
        assert_eq!(
            render_err_text(Language::English, &error),
            "Already on branch 'master'."
        );
        assert_eq!(
            render_err_text(Language::SimplifiedChinese, &error),
            "已位于分支 'master'。"
        );
    }

    #[test]
    fn plain_errors_fall_back_to_english_unchanged() {
        let error = anyhow::anyhow!("git pull failed: some raw stderr line");
        assert_eq!(
            render_err_text(Language::SimplifiedChinese, &error),
            "git pull failed: some raw stderr line"
        );
        assert_eq!(
            render_err_text(Language::English, &error),
            "git pull failed: some raw stderr line"
        );
    }

    #[test]
    fn localized_cause_inside_context_still_translates() {
        let error = op_error(TrKey::ErrNoStagedChanges, vec![]);
        let wrapped = error.context("commit preflight");
        assert_eq!(
            render_err_text(Language::SimplifiedChinese, &wrapped),
            "没有已暂存的更改可提交。"
        );
    }
}
