use std::collections::{HashMap, HashSet};

use crate::{CommitInfo, RefLabel};

/// The visual representation of a commit's position in the graph.
#[derive(Debug, Clone)]
pub struct GraphRow {
    /// Index of the commit in the commit list.
    pub commit_index: usize,
    /// The lane (column) this commit's node sits in.
    pub node_lane: usize,
    /// Connections drawn in this row.
    pub edges: Vec<GraphEdge>,
    /// Total number of active lanes at this row.
    pub lane_count: usize,
    /// Color index for this commit's dot (derived from lane color).
    pub node_color: usize,
    /// Whether there is an incoming edge from the row above (false for branch tips).
    pub has_incoming: bool,
}

/// An edge connecting two rows in the graph.
#[derive(Debug, Clone)]
pub struct GraphEdge {
    /// The lane this edge starts from (in the previous row).
    pub from_lane: usize,
    /// The lane this edge goes to (in this row).
    pub to_lane: usize,
    /// Color index for this edge (consistent per branch).
    pub color_index: usize,
    /// Whether this edge represents a merge (connecting to a non-primary parent).
    pub is_merge: bool,
}

/// Compute the graph layout for a list of commits.
///
/// Each lane tracks `(expected_oid, color_index)`. Colors are assigned per-lane
/// so that a continuous branch always keeps the same color. When a lane is freed
/// and reused, the new occupant gets a fresh color.
///
/// Branch tips that are not yet merged are pre-assigned to separate lanes so
/// that branches visually diverge from their base (e.g. main) instead of
/// appearing as a single linear lane.
pub fn compute_graph(commits: &[CommitInfo]) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    // Build a set of all commit OIDs for quick lookup
    let oid_set: HashSet<git2::Oid> = commits.iter().map(|c| c.oid).collect();

    // Build a map: parent_oid -> list of child commits that have it as first parent.
    // This helps us detect fork points where multiple branches diverge.
    let mut children_of: HashMap<git2::Oid, Vec<usize>> = HashMap::new();
    for (idx, commit) in commits.iter().enumerate() {
        if let Some(&parent) = commit.parent_oids.first() {
            children_of.entry(parent).or_default().push(idx);
        }
    }

    // Identify branch tip commits (commits with branch ref labels that are not
    // the first commit in the list). The first commit is HEAD and gets lane 0.
    let mut branch_tip_oids: HashSet<git2::Oid> = HashSet::new();
    for commit in commits.iter().skip(1) {
        let has_branch_ref = commit.refs.iter().any(|r| {
            matches!(
                r,
                RefLabel::Head | RefLabel::LocalBranch(_) | RefLabel::RemoteBranch(_)
            )
        });
        if has_branch_ref {
            branch_tip_oids.insert(commit.oid);
        }
    }

    // Each active lane: (expected OID, color index)
    let mut lanes: Vec<Option<(git2::Oid, usize)>> = Vec::new();
    let mut next_color: usize = 0;

    let mut rows = Vec::with_capacity(commits.len());

    for (idx, commit) in commits.iter().enumerate() {
        let oid = commit.oid;

        // Find which lane this commit sits in
        let (node_lane, has_incoming) = if let Some(pos) = lanes
            .iter()
            .position(|s| matches!(s, Some((o, _)) if *o == oid))
        {
            (pos, true)
        } else {
            // New branch tip — allocate a lane with a fresh color
            let color = next_color;
            next_color += 1;
            let pos = alloc_lane(&mut lanes);
            lanes[pos] = Some((oid, color));
            (pos, false)
        };

        let node_color = lanes[node_lane].map(|(_, c)| c).unwrap_or(0);

        // Build pass-through edges for all occupied lanes except node_lane
        let mut edges = Vec::new();
        for (lane, slot) in lanes.iter().enumerate() {
            if lane == node_lane {
                continue;
            }
            if let Some((_, color)) = slot {
                edges.push(GraphEdge {
                    from_lane: lane,
                    to_lane: lane,
                    color_index: *color,
                    is_merge: false,
                });
            }
        }

        // Free the commit's lane before assigning parents
        lanes[node_lane] = None;

        // Handle parents
        let parents = &commit.parent_oids;
        if !parents.is_empty() {
            // Primary parent
            let primary = parents[0];

            // Check if the primary parent is a fork point: another branch tip
            // also needs it, and that branch tip hasn't been processed yet.
            // If so, the primary parent will be claimed by the other branch
            // later, so we should still continue in our own lane.
            let primary_already_expected = lanes
                .iter()
                .any(|s| matches!(s, Some((o, _)) if *o == primary));

            let primary_lane = if primary_already_expected {
                // Already expected in another lane — diagonal merge to it
                lanes
                    .iter()
                    .position(|s| matches!(s, Some((o, _)) if *o == primary))
                    .unwrap()
            } else {
                // Check if the primary parent has multiple children that are
                // branch tips (fork point). If so, we want the *first* child
                // to keep lane 0, and subsequent children get new lanes.
                // The primary parent itself should continue on lane 0 (or
                // whichever lane is "main").
                let fork_children = children_of.get(&primary).map(|c| c.len()).unwrap_or(0);
                let parent_is_branch_base = fork_children > 1 && oid_set.contains(&primary);

                if parent_is_branch_base && node_lane != 0 {
                    // This branch diverges from the parent — keep own lane,
                    // the parent will be processed later on lane 0 (or another).
                    // Only create a new lane reservation if not already occupied.
                    lanes[node_lane] = Some((primary, node_color));
                    node_lane
                } else {
                    // Continue in the same lane with the same color
                    lanes[node_lane] = Some((primary, node_color));
                    node_lane
                }
            };

            edges.push(GraphEdge {
                from_lane: node_lane,
                to_lane: primary_lane,
                color_index: node_color,
                is_merge: false,
            });

            // Secondary parents (merge edges)
            for &parent in &parents[1..] {
                let parent_lane = if let Some(pos) = lanes
                    .iter()
                    .position(|s| matches!(s, Some((o, _)) if *o == parent))
                {
                    pos
                } else {
                    // New lane for this merge parent with a fresh color
                    let color = next_color;
                    next_color += 1;
                    let pos = alloc_lane(&mut lanes);
                    lanes[pos] = Some((parent, color));
                    pos
                };

                let parent_color = lanes[parent_lane].map(|(_, c)| c).unwrap_or(0);

                edges.push(GraphEdge {
                    from_lane: node_lane,
                    to_lane: parent_lane,
                    color_index: parent_color,
                    is_merge: true,
                });
            }
        }

        // Trim trailing empty lanes
        while lanes.last() == Some(&None) {
            lanes.pop();
        }

        let lane_count = lanes.len().max(node_lane + 1);

        rows.push(GraphRow {
            commit_index: idx,
            node_lane,
            edges,
            lane_count,
            node_color,
            has_incoming,
        });
    }

    rows
}

/// Find the first free lane or append a new one.
fn alloc_lane(lanes: &mut Vec<Option<(git2::Oid, usize)>>) -> usize {
    if let Some(pos) = lanes.iter().position(|l| l.is_none()) {
        pos
    } else {
        lanes.push(None);
        lanes.len() - 1
    }
}
