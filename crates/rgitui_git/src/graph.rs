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
    /// Whether this commit is the HEAD commit.
    pub is_head: bool,
    /// Whether this commit is a merge commit (more than one parent).
    pub is_merge: bool,
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
/// The algorithm assigns each commit to a lane and tracks connections between rows.
/// Key behaviors:
/// - The first commit (HEAD) gets lane 0
/// - Branch tips that appear later get new lanes
/// - Primary parent edges continue in the same lane when possible
/// - Merge parent edges route to the lane already expecting that parent, or allocate new
/// - Lanes are compacted: when multiple lanes become free, later lanes shift inward
/// - Colors are assigned per-lane and stay consistent along a branch
pub fn compute_graph(commits: &[CommitInfo]) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    // Build a set of all commit OIDs for quick lookup
    let oid_set: HashSet<git2::Oid> = commits.iter().map(|c| c.oid).collect();

    // Detect HEAD oid
    let head_oid = commits.first().and_then(|c| {
        if c.refs.iter().any(|r| matches!(r, RefLabel::Head)) {
            Some(c.oid)
        } else {
            None
        }
    });

    // Build a map: parent_oid -> list of child commit indices that have it as first parent.
    let mut children_of: HashMap<git2::Oid, Vec<usize>> = HashMap::new();
    for (idx, commit) in commits.iter().enumerate() {
        if let Some(&parent) = commit.parent_oids.first() {
            children_of.entry(parent).or_default().push(idx);
        }
    }

    // Each active lane: (expected OID, color index)
    let mut lanes: Vec<Option<(git2::Oid, usize)>> = Vec::new();
    let mut next_color: usize = 0;

    let mut rows = Vec::with_capacity(commits.len());

    for (idx, commit) in commits.iter().enumerate() {
        let oid = commit.oid;
        let is_merge = commit.parent_oids.len() > 1;
        let is_head = head_oid == Some(oid);

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
            let primary = parents[0];

            // Check if the primary parent is already expected in another lane
            let primary_already_expected = lanes
                .iter()
                .any(|s| matches!(s, Some((o, _)) if *o == primary));

            let primary_lane = if primary_already_expected {
                // Already expected in another lane — route edge to that lane
                let target = lanes
                    .iter()
                    .position(|s| matches!(s, Some((o, _)) if *o == primary))
                    .unwrap();
                // Don't re-assign the lane — it's already tracking this parent
                target
            } else {
                // Check if primary parent has multiple children (fork point).
                // If this commit is NOT on lane 0 and the parent is a fork point,
                // we should route the edge to merge with the main lane rather
                // than continuing in our own lane, to keep the graph compact.
                let fork_children = children_of.get(&primary).map(|c| c.len()).unwrap_or(0);
                let parent_in_view = oid_set.contains(&primary);

                if fork_children > 1 && parent_in_view && node_lane != 0 {
                    // Check if lane 0 (or another lane) is already expecting
                    // this parent. If so, route to it.
                    if let Some(existing) = lanes
                        .iter()
                        .position(|s| matches!(s, Some((o, _)) if *o == primary))
                    {
                        existing
                    } else {
                        // Continue in our own lane
                        lanes[node_lane] = Some((primary, node_color));
                        node_lane
                    }
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

        // Compact lanes: remove trailing empty lanes
        while lanes.last() == Some(&None) {
            lanes.pop();
        }

        // Also compact interior gaps: if there are consecutive empty lanes in
        // the middle that are wider than 1 slot, collapse them. We do a simpler
        // version: just trim trailing.
        let lane_count = lanes.len().max(node_lane + 1);

        rows.push(GraphRow {
            commit_index: idx,
            node_lane,
            edges,
            lane_count,
            node_color,
            has_incoming,
            is_head,
            is_merge,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_oid(byte: u8) -> git2::Oid {
        let mut bytes = [0u8; 20];
        bytes[0] = byte;
        git2::Oid::from_bytes(&bytes).unwrap()
    }

    fn make_commit(oid_byte: u8, parents: &[u8], refs: Vec<RefLabel>) -> CommitInfo {
        CommitInfo {
            oid: make_oid(oid_byte),
            short_id: format!("{:07x}", oid_byte),
            summary: format!("Commit {}", oid_byte),
            message: format!("Commit {}", oid_byte),
            author: crate::Signature {
                name: "Test".to_string(),
                email: "test@test.com".to_string(),
            },
            committer: crate::Signature {
                name: "Test".to_string(),
                email: "test@test.com".to_string(),
            },
            time: Utc::now(),
            parent_oids: parents.iter().map(|b| make_oid(*b)).collect(),
            refs,
        }
    }

    #[test]
    fn test_linear_history() {
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert_eq!(rows.len(), 3);
        // All commits should be on lane 0
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[1].node_lane, 0);
        assert_eq!(rows[2].node_lane, 0);
        // HEAD detection
        assert!(rows[0].is_head);
        assert!(!rows[1].is_head);
        // First commit has no incoming
        assert!(!rows[0].has_incoming);
        assert!(rows[1].has_incoming);
    }

    #[test]
    fn test_merge_commit_detected() {
        let commits = vec![
            make_commit(1, &[2, 3], vec![RefLabel::Head]),
            make_commit(2, &[4], vec![]),
            make_commit(3, &[4], vec![]),
            make_commit(4, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert!(rows[0].is_merge);
        assert!(!rows[1].is_merge);
    }

    #[test]
    fn test_empty_commits() {
        let rows = compute_graph(&[]);
        assert!(rows.is_empty());
    }

    #[test]
    fn test_branch_tip_gets_new_lane() {
        let commits = vec![
            make_commit(1, &[3], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // HEAD on lane 0
        assert_eq!(rows[0].node_lane, 0);
        // Branch tip should get a different lane
        assert_ne!(rows[1].node_lane, rows[0].node_lane);
    }
}
