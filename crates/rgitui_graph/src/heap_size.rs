//! [`HeapSize`] for the commit graph's retained state.
//!
//! The graph is the view whose memory scales hardest with the repository: one
//! [`CommitInfo`] and one [`GraphRow`] per commit in the loaded window, each
//! row carrying its own edge allocation. Beside those, the selection and
//! search sets are indices and the worktree nodes are one per checkout, so the
//! census's job here is less about finding hidden strings than about stating
//! plainly what the loaded window costs — and about not counting it twice,
//! since the commit snapshot is shared with `GitProject` rather than copied.

use std::collections::HashSet;
use std::sync::Arc;

use rgitui_git::{CommitInfo, GraphRow};
use rgitui_perf::{Census, HeapSize};

use crate::{GraphView, WorktreeGraphInfo, WorktreeRowPosition};

/// A worktree's pseudo-node carries a name, a path and a per-change-kind
/// tally. There is one per worktree rather than one per commit, so this is a
/// small population — measured anyway, because a node missing from the report
/// is indistinguishable from a node that cost nothing.
impl HeapSize for WorktreeGraphInfo {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.name.heap_size(census)
            + self.combined_breakdown.heap_size(census)
            + self.worktree_path.heap_size(census)
            + self.branch.heap_size(census)
    }
}

/// Every field is a lane, a row index or a colour index, so a position owns
/// nothing past the buffer its vector already accounts for.
impl HeapSize for WorktreeRowPosition {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// The parts of a [`GraphView`] the census measures.
///
/// Held apart from the view so the node labels can be checked without opening
/// a window: a `GraphView` can only be built inside a gpui context, and the
/// labels are the report's primary key — a run-to-run comparison keys off
/// them, so one that changes silently turns a comparison into two unrelated
/// reports.
struct GraphViewCensus<'a> {
    commits: &'a Arc<Vec<CommitInfo>>,
    graph_rows: &'a Arc<Vec<GraphRow>>,
    selected_rows: &'a Arc<HashSet<usize>>,
    filter_matches: &'a Vec<usize>,
    filter_match_set: &'a Arc<HashSet<usize>>,
    virtual_rows_prefix: &'a Arc<Vec<usize>>,
    worktree_infos: &'a Vec<WorktreeGraphInfo>,
    worktree_row_positions: &'a Vec<WorktreeRowPosition>,
    worktree_rows: &'a Vec<usize>,
}

impl<'a> From<&'a GraphView> for GraphViewCensus<'a> {
    fn from(view: &'a GraphView) -> Self {
        Self {
            commits: &view.commits,
            graph_rows: &view.graph_rows,
            selected_rows: &view.selected_rows,
            filter_matches: &view.filter_matches,
            filter_match_set: &view.filter_match_set_arc,
            virtual_rows_prefix: &view.virtual_rows_prefix,
            worktree_infos: &view.worktree_infos,
            worktree_row_positions: &view.worktree_row_positions,
            worktree_rows: &view.worktree_rows,
        }
    }
}

impl HeapSize for GraphViewCensus<'_> {
    fn heap_size(&self, census: &mut Census) -> usize {
        census.enter("commits", self.commits)
            + census.enter("rows", self.graph_rows)
            + census.enter("selected_rows", self.selected_rows)
            + census.enter("filter_matches", self.filter_matches)
            + census.enter("filter_match_set", self.filter_match_set)
            + census.enter("virtual_rows_prefix", self.virtual_rows_prefix)
            + census.enter("worktrees", self.worktree_infos)
            + census.enter("worktree_positions", self.worktree_row_positions)
            + census.enter("worktree_rows", self.worktree_rows)
    }
}

