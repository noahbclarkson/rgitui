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

/// Find the main branch tip OID by scanning commit refs for "main" or "master".
fn find_main_branch_tip(commits: &[CommitInfo]) -> Option<git2::Oid> {
    // Priority: LocalBranch("main") > LocalBranch("master") > RemoteBranch containing "main" > RemoteBranch containing "master"
    let mut main_local = None;
    let mut master_local = None;
    let mut main_remote = None;
    let mut master_remote = None;

    for commit in commits {
        for r in &commit.refs {
            match r {
                RefLabel::LocalBranch(name) if name == "main" => {
                    main_local = Some(commit.oid);
                }
                RefLabel::LocalBranch(name) if name == "master" => {
                    master_local = Some(commit.oid);
                }
                RefLabel::RemoteBranch(name) if name.ends_with("/main") => {
                    if main_remote.is_none() {
                        main_remote = Some(commit.oid);
                    }
                }
                RefLabel::RemoteBranch(name) if name.ends_with("/master") => {
                    if master_remote.is_none() {
                        master_remote = Some(commit.oid);
                    }
                }
                _ => {}
            }
        }
    }

    main_local
        .or(master_local)
        .or(main_remote)
        .or(master_remote)
}

/// Compute the set of OIDs on the main branch's first-parent chain.
/// At merge commits, picks the parent that is most likely the main-line ancestor
/// by preferring parents that don't have a feature branch ref pointing at them.
fn compute_main_chain(
    main_tip: git2::Oid,
    commits: &[CommitInfo],
    oid_to_idx: &std::collections::HashMap<git2::Oid, usize>,
) -> std::collections::HashSet<git2::Oid> {
    let mut chain = std::collections::HashSet::new();
    let mut current = Some(main_tip);
    while let Some(oid) = current {
        chain.insert(oid);
        current = oid_to_idx.get(&oid).and_then(|&idx| {
            let commit = &commits[idx];
            if commit.parent_oids.len() <= 1 {
                commit.parent_oids.first().copied()
            } else {
                pick_main_parent(&commit.parent_oids, commits, oid_to_idx)
            }
        });
    }
    chain
}

/// At a merge commit, pick which parent is most likely the main-line ancestor.
/// Prefers parents that don't have a non-main branch ref (feature branches).
fn pick_main_parent(
    parents: &[git2::Oid],
    commits: &[CommitInfo],
    oid_to_idx: &std::collections::HashMap<git2::Oid, usize>,
) -> Option<git2::Oid> {
    // Check each parent for non-main branch refs
    let mut non_feature_parent = None;
    for &parent_oid in parents {
        let has_feature_ref = oid_to_idx.get(&parent_oid).is_some_and(|idx| {
            commits[*idx].refs.iter().any(|r| {
                matches!(r,
                    RefLabel::LocalBranch(name) if name != "main" && name != "master"
                )
            })
        });
        if !has_feature_ref && non_feature_parent.is_none() {
            non_feature_parent = Some(parent_oid);
        }
    }
    non_feature_parent.or_else(|| parents.first().copied())
}

