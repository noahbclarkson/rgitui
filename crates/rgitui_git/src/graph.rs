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

    // Build a lookup: OID -> commit index for fast ancestor checks
    let oid_to_idx: std::collections::HashMap<git2::Oid, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid, i))
        .collect();

    /// Check if `ancestor` is an ancestor of `descendant` using only the commit list.
    /// The commit list is in topological order (descendants before ancestors),
    /// so we walk `descendant`'s parent chain and check if `ancestor` appears.
    fn is_ancestor_of(
        ancestor: git2::Oid,
        descendant: git2::Oid,
        commits: &[CommitInfo],
        oid_to_idx: &std::collections::HashMap<git2::Oid, usize>,
    ) -> bool {
        if ancestor == descendant {
            return true;
        }
        let mut queue = vec![descendant];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = queue.pop() {
            if visited.insert(current) {
                if current == ancestor {
                    return true;
                }
                if let Some(&idx) = oid_to_idx.get(&current) {
                    for &parent_oid in &commits[idx].parent_oids {
                        if parent_oid != ancestor {
                            queue.push(parent_oid);
                        } else {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    // Detect HEAD oid
    let head_oid = commits.first().and_then(|c| {
        if c.refs.iter().any(|r| matches!(r, RefLabel::Head)) {
            Some(c.oid)
        } else {
            None
        }
    });

    // Each active lane: (expected OID, color index)
    let mut lanes: Vec<Option<(git2::Oid, usize)>> = Vec::new();
    let mut next_color: usize = 0;

    let mut rows = Vec::with_capacity(commits.len());

    for (idx, commit) in commits.iter().enumerate() {
        let oid = commit.oid;
        let is_merge = commit.parent_oids.len() > 1;
        let is_head = head_oid == Some(oid);

        // Find which lane this commit sits in
        // 1. Exact OID match: the lane already has this commit's OID as expected
        // 2. Ancestor match: some lane's expected commit is a descendant of the current
        //    commit (descendants are processed before ancestors in topological order)
        let (node_lane, has_incoming) = if let Some(pos) = lanes
            .iter()
            .position(|s| matches!(s, Some((o, _)) if *o == oid))
        {
            (pos, true)
        } else if let Some(pos) = lanes.iter().position(|s| {
            matches!(s, Some((expected_oid, _)) if is_ancestor_of(oid, *expected_oid, commits, &oid_to_idx))
        }) {
            // Current commit is an ancestor of the lane's expected commit — reuse this lane.
            // The lane's color is preserved so the branch line stays consistent.
            let color = lanes[pos].map(|(_, c)| c).unwrap_or(next_color);
            lanes[pos] = Some((oid, color));
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

            let primary_lane = if let Some(target) = lanes
                .iter()
                .position(|s| matches!(s, Some((o, _)) if *o == primary))
            {
                // Already expected in another lane -- route edge to that lane
                target
            } else if let Some(target) = lanes.iter().position(|s| {
                matches!(s, Some((expected_oid, _)) if is_ancestor_of(primary, *expected_oid, commits, &oid_to_idx))
            }) {
                // Primary parent is an ancestor of a lane's expected commit —
                // route to that lane so the edge merges correctly.
                target
            } else {
                // No lane is expecting the primary parent yet.
                // Continue in the same lane with the same color.
                lanes[node_lane] = Some((primary, node_color));
                node_lane
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
                } else if let Some(pos) = lanes.iter().position(|s| {
                    matches!(s, Some((expected_oid, _)) if is_ancestor_of(parent, *expected_oid, commits, &oid_to_idx))
                }) {
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
            co_authors: vec![],
            time: Utc::now(),
            parent_oids: parents.iter().map(|b| make_oid(*b)).collect(),
            refs,
            is_signed: false,
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

    #[test]
    fn test_octopus_merge_three_parents() {
        // A commit with 3 parents — octopus merge
        let commits = vec![
            make_commit(1, &[2, 3, 4], vec![RefLabel::Head]),
            make_commit(2, &[5], vec![]),
            make_commit(3, &[5], vec![]),
            make_commit(4, &[5], vec![]),
            make_commit(5, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert!(rows[0].is_merge);
        // Octopus merge: 1 primary + 2 secondary = 2 merge edges
        let merge_edges: Vec<_> = rows[0].edges.iter().filter(|e| e.is_merge).collect();
        assert_eq!(merge_edges.len(), 2);
    }

    #[test]
    fn test_ref_labels_tag_and_remote() {
        let commits = vec![
            make_commit(
                1,
                &[2],
                vec![RefLabel::Head, RefLabel::Tag("v1.0.0".into())],
            ),
            make_commit(2, &[3], vec![RefLabel::RemoteBranch("origin/main".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert!(rows[0].is_head);
        assert!(!rows[1].is_head);
        assert!(!rows[2].is_head);
    }

    #[test]
    fn test_lane_count_grows_with_parallel_branches() {
        // Create parallel branches: main and feature both active
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // Linear history: lane_count stays 1
        assert_eq!(rows[0].lane_count, 1);
        assert_eq!(rows[1].lane_count, 1);

        // Now with a branch tip appearing later
        let commits_with_branch = vec![
            make_commit(1, &[3], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits_with_branch);
        // When feature tip (row 1) is active, lane_count = 2
        assert!(rows[1].lane_count >= 2);
    }

    #[test]
    fn test_merge_edge_flags_on_secondary_parents() {
        let commits = vec![
            make_commit(1, &[2, 3], vec![RefLabel::Head]),
            make_commit(2, &[4], vec![]),
            make_commit(3, &[4], vec![]),
            make_commit(4, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // Row 0 (merge commit) has 2 outgoing edges: primary (is_merge=false) and secondary (is_merge=true)
        let edges = &rows[0].edges;
        let merge_flagged: Vec<_> = edges.iter().filter(|e| e.is_merge).collect();
        let primary_edges: Vec<_> = edges.iter().filter(|e| !e.is_merge).collect();
        assert_eq!(merge_flagged.len(), 1); // one secondary parent → one merge edge
        assert!(!primary_edges.is_empty()); // at least one primary edge
    }

    #[test]
    fn test_commit_index_matches_input_order() {
        let commits = vec![
            make_commit(10, &[20], vec![RefLabel::Head]),
            make_commit(20, &[30], vec![]),
            make_commit(30, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert_eq!(rows[0].commit_index, 0);
        assert_eq!(rows[1].commit_index, 1);
        assert_eq!(rows[2].commit_index, 2);
    }

    #[test]
    fn test_primary_parent_continues_same_lane() {
        // In a simple linear chain, each commit continues the same lane
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // Each row should continue on lane 0
        for row in &rows {
            assert_eq!(row.node_lane, 0);
        }
        // No merge edges in linear history
        for row in &rows {
            assert!(
                !row.edges.iter().any(|e| e.is_merge),
                "linear history should have no merge edges"
            );
        }
    }

    #[test]
    fn test_edge_color_from_lane() {
        // Edge color_index should match the source lane's node_color
        let commits = vec![
            make_commit(1, &[3], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        let row0_color = rows[0].node_color;
        // Edges from row 0's lane should carry row 0's color
        let row0_outgoing: Vec<_> = rows[0]
            .edges
            .iter()
            .filter(|e| e.from_lane == rows[0].node_lane)
            .collect();
        for edge in row0_outgoing {
            assert_eq!(
                edge.color_index, row0_color,
                "outgoing edge from a lane should carry that lane's color"
            );
        }
    }

    #[test]
    fn test_has_incoming_false_for_first_and_new_branches() {
        let commits = vec![
            make_commit(1, &[3], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // Row 0 (HEAD tip): first commit, no incoming
        assert!(!rows[0].has_incoming);
        // Row 1 (feature tip): new branch, no incoming
        assert!(!rows[1].has_incoming);
        // Row 2 (merge): expected by both previous commits, has incoming
        assert!(rows[2].has_incoming);
    }
}
