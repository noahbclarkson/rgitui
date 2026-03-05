use crate::CommitInfo;

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
pub fn compute_graph(commits: &[CommitInfo]) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    // Each active lane: (expected OID, color index)
    let mut lanes: Vec<Option<(git2::Oid, usize)>> = Vec::new();
    let mut next_color: usize = 0;

    let mut rows = Vec::with_capacity(commits.len());

    for (idx, commit) in commits.iter().enumerate() {
        let oid = commit.oid;

        // Find which lane this commit sits in
        let (node_lane, has_incoming) =
            if let Some(pos) = lanes.iter().position(|s| matches!(s, Some((o, _)) if *o == oid)) {
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
            let primary_lane =
                if let Some(pos) = lanes.iter().position(|s| matches!(s, Some((o, _)) if *o == primary)) {
                    // Already expected in another lane — diagonal to it
                    pos
                } else {
                    // Continue in the same lane with the same color
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
                let parent_lane =
                    if let Some(pos) = lanes.iter().position(|s| matches!(s, Some((o, _)) if *o == parent)) {
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