impl GraphView {
    /// Records this view's retained memory under the current census path,
    /// returning the bytes attributed so an enclosing scope can roll them up.
    ///
    /// `commits` is the same allocation `GitProject` holds, and the `Arc` impl
    /// charges whichever path reaches it first. Whether the snapshot lands
    /// under the project or under the graph is therefore a property of the
    /// walk order, not of the data; what matters is that it appears once, so
    /// the total stays equal to what the process allocated.
    ///
    /// The selection's `BTreeSet`s are the one thing left out: `selected_rows`
    /// is the same membership projected onto row indices and is measured, and
    /// a second copy of at most one index per commit would add noise without
    /// adding a decision.
    pub fn census(&self, census: &mut Census) -> usize {
        GraphViewCensus::from(self).heap_size(census)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use rgitui_git::FileChangeKind;

    use super::*;

    fn worktree_info(name: &str) -> WorktreeGraphInfo {
        let mut combined_breakdown = HashMap::new();
        combined_breakdown.insert(FileChangeKind::Modified, 3);
        WorktreeGraphInfo {
            name: name.to_string(),
            is_current: true,
            head_oid: Some(git2::Oid::zero()),
            staged_count: 1,
            unstaged_count: 2,
            combined_breakdown,
            worktree_path: PathBuf::from("/repos/rgitui"),
            branch: Some("main".to_string()),
            head_detached: false,
        }
    }

    fn row_position() -> WorktreeRowPosition {
        WorktreeRowPosition {
            worktree_idx: 0,
            list_index: 0,
            commit_index: Some(0),
            node_lane: 1,
            parent_lane: Some(0),
            color_index: 2,
        }
    }

    /// Owns what a [`GraphViewCensus`] borrows, so a test can assemble one
    /// without a gpui context to build a real [`GraphView`] in.
    #[derive(Default)]
    struct Fields {
        commits: Arc<Vec<CommitInfo>>,
        graph_rows: Arc<Vec<GraphRow>>,
        selected_rows: Arc<HashSet<usize>>,
        filter_matches: Vec<usize>,
        filter_match_set: Arc<HashSet<usize>>,
        virtual_rows_prefix: Arc<Vec<usize>>,
        worktree_infos: Vec<WorktreeGraphInfo>,
        worktree_row_positions: Vec<WorktreeRowPosition>,
        worktree_rows: Vec<usize>,
    }

    impl Fields {
        fn view(&self) -> GraphViewCensus<'_> {
            GraphViewCensus {
                commits: &self.commits,
                graph_rows: &self.graph_rows,
                selected_rows: &self.selected_rows,
                filter_matches: &self.filter_matches,
                filter_match_set: &self.filter_match_set,
                virtual_rows_prefix: &self.virtual_rows_prefix,
                worktree_infos: &self.worktree_infos,
                worktree_row_positions: &self.worktree_row_positions,
                worktree_rows: &self.worktree_rows,
            }
        }
    }

    #[test]
    fn a_worktree_info_is_charged_for_its_strings_and_its_breakdown_map() {
        let mut census = Census::new();
        let info = worktree_info("feature-branch");

        let strings = "feature-branch".len() + "/repos/rgitui".len() + "main".len();
        let table = info.combined_breakdown.capacity() * (size_of::<(FileChangeKind, usize)>() + 1);

        assert!(table > 0);
        assert_eq!(info.heap_size(&mut census), strings + table);
    }

    #[test]
    fn worktree_positions_cost_only_the_buffer_that_holds_them() {
        let mut census = Census::new();
        let positions = vec![row_position(), row_position()];

        assert_eq!(
            positions.heap_size(&mut census),
            positions.capacity() * size_of::<WorktreeRowPosition>()
        );
    }

    #[test]
    fn the_graph_census_emits_one_node_per_measured_field() {
        let fields = Fields::default();
        let mut census = Census::new();
        census.enter("graph", &fields.view());

        let mut paths: Vec<String> = census
            .into_nodes()
            .into_iter()
            .map(|node| node.path)
            .collect();
        paths.sort();

        assert_eq!(
            paths,
            [
                "graph",
                "graph/commits",
                "graph/filter_match_set",
                "graph/filter_matches",
                "graph/rows",
                "graph/selected_rows",
                "graph/virtual_rows_prefix",
                "graph/worktree_positions",
                "graph/worktree_rows",
                "graph/worktrees",
            ]
        );
    }

    #[test]
    fn the_graph_census_reports_the_populations_it_walked() {
        let fields = Fields {
            filter_matches: vec![1, 2, 3],
            filter_match_set: Arc::new(HashSet::from([1, 2, 3])),
            virtual_rows_prefix: Arc::new(vec![0]),
            worktree_infos: vec![worktree_info("main")],
            worktree_row_positions: vec![row_position()],
            worktree_rows: vec![0],
            ..Default::default()
        };

        let mut census = Census::new();
        let bytes = census.enter("graph", &fields.view());
        let nodes = census.into_nodes();

        let node = |path: &str| {
            nodes
                .iter()
                .find(|node| node.path == path)
                .unwrap_or_else(|| panic!("{path} is missing from the census"))
        };

        assert!(bytes > 0);
        assert_eq!(node("graph/filter_matches").count, Some(3));
        assert_eq!(node("graph/worktrees").count, Some(1));
        assert!(node("graph/worktrees").bytes > 0);
    }
}
