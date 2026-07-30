//! Turning a graph multi-selection into an interactive-rebase plan that squashes
//! the selected commits together.
//!
//! [`GitProject::rebase_interactive`](rgitui_git::GitProject::rebase_interactive)
//! accepts a plan only when its commit set is *exactly* HEAD's contiguous
//! first-parent range `base..HEAD`, and hard-fails otherwise — it derives the
//! range from HEAD itself rather than from the plan, so a cross-branch or stale
//! plan cannot replay commits from an unrelated branch. The commit graph, on the
//! other hand, lists every ref in date order, so a plausible-looking selection
//! can easily straddle two branches.
//!
//! Everything here is therefore validated *before* anything is executed, and the
//! rejection carries an actionable message. It is all pure — no gpui, no git2
//! repository — so the rules are unit-tested without a display or a fixture repo.

use git2::Oid;
use rgitui_git::CommitInfo;

use crate::interactive_rebase::{RebaseAction, RebaseEntry};

/// A validated squash, ready to pre-fill the interactive rebase dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SquashPlan {
    /// Every commit that will be replayed, newest first — the order the dialog
    /// lists them in. Covers HEAD down to the oldest selected commit inclusive.
    pub entries: Vec<RebaseEntry>,
    /// Short id of the commit the range is replayed onto, for the dialog's title.
    pub base_label: String,
}

/// Why a selection cannot be squashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SquashRejection {
    /// Fewer than two distinct commits are selected.
    TooFewSelected(usize),
    /// At least one selected commit is not on HEAD's first-parent chain.
    NotOnHeadChain,
    /// The selected commits are all on HEAD's chain but skip one in between.
    NotContiguous,
    /// A merge commit sits inside the range that would be rewritten.
    CrossesMerge(String),
    /// The range reaches the first commit in the repository, which has no parent
    /// to rebase onto.
    ReachesRootCommit,
}

impl SquashRejection {
    /// An actionable, user-facing explanation for a toast.
    pub(crate) fn message(&self) -> String {
        match self {
            SquashRejection::TooFewSelected(_) => "Select two or more commits to squash them: \
                 hold shift to extend the selection by row, or the primary modifier to add \
                 single rows."
                .to_owned(),
            SquashRejection::NotOnHeadChain => "Squash only rewrites the checked-out branch. \
                 The graph lists every branch in date order, so at least one selected commit is \
                 not on this branch's history — select commits from the current branch instead."
                .to_owned(),
            SquashRejection::NotContiguous => "Squash needs an unbroken run of commits. The \
                 selection skips at least one commit in between: either include it, or narrow \
                 the selection."
                .to_owned(),
            SquashRejection::CrossesMerge(short_id) => format!(
                "Squash would have to replay merge commit {short_id}, which sits between the \
                 selection and HEAD. Select commits newer than that merge instead."
            ),
            SquashRejection::ReachesRootCommit => "Squash cannot include the repository's first \
                 commit, which has no parent to rebase onto."
                .to_owned(),
        }
    }
}

/// HEAD's first-parent chain, newest first, restricted to the loaded commits.
///
/// Stops at the first commit the graph has not loaded: the plan may only name
/// commits that are actually on screen, and the rebase layer would reject a plan
/// referring to anything else anyway.
fn first_parent_chain(commits: &[CommitInfo], head_oid: Oid) -> Vec<&CommitInfo> {
    let mut chain: Vec<&CommitInfo> = Vec::new();
    let mut next = Some(head_oid);
    while let Some(oid) = next {
        let Some(commit) = commits.iter().find(|commit| commit.oid == oid) else {
            break;
        };
        chain.push(commit);
        // A history longer than the list it is drawn from means the parent links
        // loop; bail rather than spinning on the UI thread.
        if chain.len() > commits.len() {
            break;
        }
        next = commit.parent_oids.first().copied();
    }
    chain
}