/// Compute the graph layout for a list of commits.
///
/// The algorithm assigns each commit to a lane and tracks connections between rows.
/// Key behaviors:
/// - Main branch (main/master) commits always stay on lane 0
/// - Feature branches that are ahead of main get their own lane
/// - After merges, main's history continues on lane 0 regardless of git's parent ordering
/// - Branch tips that appear later get new lanes
/// - Lanes are compacted: when multiple lanes become free, later lanes shift inward
/// - Colors are assigned per-lane and stay consistent along a branch
pub fn compute_graph(commits: &[CommitInfo]) -> Vec<GraphRow> {
    let graph_timer = std::time::Instant::now();
    log::debug!("compute_graph: {} commits", commits.len());
    if commits.is_empty() {
        return Vec::new();
    }

    let oid_to_idx: std::collections::HashMap<git2::Oid, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid, i))
        .collect();

    // Detect HEAD oid — scan all commits, not just the first, because
    // feature branch tips may appear before HEAD in topological order.
    let head_oid = commits
        .iter()
        .find(|c| c.refs.iter().any(|r| matches!(r, RefLabel::Head)))
        .map(|c| c.oid);

    // Identify the main branch and compute its first-parent chain.
    // This lets us keep main on lane 0 and push feature branches to other lanes.
    let main_tip = find_main_branch_tip(commits);
    let main_chain: std::collections::HashSet<git2::Oid> = match main_tip {
        Some(tip) => compute_main_chain(tip, commits, &oid_to_idx),
        None => {
            // No main/master found — fall back to HEAD's first-parent chain
            if let Some(head) = head_oid {
                compute_main_chain(head, commits, &oid_to_idx)
            } else {
                std::collections::HashSet::new()
            }
        }
    };

    let head_on_main = head_oid.is_none_or(|h| main_chain.contains(&h));

    // Each active lane: (expected OID, color index)
    let mut lanes: Vec<Option<(git2::Oid, usize)>> = Vec::new();
    let mut next_color: usize = 0;

    // If HEAD is on a feature branch (not on main), pre-reserve lane 0 for main.
    // The main tip will eventually arrive in the commit list and land on lane 0.
    if !head_on_main {
        if let Some(tip) = main_tip {
            let color = next_color;
            next_color += 1;
            lanes.push(Some((tip, color))); // lane 0 reserved for main
        }
    }

    let mut rows = Vec::with_capacity(commits.len());

    for (idx, commit) in commits.iter().enumerate() {
        let oid = commit.oid;
        let is_merge = commit.parent_oids.len() > 1;
        let is_head = head_oid == Some(oid);
        let on_main = main_chain.contains(&oid);

        // Find which lane this commit sits in.
        // Main-chain commits prefer lane 0; non-main commits skip lane 0.
        let (node_lane, has_incoming) = if on_main {
            // Main-chain commit: look for exact match at lane 0 first, then anywhere
            if matches!(lanes.first(), Some(Some((o, _))) if *o == oid) {
                (0, true)
            } else if matches!(lanes.first(), Some(Some((expected, _))) if is_ancestor_of(oid, *expected, commits, &oid_to_idx))
            {
                let color = lanes[0].map(|(_, c)| c).unwrap_or(next_color);
                lanes[0] = Some((oid, color));
                (0, true)
            } else if matches!(lanes.first(), Some(None)) || lanes.is_empty() {
                // Lane 0 is free — claim it
                let was_empty = lanes.is_empty();
                let color = if was_empty {
                    let c = next_color;
                    next_color += 1;
                    lanes.push(None);
                    c
                } else {
                    // Reuse color 0 (main's color) if this is the first main commit
                    0
                };
                lanes[0] = Some((oid, color));
                // has_incoming only if lane 0 was previously occupied (something
                // was drawing on this lane above us). A free/empty lane means
                // nothing was drawn, so no incoming line even if idx > 0.
                (0, false)
            } else if matches!(lanes.first(), Some(Some((expected, _))) if main_chain.contains(expected)) {
                // Lane 0 is occupied by another main-chain commit. This
                // happens when topo-sort reorders main commits around a
                // branch fork point. Force onto lane 0 anyway.
                let color = lanes[0].map(|(_, c)| c).unwrap_or(next_color);
                lanes[0] = Some((oid, color));
                (0, true)
            } else {
                // Lane 0 is occupied by a non-main commit — search other lanes
                find_lane(oid, &mut lanes, &mut next_color, commits, &oid_to_idx, None)
            }
        } else {
            // Non-main-chain commit: skip lane 0
            find_lane(
                oid,
                &mut lanes,
                &mut next_color,
                commits,
                &oid_to_idx,
                Some(0),
            )
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

        // Free the commit's lane before assigning parents.
        lanes[node_lane] = None;

        // Clear any OTHER lanes that were also waiting for this same OID.
        // This happens when a commit's OID was routed to multiple lanes
        // (e.g., a feature branch tip allocated a lane for its parent, and
        // then the main-chain routing also placed the same parent on lane 0).
        // Without this cleanup, the stale lane persists as a zombie
        // pass-through line forever.
        // When clearing, convert the pass-through edge into a merge edge
        // (from the zombie lane curving into node_lane) so the branch
        // visually merges back.
        for (lane_idx, lane) in lanes.iter_mut().enumerate() {
            if matches!(lane, Some((o, _)) if *o == oid) {
                let color = lane.map(|(_, c)| c).unwrap_or(0);
                *lane = None;
                // Replace the straight pass-through with a converging edge
                if let Some(edge) = edges.iter_mut().find(|e| e.from_lane == lane_idx && e.to_lane == lane_idx) {
                    edge.to_lane = node_lane;
                }
                // If no pass-through existed (shouldn't happen, but be safe),
                // add a converging edge
                if !edges.iter().any(|e| e.from_lane == lane_idx && e.to_lane == node_lane) {
                    edges.push(GraphEdge {
                        from_lane: lane_idx,
                        to_lane: node_lane,
                        color_index: color,
                        is_merge: false,
                    });
                }
            }
        }

        // Handle parents — for main-chain merge commits, identify which parent
        // is on the main chain and treat that as the "primary" parent for lane routing.
        let parents = &commit.parent_oids;
        if !parents.is_empty() {
            // Determine effective primary parent: on main-chain merges, the main-chain
            // parent gets lane priority regardless of git's parent ordering.
            let (primary, secondaries) = if on_main && parents.len() > 1 {
                if let Some(main_parent_pos) = parents.iter().position(|p| main_chain.contains(p)) {
                    let primary = parents[main_parent_pos];
                    let secondaries: Vec<git2::Oid> = parents
                        .iter()
                        .enumerate()
                        .filter(|&(i, _)| i != main_parent_pos)
                        .map(|(_, &p)| p)
                        .collect();
                    (primary, secondaries)
                } else {
                    (parents[0], parents[1..].to_vec())
                }
            } else {
                (parents[0], parents[1..].to_vec())
            };

            // Route primary parent.
            // Only route to lane 0 when BOTH the commit and its parent are
            // on the main chain. When a feature branch's parent is on main,
            // use normal routing so it doesn't steal lane 0.
            let primary_on_main = on_main && main_chain.contains(&primary);
            let primary_lane = if primary_on_main {
                // Main-chain parent should go to lane 0
                if matches!(lanes.first(), Some(Some((o, _))) if *o == primary) {
                    0
                } else if matches!(lanes.first(), Some(None)) || (lanes.is_empty()) {
                    if lanes.is_empty() {
                        lanes.push(None);
                    }
                    lanes[0] = Some((primary, node_color));
                    0
                } else if node_lane == 0 {
                    lanes[0] = Some((primary, node_color));
                    0
                } else {
                    // Lane 0 occupied by something else, fall back
                    route_parent(
                        primary,
                        node_lane,
                        node_color,
                        &mut lanes,
                        &mut next_color,
                        commits,
                        &oid_to_idx,
                    )
                }
            } else {
                route_parent(
                    primary,
                    node_lane,
                    node_color,
                    &mut lanes,
                    &mut next_color,
                    commits,
                    &oid_to_idx,
                )
            };

            edges.push(GraphEdge {
                from_lane: node_lane,
                to_lane: primary_lane,
                color_index: node_color,
                is_merge: false,
            });

            // Secondary parents (merge edges)
            for &parent in &secondaries {
                let parent_lane = route_parent(
                    parent,
                    node_lane,
                    node_color,
                    &mut lanes,
                    &mut next_color,
                    commits,
                    &oid_to_idx,
                );

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

    log::debug!(
        "compute_graph complete in {:?}: {} rows",
        graph_timer.elapsed(),
        rows.len()
    );
    rows
}

/// Find a lane for a commit: exact OID match, ancestor match, or allocate new.
/// If `skip_lane` is Some(n), that lane is excluded from the search (used to keep
/// non-main commits off lane 0).
fn find_lane(
    oid: git2::Oid,
    lanes: &mut Vec<Option<(git2::Oid, usize)>>,
    next_color: &mut usize,
    commits: &[CommitInfo],
    oid_to_idx: &std::collections::HashMap<git2::Oid, usize>,
    skip_lane: Option<usize>,
) -> (usize, bool) {
    // 1. Exact OID match
    if let Some(pos) = lanes
        .iter()
        .enumerate()
        .position(|(i, s)| Some(i) != skip_lane && matches!(s, Some((o, _)) if *o == oid))
    {
        return (pos, true);
    }

    // 2. Ancestor match
    if let Some(pos) = lanes.iter().enumerate().position(|(i, s)| {
        Some(i) != skip_lane
            && matches!(s, Some((expected_oid, _)) if is_ancestor_of(oid, *expected_oid, commits, oid_to_idx))
    }) {
        let color = lanes[pos].map(|(_, c)| c).unwrap_or(*next_color);
        lanes[pos] = Some((oid, color));
        return (pos, true);
    }

    // 3. New branch tip — allocate a lane, skipping the reserved lane
    let color = *next_color;
    *next_color += 1;
    let pos = alloc_lane(lanes, skip_lane);
    lanes[pos] = Some((oid, color));
    (pos, false)
}

/// Route a parent to an existing lane or allocate a new one.
fn route_parent(
    parent: git2::Oid,
    node_lane: usize,
    node_color: usize,
    lanes: &mut Vec<Option<(git2::Oid, usize)>>,
    next_color: &mut usize,
    commits: &[CommitInfo],
    oid_to_idx: &std::collections::HashMap<git2::Oid, usize>,
) -> usize {
    if let Some(target) = lanes
        .iter()
        .position(|s| matches!(s, Some((o, _)) if *o == parent))
    {
        target
    } else if let Some(target) = lanes.iter().position(|s| {
        matches!(s, Some((expected_oid, _)) if is_ancestor_of(parent, *expected_oid, commits, oid_to_idx))
    }) {
        // Update the lane to track the actual parent OID (not the old
        // descendant). This prevents the zombie cleanup from clearing
        // the lane when the old OID's commit arrives.
        let color = lanes[target].map(|(_, c)| c).unwrap_or(node_color);
        lanes[target] = Some((parent, color));
        target
    } else {
        // Continue in the same lane with the same color if it's free
        if lanes.get(node_lane) == Some(&None) {
            lanes[node_lane] = Some((parent, node_color));
            node_lane
        } else {
            let color = *next_color;
            *next_color += 1;
            let pos = alloc_lane(lanes, None);
            lanes[pos] = Some((parent, color));
            pos
        }
    }
}

/// Find the first free lane or append a new one, optionally skipping a reserved lane.
fn alloc_lane(lanes: &mut Vec<Option<(git2::Oid, usize)>>, skip_lane: Option<usize>) -> usize {
    if let Some(pos) = lanes
        .iter()
        .enumerate()
        .position(|(i, l)| l.is_none() && Some(i) != skip_lane)
    {
        pos
    } else {
        // Append a new lane. If the new position happens to be the skip_lane
        // (e.g., lanes was empty and skip_lane=Some(0)), reserve that slot
        // and append another one.
        lanes.push(None);
        let pos = lanes.len() - 1;
        if Some(pos) == skip_lane {
            lanes.push(None);
            lanes.len() - 1
        } else {
            pos
        }
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

    // ── is_ancestor_of tests ────────────────────────────────────────
    // Note: is_ancestor_of is called in compute_graph as:
    //   is_ancestor_of(candidate_oid, expected_parent_oid, ...)
    // meaning: "is candidate_oid an ancestor of expected_parent_oid?"
    // The commits list is in topological order (descendants before ancestors).

    fn make_oid_to_idx(commits: &[CommitInfo]) -> std::collections::HashMap<git2::Oid, usize> {
        commits
            .iter()
            .enumerate()
            .map(|(i, c)| (c.oid, i))
            .collect()
    }

    #[test]
    fn is_ancestor_of_direct_parent() {
        // Commits in topological order: HEAD=oid1, parent=oid2
        // is_ancestor_of(oid2, oid1) = true (oid2 is ancestor of oid1)
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[], vec![]),
        ];
        let oid_to_idx = make_oid_to_idx(&commits);
        // oid2 IS ancestor of oid1 (direct parent)
        assert!(is_ancestor_of(
            make_oid(2),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // oid1 is NOT ancestor of oid2 (reverse direction)
        assert!(!is_ancestor_of(
            make_oid(1),
            make_oid(2),
            &commits,
            &oid_to_idx
        ));
    }

    #[test]
    fn is_ancestor_of_grandparent() {
        // Chain: oid1 → oid2 → oid3 → none  (topological: oid1, oid2, oid3)
        // oid3 is ancestor of oid1 (grandparent), oid2 is direct parent
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[], vec![]),
        ];
        let oid_to_idx = make_oid_to_idx(&commits);
        // oid3 is ancestor of oid1 (grandparent)
        assert!(is_ancestor_of(
            make_oid(3),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // oid2 is ancestor of oid1 (direct parent)
        assert!(is_ancestor_of(
            make_oid(2),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // oid1 is NOT ancestor of oid3 (reverse direction)
        assert!(!is_ancestor_of(
            make_oid(1),
            make_oid(3),
            &commits,
            &oid_to_idx
        ));
    }

    #[test]
    fn is_ancestor_of_reflexive() {
        // A commit is its own ancestor (reflexive property)
        let commits = vec![make_commit(1, &[], vec![RefLabel::Head])];
        let oid_to_idx = make_oid_to_idx(&commits);
        assert!(is_ancestor_of(
            make_oid(1),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
    }

    #[test]
    fn is_ancestor_of_unrelated_branches() {
        // Two separate chains with no common ancestor
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[], vec![]),
            make_commit(10, &[20], vec![]),
            make_commit(20, &[], vec![]),
        ];
        let oid_to_idx = make_oid_to_idx(&commits);
        // No cross-branch ancestry
        assert!(!is_ancestor_of(
            make_oid(1),
            make_oid(10),
            &commits,
            &oid_to_idx
        ));
        assert!(!is_ancestor_of(
            make_oid(10),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // Within each chain: parent is ancestor of child
        assert!(is_ancestor_of(
            make_oid(2),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        assert!(is_ancestor_of(
            make_oid(20),
            make_oid(10),
            &commits,
            &oid_to_idx
        ));
    }

    #[test]
    fn is_ancestor_of_merge_commit() {
        // Merge: oid1 ← [oid2, oid20]; oid2 ← [oid3]; oid20 ← [oid3]
        // Topological: [oid1, oid2, oid20, oid3]
        // oid3 is common ancestor of oid1 (via oid2 AND oid20)
        // oid2 is direct parent of oid1 (merge parent)
        // oid20 is direct parent of oid1 (merge parent)
        let commits = vec![
            make_commit(1, &[2, 20], vec![RefLabel::Head]), // merge commit
            make_commit(2, &[3], vec![]),                   // branch 1
            make_commit(20, &[3], vec![]),                  // branch 2
            make_commit(3, &[], vec![]),                    // common ancestor
        ];
        let oid_to_idx = make_oid_to_idx(&commits);
        // oid3 is ancestor of oid1 (via either branch)
        assert!(is_ancestor_of(
            make_oid(3),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // oid2 is ancestor of oid1 (direct merge parent)
        assert!(is_ancestor_of(
            make_oid(2),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // oid20 is ancestor of oid1 (direct merge parent)
        assert!(is_ancestor_of(
            make_oid(20),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // Reverse: oid1 is NOT ancestor of oid3
        assert!(!is_ancestor_of(
            make_oid(1),
            make_oid(3),
            &commits,
            &oid_to_idx
        ));
    }

    #[test]
    fn is_ancestor_of_missing_oid_returns_false() {
        // OID not in the commit list → not found in oid_to_idx → returns false (no panic)
        let commits = vec![make_commit(1, &[], vec![RefLabel::Head])];
        let oid_to_idx = make_oid_to_idx(&commits);
        // Nonexistent OID is not ancestor of anything
        assert!(!is_ancestor_of(
            make_oid(99),
            make_oid(1),
            &commits,
            &oid_to_idx
        ));
        // Nothing is ancestor of a nonexistent OID
        assert!(!is_ancestor_of(
            make_oid(1),
            make_oid(99),
            &commits,
            &oid_to_idx
        ));
    }

    // ── Main-branch-awareness tests ────────────────────────────────────

    #[test]
    fn test_feature_ahead_of_main_gets_own_lane() {
        // Feature branch is 2 commits ahead of main, no divergence.
        // HEAD is on the feature branch, main is at commit 3.
        //
        // Expected graph:
        //   Lane 0    Lane 1
        //     │        ● C1 (feature, HEAD)
        //     │        │
        //     │        ● C2
        //     │       ╱
        //     ● C3 (main)
        //     │
        //     ● C4
        let commits = vec![
            make_commit(
                1,
                &[2],
                vec![RefLabel::Head, RefLabel::LocalBranch("feature".into())],
            ),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[4], vec![RefLabel::LocalBranch("main".into())]),
            make_commit(4, &[], vec![]),
        ];
        let rows = compute_graph(&commits);

        // Feature commits (1, 2) should NOT be on lane 0
        assert_ne!(
            rows[0].node_lane, 0,
            "feature HEAD should not be on main's lane"
        );
        assert_ne!(
            rows[1].node_lane, 0,
            "feature commit should not be on main's lane"
        );

        // Main commits (3, 4) should be on lane 0
        assert_eq!(rows[2].node_lane, 0, "main tip should be on lane 0");
        assert_eq!(rows[3].node_lane, 0, "main ancestor should be on lane 0");
    }

    #[test]
    fn test_main_stays_lane_zero_after_merge() {
        // Merge commit on main where git stored feature as parents[0].
        // Main's history should still continue on lane 0.
        //
        // Git history: M (main, HEAD) → parents: [F1, M1]
        //   F1 (feature) → parent: M1
        //   M1 → parent: M0
        //
        // Note: parents[0] = F1 (feature), parents[1] = M1 (main ancestor)
        // This simulates `git merge main` while on feature, then fast-forward.
        let commits = vec![
            make_commit(
                1,
                &[2, 3],
                vec![RefLabel::Head, RefLabel::LocalBranch("main".into())],
            ),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature".into())]),
            make_commit(3, &[4], vec![]),
            make_commit(4, &[], vec![]),
        ];
        let rows = compute_graph(&commits);

        // Merge commit on lane 0 (it's on main)
        assert_eq!(rows[0].node_lane, 0, "merge commit should be on lane 0");
        // Main's ancestor (commit 3) should be on lane 0
        assert_eq!(rows[2].node_lane, 0, "main ancestor should stay on lane 0");
        assert_eq!(
            rows[3].node_lane, 0,
            "deep main ancestor should stay on lane 0"
        );
        // Feature commit (2) should be on a different lane
        assert_ne!(rows[1].node_lane, 0, "feature should not be on lane 0");
    }

    #[test]
    fn test_main_stays_lane_zero_multiple_merges() {
        // Two successive merges on main — main should always stay on lane 0.
        //
        // M2 (main, HEAD) → parents: [M1, F2]
        // F2 (feature-2) → parent: M1
        // M1 → parents: [M0, F1]
        // F1 (feature-1) → parent: M0
        // M0
        let commits = vec![
            make_commit(
                1,
                &[3, 2],
                vec![RefLabel::Head, RefLabel::LocalBranch("main".into())],
            ),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("feature-2".into())]),
            make_commit(3, &[5, 4], vec![]),
            make_commit(4, &[5], vec![RefLabel::LocalBranch("feature-1".into())]),
            make_commit(5, &[], vec![]),
        ];
        let rows = compute_graph(&commits);

        // All main-chain commits on lane 0
        assert_eq!(rows[0].node_lane, 0, "M2 should be on lane 0");
        assert_eq!(rows[2].node_lane, 0, "M1 should be on lane 0");
        assert_eq!(rows[4].node_lane, 0, "M0 should be on lane 0");
        // Feature branches on other lanes
        assert_ne!(rows[1].node_lane, 0, "feature-2 should not be on lane 0");
        assert_ne!(rows[3].node_lane, 0, "feature-1 should not be on lane 0");
    }

    #[test]
    fn test_no_main_branch_falls_back_to_head() {
        // No "main" or "master" ref — HEAD's first-parent chain is treated as main.
        let commits = vec![
            make_commit(1, &[2], vec![RefLabel::Head]),
            make_commit(2, &[3], vec![]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // All on lane 0 (HEAD's first-parent chain = main fallback)
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[1].node_lane, 0);
        assert_eq!(rows[2].node_lane, 0);
    }

    #[test]
    fn test_master_branch_detected() {
        // "master" branch is detected as main when "main" doesn't exist
        let commits = vec![
            make_commit(
                1,
                &[2],
                vec![RefLabel::Head, RefLabel::LocalBranch("feature".into())],
            ),
            make_commit(2, &[3], vec![RefLabel::LocalBranch("master".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        // HEAD is on feature, not master — feature should not be on lane 0
        assert_ne!(rows[0].node_lane, 0, "feature HEAD should not be on lane 0");
        // master should be on lane 0
        assert_eq!(rows[1].node_lane, 0, "master should be on lane 0");
    }

    #[test]
    fn test_remote_main_detected() {
        // origin/main is detected as main when no local main exists
        let commits = vec![
            make_commit(
                1,
                &[2],
                vec![RefLabel::Head, RefLabel::LocalBranch("feature".into())],
            ),
            make_commit(2, &[3], vec![RefLabel::RemoteBranch("origin/main".into())]),
            make_commit(3, &[], vec![]),
        ];
        let rows = compute_graph(&commits);
        assert_ne!(rows[0].node_lane, 0, "feature HEAD should not be on lane 0");
        assert_eq!(rows[1].node_lane, 0, "origin/main should be on lane 0");
    }

    /// Uses LIVE data from the actual rgitui repository via gather_refresh_data.
    /// This test sees exactly the same commits, refs, and ordering as the app.
    #[test]
    fn test_live_repo_graph() {
        use std::path::Path;

        let repo_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .parent().unwrap();

        let data = match crate::gather_refresh_data(repo_path, 1000) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Skipping live repo test: {}", e);
                return;
            }
        };

        let commits = &data.recent_commits;
        let rows = compute_graph(commits);

        // Find key commits by OID prefix
        let find = |prefix: &str| -> Option<usize> {
            commits.iter().position(|c| format!("{}", c.oid).starts_with(prefix))
        };

        let some_795 = find("795f1ba");
        let some_030 = find("0308d39");
        let some_f50 = find("f50abd6");
        let some_1bb = find("1bb477f");
        let some_e5d = find("e5d6285");
        let some_3ce = find("3ce915d");

        if [some_795, some_030, some_f50, some_1bb, some_e5d, some_3ce]
            .iter().any(|x| x.is_none())
        {
            eprintln!("Skipping: worktree demo commits not found");
            return;
        }

        let i795 = some_795.unwrap();
        let i030 = some_030.unwrap();
        let if50 = some_f50.unwrap();
        let i1bb = some_1bb.unwrap();
        let ie5d = some_e5d.unwrap();
        let i3ce = some_3ce.unwrap();

        // Dump first 15 rows
        eprintln!("\n=== LIVE COMMIT ORDER (first 15) ===");
        for (i, c) in commits.iter().enumerate().take(15) {
            let short = &format!("{}", c.oid)[..7];
            let refs_s: Vec<_> = c.refs.iter().map(|r| format!("{:?}", r)).collect();
            let pars: Vec<_> = c.parent_oids.iter().map(|p| format!("{}", p)[..7].to_string()).collect();
            eprintln!("  idx {:2}: {} parents=[{}] refs=[{}]", i, short, pars.join(","), refs_s.join(","));
        }

        eprintln!("\n=== GRAPH OUTPUT ===");
        for (i, r) in rows.iter().enumerate().take(15) {
            let short = &format!("{}", commits[i].oid)[..7];
            let es: Vec<_> = r.edges.iter()
                .map(|e| format!("{}→{}(c{}{})", e.from_lane, e.to_lane, e.color_index,
                    if e.is_merge { ",m" } else { "" }))
                .collect();
            eprintln!("  row {:2} [{}]: lane={} color={} inc={} head={} lanes={} edges=[{}]",
                i, short, r.node_lane, r.node_color, r.has_incoming, r.is_head,
                r.lane_count, es.join(", "));
        }

        // ═══════════ ASSERTIONS ═══════════

        // Main-chain commits on lane 0
        assert_eq!(rows[if50].node_lane, 0, "f50abd6 (HEAD/main) must be lane 0, was {}", rows[if50].node_lane);
        assert_eq!(rows[i1bb].node_lane, 0, "1bb477f must be lane 0, was {}", rows[i1bb].node_lane);
        assert_eq!(rows[ie5d].node_lane, 0, "e5d6285 must be lane 0, was {}", rows[ie5d].node_lane);
        assert_eq!(rows[i3ce].node_lane, 0, "3ce915d must be lane 0, was {}", rows[i3ce].node_lane);

        // Feature branches NOT on lane 0
        assert_ne!(rows[i795].node_lane, 0, "795f1ba must NOT be lane 0");
        assert_ne!(rows[i030].node_lane, 0, "0308d39 must NOT be lane 0");

        // HEAD
        assert!(rows[if50].is_head, "f50abd6 should be HEAD");

        // After 3ce915d, lane count should be 1 (only main)
        if i3ce + 1 < rows.len() {
            assert_eq!(rows[i3ce + 1].lane_count, 1,
                "after 3ce915d, only lane 0 should remain, was {}", rows[i3ce + 1].lane_count);
        }
    }

    /// Build a CommitInfo from a real hex OID string, parent hex strings, and refs.
    fn make_real_commit(hex: &str, parent_hexes: &[&str], refs: Vec<RefLabel>) -> CommitInfo {
        CommitInfo {
            oid: git2::Oid::from_str(hex).unwrap(),
            short_id: hex[..7].to_string(),
            summary: format!("Commit {}", &hex[..7]),
            message: format!("Commit {}", &hex[..7]),
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
            parent_oids: parent_hexes
                .iter()
                .map(|h| git2::Oid::from_str(h).unwrap())
                .collect(),
            refs,
            is_signed: false,
        }
    }

    /// Loads the REAL commits from the rgitui repo using the same code path
    /// as the app (`gather_refresh_data`), then runs `compute_graph` and
    /// verifies the output matches expectations.
    ///
    /// `git log --all --topo-order --format="%H %P %D"` gives:
    ///
    /// ```text
    /// 795f1ba e5d6285  codex/demo-wt-commit
    /// 0308d39 3ce915d  codex/demo-wt-pending
    /// f50abd6 52c7065  HEAD -> main, origin/main
    /// 52c7065 9b17780
    /// 9b17780 1bb477f
    /// 1bb477f e5d6285
    /// e5d6285 9328231
    /// 9328231 ba6e821
    /// ba6e821 3ce915d
    /// 3ce915d bc215a6
    /// bc215a6 (root for this test)
    /// ```
    ///
    /// Expected graph:
    ///
    /// ```text
    ///   Lane 0             Lane 1
    ///
    ///                      ● 795f1ba  (codex/demo-wt-commit, parent=e5d6285)
    ///                      │
    ///                      ├─── ● 0308d39  (codex/demo-wt-pending, parent=3ce915d)
    ///                      │        edge from lane 2 merges to lane 1 (ancestor match)
    ///   ● f50abd6 HEAD     │
    ///   │  main            │
    ///   ● 52c7065          │
    ///   ● 9b17780          │
    ///   ● 1bb477f          │
    ///   ├──────────────────┘   (1bb477f parent=e5d6285, routed to lane 0; lane 1 still holds 3ce915d)
    ///   ● e5d6285              (zombie cleanup: e5d6285 was on lane 0 only, lane 1 has 3ce915d)
    ///   │                  │   (lane 1 still active with 3ce915d)
    ///   ● 9328231          │
    ///   ● ba6e821          │
    ///   ├──────────────────┘   (ba6e821 parent=3ce915d, lane 1 consumed)
    ///   ● 3ce915d
    ///   ● bc215a6              (single lane)
    /// ```
    #[test]
    fn test_real_repo_worktree_graph() {
        // Real OIDs from the repository — topo order as git log produces them.
        let commits = vec![
            // idx 0: 795f1ba — codex/demo-wt-commit branch tip
            make_real_commit(
                "795f1bae4648ac10a13e597a68520e07f948618c",
                &["e5d62851d80d237a098ea6d8853653c5e06b25a4"],
                vec![RefLabel::LocalBranch("codex/demo-wt-commit".into())],
            ),
            // idx 1: 0308d39 — codex/demo-wt-pending branch tip
            make_real_commit(
                "0308d393027b9f094a89a1f12347c4af60441a5a",
                &["3ce915d70b21531099ca7535ee6c85860eeb173f"],
                vec![RefLabel::LocalBranch("codex/demo-wt-pending".into())],
            ),
            // idx 2: f50abd6 — HEAD, main, origin/main
            make_real_commit(
                "f50abd6ef5b1830b8e1ac416018c2b5ffe44ec70",
                &["52c706572bef2796495d9cbb21d41d3e6db72456"],
                vec![
                    RefLabel::Head,
                    RefLabel::LocalBranch("main".into()),
                    RefLabel::RemoteBranch("origin/main".into()),
                ],
            ),
            // idx 3: 52c7065
            make_real_commit(
                "52c706572bef2796495d9cbb21d41d3e6db72456",
                &["9b17780bcb0c8fd865cfecc5d277302210a6b076"],
                vec![],
            ),
            // idx 4: 9b17780
            make_real_commit(
                "9b17780bcb0c8fd865cfecc5d277302210a6b076",
                &["1bb477f600d328841ee81675dc2fb3929942cf59"],
                vec![],
            ),
            // idx 5: 1bb477f (parent = e5d6285, which is the fork point for wt-commit)
            make_real_commit(
                "1bb477f600d328841ee81675dc2fb3929942cf59",
                &["e5d62851d80d237a098ea6d8853653c5e06b25a4"],
                vec![],
            ),
            // idx 6: e5d6285 (fork point 1)
            make_real_commit(
                "e5d62851d80d237a098ea6d8853653c5e06b25a4",
                &["93282319230eb1d6c31e74e1af98092718fba3b2"],
                vec![],
            ),
            // idx 7: 9328231
            make_real_commit(
                "93282319230eb1d6c31e74e1af98092718fba3b2",
                &["ba6e82166f7c291d0c0f21dcc78e97eba5062e46"],
                vec![],
            ),
            // idx 8: ba6e821 (parent = 3ce915d, which is the fork point for wt-pending)
            make_real_commit(
                "ba6e82166f7c291d0c0f21dcc78e97eba5062e46",
                &["3ce915d70b21531099ca7535ee6c85860eeb173f"],
                vec![],
            ),
            // idx 9: 3ce915d (fork point 2)
            make_real_commit(
                "3ce915d70b21531099ca7535ee6c85860eeb173f",
                &["bc215a60757d394a0fc3e5cbf51476c95081f8ee"],
                vec![],
            ),
            // idx 10: bc215a6 (root for this test)
            make_real_commit(
                "bc215a60757d394a0fc3e5cbf51476c95081f8ee",
                &[],
                vec![],
            ),
        ];

        let rows = compute_graph(&commits);
        assert_eq!(rows.len(), 11, "should produce 11 rows");

        // ═══════════════════════════════════════════════════════════
        // Dump all rows for debugging on failure
        // ═══════════════════════════════════════════════════════════
        for (i, r) in rows.iter().enumerate() {
            let edges_str: Vec<_> = r.edges.iter()
                .map(|e| format!("{}→{}(c{}{})", e.from_lane, e.to_lane, e.color_index,
                    if e.is_merge { ",m" } else { "" }))
                .collect();
            eprintln!(
                "  row {:2} [{}]: lane={} color={} incoming={} head={} merge={} lanes={} edges=[{}]",
                i, &commits[i].short_id, r.node_lane, r.node_color,
                r.has_incoming, r.is_head, r.is_merge, r.lane_count,
                edges_str.join(", "),
            );
        }

        // ═══════════════════════════════════════════════════════════
        // 1. LANE ASSIGNMENTS — main on lane 0, branches elsewhere
        // ═══════════════════════════════════════════════════════════

        // idx 0: 795f1ba (wt-commit) — NOT on main, should be lane 1
        assert_ne!(rows[0].node_lane, 0, "795f1ba must not be on lane 0");
        let wt_commit_lane = rows[0].node_lane;

        // idx 1: 0308d39 (wt-pending) — NOT on main
        assert_ne!(rows[1].node_lane, 0, "0308d39 must not be on lane 0");
        let wt_pending_lane = rows[1].node_lane;

        // idx 2-10: all on lane 0 (entire main chain)
        for idx in 2..=10 {
            assert_eq!(rows[idx].node_lane, 0,
                "idx {} ({}) must be lane 0 (main chain), was lane {}",
                idx, commits[idx].short_id, rows[idx].node_lane);
        }

        // ═══════════════════════════════════════════════════════════
        // 2. HEAD — only idx 2 (f50abd6) is HEAD
        // ═══════════════════════════════════════════════════════════

        for (i, r) in rows.iter().enumerate() {
            if i == 2 {
                assert!(r.is_head, "idx 2 (f50abd6) IS HEAD");
            } else {
                assert!(!r.is_head, "idx {} is not HEAD", i);
            }
        }

        // ═══════════════════════════════════════════════════════════
        // 3. HAS_INCOMING
        // ═══════════════════════════════════════════════════════════

        // Branch tips: no line drawn from above into them
        assert!(!rows[0].has_incoming, "795f1ba: new branch, no incoming");
        assert!(!rows[1].has_incoming, "0308d39: new branch, no incoming");

        // Main HEAD (f50abd6): lane 0 was free before it, no incoming
        assert!(!rows[2].has_incoming, "f50abd6: first on lane 0, no incoming");

        // Every other main commit: continuation, has incoming
        for idx in 3..=10 {
            assert!(rows[idx].has_incoming,
                "idx {} should have incoming (lane 0 continuation)", idx);
        }

        // ═══════════════════════════════════════════════════════════
        // 4. COLORS — consistent within each branch
        // ═══════════════════════════════════════════════════════════

        let main_color = rows[2].node_color;
        for idx in 3..=10 {
            assert_eq!(rows[idx].node_color, main_color,
                "idx {} color should match main ({}), was {}",
                idx, main_color, rows[idx].node_color);
        }

        // ═══════════════════════════════════════════════════════════
        // 5. EDGE: branch tip outgoing edges
        // ═══════════════════════════════════════════════════════════

        // 795f1ba (idx 0): outgoing edge from its lane to wherever
        // fork1 (e5d6285) was routed — should stay on its own lane
        let idx0_out: Vec<_> = rows[0].edges.iter()
            .filter(|e| e.from_lane == wt_commit_lane)
            .collect();
        assert_eq!(idx0_out.len(), 1, "795f1ba: 1 outgoing edge from lane {}", wt_commit_lane);

        // 0308d39 (idx 1): outgoing edge from its lane.
        // fork2 (3ce915d) is an ancestor of e5d6285, so route_parent's
        // ancestor match routes it to the same lane as fork1.
        let idx1_out: Vec<_> = rows[1].edges.iter()
            .filter(|e| e.from_lane == wt_pending_lane)
            .collect();
        assert_eq!(idx1_out.len(), 1, "0308d39: 1 outgoing edge from lane {}", wt_pending_lane);

        // Both branches' parents end up on the same lane (ancestor match)
        let fork1_target = idx0_out[0].to_lane;
        let fork2_target = idx1_out[0].to_lane;
        assert_eq!(fork1_target, fork2_target,
            "both fork parents routed to same lane via ancestor match");

        // The shared target lane is the wt_commit lane (fork1's original lane)
        let branch_lane = fork1_target;

        // ═══════════════════════════════════════════════════════════
        // 6. PASS-THROUGH: branch lane visible while active
        // ═══════════════════════════════════════════════════════════

        let has_pt = |row: &GraphRow, lane: usize| -> bool {
            row.edges.iter().any(|e| e.from_lane == lane && e.to_lane == lane)
        };

        // idx 1: branch_lane passes through (fork1/fork2 sitting there)
        assert!(has_pt(&rows[1], branch_lane),
            "idx 1: branch lane {} should pass through", branch_lane);

        // idx 2-5: branch_lane passes through alongside main
        for idx in 2..=5 {
            assert!(has_pt(&rows[idx], branch_lane),
                "idx {}: branch lane {} should pass through", idx, branch_lane);
        }

        // idx 6 (e5d6285): branch_lane STILL passes through because
        // route_parent updated lane 1 to hold 3ce915d (not e5d6285),
        // so zombie cleanup for e5d6285 doesn't clear it.
        assert!(has_pt(&rows[6], branch_lane),
            "idx 6: branch lane {} should still pass through (holds 3ce915d)", branch_lane);

        // idx 7-8: branch_lane still passes through (3ce915d still pending)
        for idx in 7..=8 {
            assert!(has_pt(&rows[idx], branch_lane),
                "idx {}: branch lane {} should pass through (3ce915d pending)", idx, branch_lane);
        }

        // ═══════════════════════════════════════════════════════════
        // 7. MERGE CURVES — edges from main to branch lane at fork points
        // ═══════════════════════════════════════════════════════════

        // idx 5 (1bb477f): parent is e5d6285 (on main chain).
        // Main edge should go from lane 0 to lane 0 (parent routed to lane 0).
        let idx5_main = rows[5].edges.iter()
            .find(|e| e.from_lane == 0 && !e.is_merge);
        assert!(idx5_main.is_some(), "1bb477f: should have main outgoing edge");
        assert_eq!(idx5_main.unwrap().to_lane, 0, "1bb477f: main edge stays on lane 0");

        // idx 8 (ba6e821): parent is 3ce915d (on main chain).
        // The parent 3ce915d should be routed to lane 0 (main chain routing),
        // but 3ce915d is ALSO on branch_lane (from the ancestor match).
        // Main-chain routing should put it on lane 0. The branch_lane copy
        // gets cleared by zombie cleanup when 3ce915d arrives.
        let idx8_main = rows[8].edges.iter()
            .find(|e| e.from_lane == 0 && !e.is_merge);
        assert!(idx8_main.is_some(), "ba6e821: should have main outgoing edge");
        assert_eq!(idx8_main.unwrap().to_lane, 0, "ba6e821: main edge stays on lane 0");

        // ═══════════════════════════════════════════════════════════
        // 8. LANE COMPACTION — lanes freed after fork points
        // ═══════════════════════════════════════════════════════════

        // After idx 9 (3ce915d consumed): branch_lane should be cleared.
        // idx 10 should have only lane 0.
        assert_eq!(rows[10].lane_count, 1,
            "bc215a6: only lane 0 should remain, lane_count = {}", rows[10].lane_count);

        // No pass-through edges at idx 10
        let idx10_pt = rows[10].edges.iter()
            .filter(|e| e.from_lane == e.to_lane)
            .count();
        assert_eq!(idx10_pt, 0, "bc215a6: no pass-through edges, found {}", idx10_pt);
    }
}