/// Builds the plan that squashes `selected` together, or explains why it cannot.
///
/// `commits` is the graph's display list (date-ordered, every ref), `head_oid`
/// the commit the branch is actually on. On success the plan replays HEAD down to
/// the oldest selected commit: the oldest selected commit stays a `pick` and the
/// rest of the selection becomes `squash`, because git melds a `squash` into the
/// todo line above it. Commits newer than the selection are replayed unchanged.
pub(crate) fn plan_squash(
    commits: &[CommitInfo],
    head_oid: Oid,
    selected: &[Oid],
) -> Result<SquashPlan, SquashRejection> {
    if selected.len() < 2 {
        return Err(SquashRejection::TooFewSelected(selected.len()));
    }

    let chain = first_parent_chain(commits, head_oid);
    let mut positions = Vec::with_capacity(selected.len());
    for oid in selected {
        let Some(position) = chain.iter().position(|commit| commit.oid == *oid) else {
            return Err(SquashRejection::NotOnHeadChain);
        };
        positions.push(position);
    }
    positions.sort_unstable();
    positions.dedup();
    if positions.len() < 2 {
        return Err(SquashRejection::TooFewSelected(positions.len()));
    }

    // The chain runs newest first, so the smallest position is the newest commit.
    let newest = positions[0];
    let oldest = positions[positions.len() - 1];
    if oldest - newest + 1 != positions.len() {
        return Err(SquashRejection::NotContiguous);
    }

    let covered = &chain[..=oldest];
    if let Some(merge) = covered
        .iter()
        .find(|commit| commit.parent_oids.len() > 1)
        .map(|commit| commit.short_id.clone())
    {
        return Err(SquashRejection::CrossesMerge(merge));
    }
    let base_oid = covered
        .last()
        .and_then(|commit| commit.parent_oids.first())
        .copied();
    let Some(base_oid) = base_oid else {
        return Err(SquashRejection::ReachesRootCommit);
    };

    let entries = covered
        .iter()
        .enumerate()
        .map(|(position, commit)| RebaseEntry {
            oid: commit.oid.to_string(),
            original_message: commit.summary.clone(),
            author: commit.author.name.clone(),
            // The oldest selected commit carries the squash, so it stays a pick.
            action: if position >= newest && position < oldest {
                RebaseAction::Squash
            } else {
                RebaseAction::Pick
            },
        })
        .collect();

    // The base itself is usually loaded, which gives a nicer label than an abbrev
    // of the raw OID; fall back to that when it is out of the loaded window.
    let base_label = commits
        .iter()
        .find(|commit| commit.oid == base_oid)
        .map(|commit| commit.short_id.clone())
        .unwrap_or_else(|| base_oid.to_string()[..7].to_owned());

    Ok(SquashPlan {
        entries,
        base_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rgitui_git::{RefLabel, Signature};

    fn oid(byte: u8) -> Oid {
        let mut bytes = [0_u8; 20];
        bytes[0] = byte;
        Oid::from_bytes(&bytes).unwrap()
    }

    fn commit(id: u8, parents: &[u8]) -> CommitInfo {
        CommitInfo {
            oid: oid(id),
            short_id: format!("{id:07x}"),
            summary: format!("Commit {id}"),
            message: format!("Commit {id}"),
            author: Signature {
                name: "Test".to_owned(),
                email: "test@example.com".to_owned(),
            },
            committer: Signature {
                name: "Test".to_owned(),
                email: "test@example.com".to_owned(),
            },
            co_authors: Vec::new(),
            time: Utc::now(),
            parent_oids: parents.iter().map(|parent| oid(*parent)).collect(),
            refs: Vec::new(),
            is_signed: false,
        }
    }

    /// A linear branch: 5 (HEAD) -> 4 -> 3 -> 2 -> 1, plus a root commit 0 so the
    /// range never runs off the end of history.
    fn linear_history() -> Vec<CommitInfo> {
        vec![
            commit(5, &[4]),
            commit(4, &[3]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[0]),
            commit(0, &[]),
        ]
    }

    fn actions(plan: &SquashPlan) -> Vec<(&str, &RebaseAction)> {
        plan.entries
            .iter()
            .map(|entry| (entry.oid.as_str(), &entry.action))
            .collect()
    }

    #[test]
    fn three_of_five_commits_squash_into_the_oldest_selected() {
        let commits = linear_history();
        // Select 4, 3 and 2 — the middle of the branch, with 5 newer than them.
        let plan = plan_squash(&commits, oid(5), &[oid(4), oid(3), oid(2)])
            .expect("a contiguous run on HEAD's chain is squashable");

        // The plan covers HEAD down to the oldest selected commit, newest first.
        assert_eq!(
            actions(&plan),
            vec![
                (oid(5).to_string().as_str(), &RebaseAction::Pick),
                (oid(4).to_string().as_str(), &RebaseAction::Squash),
                (oid(3).to_string().as_str(), &RebaseAction::Squash),
                // `squash` melds into the line above, so the oldest selected
                // commit stays the pick that carries the others.
                (oid(2).to_string().as_str(), &RebaseAction::Pick),
            ]
        );
        // Nothing older than the selection is replayed.
        assert_eq!(plan.entries.len(), 4);
        // Replayed onto the parent of the oldest covered commit.
        assert_eq!(plan.base_label, commits[4].short_id);
    }

    #[test]
    fn selecting_the_two_newest_commits_squashes_them_alone() {
        let commits = linear_history();
        let plan = plan_squash(&commits, oid(5), &[oid(5), oid(4)]).expect("squashable");
        assert_eq!(
            actions(&plan),
            vec![
                (oid(5).to_string().as_str(), &RebaseAction::Squash),
                (oid(4).to_string().as_str(), &RebaseAction::Pick),
            ]
        );
    }

    #[test]
    fn the_selection_order_does_not_matter() {
        let commits = linear_history();
        let ascending = plan_squash(&commits, oid(5), &[oid(2), oid(3), oid(4)]).unwrap();
        let descending = plan_squash(&commits, oid(5), &[oid(4), oid(3), oid(2)]).unwrap();
        assert_eq!(ascending, descending);
    }

    #[test]
    fn a_gap_in_the_selection_is_rejected() {
        let commits = linear_history();
        // 4 and 2 are on the chain but 3 sits between them.
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(4), oid(2)]),
            Err(SquashRejection::NotContiguous)
        );
    }

    #[test]
    fn fewer_than_two_commits_cannot_be_squashed() {
        let commits = linear_history();
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(4)]),
            Err(SquashRejection::TooFewSelected(1))
        );
        assert_eq!(
            plan_squash(&commits, oid(5), &[]),
            Err(SquashRejection::TooFewSelected(0))
        );
        // The same commit twice is still one commit.
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(4), oid(4)]),
            Err(SquashRejection::TooFewSelected(1))
        );
    }

    /// The graph is date-ordered across every ref, so a run of adjacent *rows*
    /// can easily include a commit from another branch.
    #[test]
    fn a_commit_from_another_branch_is_rejected() {
        // 5 (HEAD) -> 4 -> 1, and 9 -> 1 on a side branch that the date order
        // interleaves between 5 and 4.
        let commits = vec![
            commit(5, &[4]),
            commit(9, &[1]),
            commit(4, &[1]),
            commit(1, &[0]),
            commit(0, &[]),
        ];
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(5), oid(9)]),
            Err(SquashRejection::NotOnHeadChain)
        );
        // Its own two commits are fine, because both are on HEAD's chain.
        assert!(plan_squash(&commits, oid(5), &[oid(5), oid(4)]).is_ok());
    }

    #[test]
    fn a_commit_that_is_not_loaded_is_rejected() {
        let commits = linear_history();
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(4), oid(42)]),
            Err(SquashRejection::NotOnHeadChain)
        );
    }

    #[test]
    fn a_merge_inside_the_replayed_range_is_rejected() {
        // 5 (HEAD) is a merge of 4 and 9; squashing 3 into 2 would have to replay
        // it, which `git rebase -i` would flatten.
        let commits = vec![
            commit(5, &[4, 9]),
            commit(9, &[3]),
            commit(4, &[3]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        assert_eq!(
            plan_squash(&commits, oid(5), &[oid(3), oid(2)]),
            Err(SquashRejection::CrossesMerge(commits[0].short_id.clone()))
        );
    }

    #[test]
    fn a_range_reaching_the_root_commit_is_rejected() {
        let commits = vec![commit(3, &[2]), commit(2, &[1]), commit(1, &[])];
        assert_eq!(
            plan_squash(&commits, oid(3), &[oid(2), oid(1)]),
            Err(SquashRejection::ReachesRootCommit)
        );
    }

    /// The base is only a label, so an unloaded parent must not fail the plan.
    #[test]
    fn an_unloaded_base_falls_back_to_the_abbreviated_oid() {
        // The parent of commit 1 is not in the list.
        let commits = vec![commit(3, &[2]), commit(2, &[1]), commit(1, &[7])];
        let plan = plan_squash(&commits, oid(3), &[oid(2), oid(1)]).unwrap();
        assert_eq!(plan.base_label, oid(7).to_string()[..7]);
    }

    /// The plan must match what `rebase_interactive` recomputes from HEAD: exactly
    /// the last N first-parent commits, one entry each, no duplicates.
    #[test]
    fn the_plan_matches_heads_first_parent_range_exactly() {
        let commits = linear_history();
        let plan = plan_squash(&commits, oid(5), &[oid(3), oid(2)]).unwrap();

        let expected: Vec<String> = [5_u8, 4, 3, 2]
            .iter()
            .map(|id| oid(*id).to_string())
            .collect();
        let planned: Vec<String> = plan.entries.iter().map(|e| e.oid.clone()).collect();
        assert_eq!(planned, expected);
    }

    #[test]
    fn a_refs_label_on_head_does_not_affect_the_plan() {
        // The chain is walked from the OID, not from the HEAD ref label.
        let mut commits = linear_history();
        commits[2].refs = vec![RefLabel::Head];
        let plan = plan_squash(&commits, oid(5), &[oid(5), oid(4)]).unwrap();
        assert_eq!(plan.entries.len(), 2);
    }

    #[test]
    fn every_rejection_explains_itself() {
        for rejection in [
            SquashRejection::TooFewSelected(1),
            SquashRejection::NotOnHeadChain,
            SquashRejection::NotContiguous,
            SquashRejection::CrossesMerge("abc1234".to_owned()),
            SquashRejection::ReachesRootCommit,
        ] {
            let message = rejection.message();
            assert!(message.ends_with('.'), "{message}");
            assert!(!message.contains("  "), "double space in {message}");
        }
    }
}
