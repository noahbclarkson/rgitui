//! Integration tests for rgitui.
//!
//! These tests verify core functionality without requiring a display.
//! For visual testing, use scripts/screenshot.sh

use chrono::TimeZone;
use rgitui_test_support::TempRepo;
use std::path::PathBuf;

/// Creates a temporary git repository with three commits and a branch that was
/// cut from the first of them.
fn setup_test_repo() -> TempRepo {
    let repo = TempRepo::init();
    repo.commit_file("README.md", "# Test Repository\n", "Initial commit");
    repo.branch("feature-branch");
    repo.commit_file(
        "README.md",
        "# Test Repository\n\nUpdated content.\n",
        "Update README",
    );
    repo.commit_file("src/main.rs", "fn main() {}\n", "Add main.rs source file");
    repo
}

#[test]
fn test_repo_path_exists() {
    let fixture = setup_test_repo();
    assert!(fixture.path().exists());
    assert!(fixture.path().is_dir());
}

#[test]
fn test_git_repo_can_be_opened() {
    let fixture = setup_test_repo();

    let repo = git2::Repository::discover(fixture.path()).expect("failed to open git repo");
    let head = repo.head().expect("failed to get HEAD");
    assert!(head.shorthand().is_some());
}

#[test]
fn test_compute_graph_empty() {
    let rows = rgitui_git::compute_graph(&[]);
    assert!(rows.is_empty(), "Empty commits should produce empty graph");
}

#[test]
fn test_compute_graph_with_real_commits() {
    let fixture = setup_test_repo();

    let repo = git2::Repository::discover(fixture.path()).expect("failed to open repo");
    let mut revwalk = repo.revwalk().expect("failed to create revwalk");
    revwalk.push_head().expect("failed to push HEAD to revwalk");

    let mut commits: Vec<rgitui_git::CommitInfo> = Vec::new();
    for oid_result in revwalk.take(200) {
        let oid = match oid_result {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let author_sig = commit.author();
        let committer_sig = commit.committer();
        let time_secs = commit.time().seconds();
        let time = chrono::Utc
            .timestamp_opt(time_secs, 0)
            .single()
            .unwrap_or_else(chrono::Utc::now);

        commits.push(rgitui_git::CommitInfo {
            oid,
            short_id: oid.to_string()[..7].to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author: rgitui_git::Signature {
                name: author_sig.name().unwrap_or("").to_string(),
                email: author_sig.email().unwrap_or("").to_string(),
            },
            committer: rgitui_git::Signature {
                name: committer_sig.name().unwrap_or("").to_string(),
                email: committer_sig.email().unwrap_or("").to_string(),
            },
            co_authors: vec![],
            time,
            parent_oids: commit.parent_ids().collect(),
            refs: Vec::new(),
            is_signed: false,
        });
    }

    assert!(
        !commits.is_empty(),
        "Test repo should have at least one commit"
    );

    let rows = rgitui_git::compute_graph(&commits);
    assert_eq!(
        rows.len(),
        commits.len(),
        "Graph rows should match commit count"
    );
    for row in &rows {
        assert!(
            row.commit_index < commits.len(),
            "Row commit_index out of bounds"
        );
    }
}

#[test]
fn test_compute_graph_correct_commit_count() {
    let fixture = setup_test_repo();

    let repo = git2::Repository::discover(fixture.path()).expect("failed to open repo");
    let mut revwalk = repo.revwalk().expect("failed to create revwalk");
    revwalk.push_head().expect("failed to push HEAD");

    let commit_count = revwalk.count();
    assert_eq!(commit_count, 3, "Test repo should have exactly 3 commits");
}

#[test]
fn test_repo_has_feature_branch() {
    let fixture = setup_test_repo();
    let repo = git2::Repository::discover(fixture.path()).expect("failed to open repo");

    let branch = repo
        .find_branch("feature-branch", git2::BranchType::Local)
        .expect("feature-branch should exist");
    assert!(!branch.is_head());
}

/// Test that the app binary exists after build
#[test]
#[ignore] // Only run manually
fn test_binary_launches() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let binary = PathBuf::from(manifest)
        .ancestors()
        .nth(2)
        .unwrap()
        .join("target/debug/rgitui");

    if !binary.exists() {
        println!("Binary not built yet, skipping");
        return;
    }

    println!("Binary exists at: {}", binary.display());
}

/// Headless GUI smoke test: launch the app with Lavapipe software renderer and
/// verify it starts up without immediately crashing.
///
/// Requires: Lavapipe Vulkan ICD (`lvp_icd.json`), Xvfb (`xvfb-run`).
/// Run with: `cargo test test_headless_smoke -- --include-ignored`
#[test]
#[ignore = "requires Lavapipe + Xvfb; run manually with --include-ignored"]
fn test_headless_smoke() {
    use std::process::Command;

    let manifest = env!("CARGO_MANIFEST_DIR");
    let binary = PathBuf::from(manifest)
        .ancestors()
        .nth(2)
        .unwrap()
        .join("target/debug/rgitui");

    if !binary.exists() {
        println!("Binary not built yet, skipping");
        return;
    }

    // Verify Lavapipe ICD and xvfb-run are available
    let lvp_icd = "/usr/share/vulkan/icd.d/lvp_icd.json";
    if !std::path::Path::new(lvp_icd).exists() {
        println!("Lavapipe ICD not found at {}, skipping", lvp_icd);
        return;
    }

    let xvfb = std::path::Path::new("/usr/bin/xvfb-run");
    if !xvfb.exists() {
        println!("xvfb-run not found, skipping");
        return;
    }

    // Create a temp repo to open
    let fixture = TempRepo::init();
    let repo_path = fixture.path();

    // Launch with Lavapipe + Xvfb, give it 5 seconds to start, then terminate
    let mut child = Command::new("/usr/bin/xvfb-run")
        .args([
            "-a",
            "--",
            "env",
            &format!("VK_ICD_FILENAMES={}", lvp_icd),
            "DISPLAY=:99",
        ])
        .arg(&binary)
        .arg(repo_path)
        .spawn()
        .expect("Failed to spawn rgitui under xvfb");

    std::thread::sleep(std::time::Duration::from_secs(5));

    // Check if the process is still running (didn't crash on startup)
    match child.try_wait() {
        Ok(Some(status)) => {
            // Process exited — check it was a clean exit
            assert!(
                status.success(),
                "rgitui exited unexpectedly with status: {}",
                status
            );
        }
        Ok(None) => {
            // Still running — good, it started successfully
            println!(
                "rgitui started successfully under Lavapipe/Xvfb (PID: {})",
                child.id()
            );
        }
        Err(e) => {
            panic!("Failed to check process status: {}", e);
        }
    }

    // Clean up
    child.kill().ok();
    drop(fixture);
}

/// Helper: convert a git commit iterator result into a CommitInfo.
fn commit_to_commit_info(commit: &git2::Commit) -> rgitui_git::CommitInfo {
    let author_sig = commit.author();
    let committer_sig = commit.committer();
    let time_secs = commit.time().seconds();
    let time = chrono::Utc
        .timestamp_opt(time_secs, 0)
        .single()
        .unwrap_or_else(chrono::Utc::now);

    rgitui_git::CommitInfo {
        oid: commit.id(),
        short_id: commit.id().to_string()[..7].to_string(),
        summary: commit.summary().unwrap_or_default().to_string(),
        message: commit.message().unwrap_or_default().to_string(),
        author: rgitui_git::Signature {
            name: author_sig.name().unwrap_or_default().to_string(),
            email: author_sig.email().unwrap_or_default().to_string(),
        },
        committer: rgitui_git::Signature {
            name: committer_sig.name().unwrap_or_default().to_string(),
            email: committer_sig.email().unwrap_or_default().to_string(),
        },
        co_authors: vec![],
        time,
        parent_oids: commit.parent_ids().collect(),
        refs: vec![],
        is_signed: false,
    }
}

/// Integration test: compute_graph correctly handles a repo with a merge commit.
///
/// Creates this topology:
///
///   C1 (HEAD/main) ─── merge (parents: C2, C3)
///   C2 ─── C4 (feature branch tip)
///   C3 ─── C4
///   C4 (initial)
///
/// The merge commit has two parents, which is a key edge case for lane assignment.
#[test]
fn test_compute_graph_handles_merge_commit() {
    use git2::BranchType;

    let fixture = TempRepo::init();
    let repo = fixture.repo();

    // C4: initial commit, then a feature branch cut from it
    fixture.commit_file("README.md", "# Test\n", "Initial commit");
    fixture.branch("feature");

    // C3: commit on main (first parent of merge)
    let c3 = fixture.commit_file("README.md", "# Test\nMain change\n", "Update on main");

    // Checkout feature branch and make C2
    let _feature_branch = repo
        .find_branch("feature", BranchType::Local)
        .expect("failed to find feature branch");
    repo.set_head("refs/heads/feature")
        .expect("failed to checkout feature");
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .expect("failed to checkout feature");
    let c2 = fixture.commit_file("README.md", "# Test\nFeature change\n", "Update on feature");

    // Go back to main and make merge commit C1
    repo.set_head("refs/heads/main")
        .expect("failed to checkout main");
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .expect("failed to checkout main");

    let tree_oid = repo.index().unwrap().write_tree().unwrap();
    fixture.commit_tree(Some("HEAD"), "Merge feature into main", tree_oid, &[c3, c2]);

    // Collect all commits (HEAD = main with merge commit)
    let mut revwalk = repo.revwalk().expect("failed to create revwalk");
    revwalk.push_head().expect("failed to push HEAD");

    let commits: Vec<_> = revwalk
        .filter_map(|oid| oid.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|c| commit_to_commit_info(&c))
        .collect();

    assert_eq!(
        commits.len(),
        4,
        "Should have 4 commits (c4, c3, c2, merge)"
    );

    // The merge commit should be first (most recent) and must have 2 parents
    assert_eq!(
        commits[0].parent_oids.len(),
        2,
        "Merge commit must have 2 parents"
    );
    // The two branch commits (c3 and c2) each have exactly 1 parent
    assert_eq!(
        commits[1].parent_oids.len(),
        1,
        "Branch commit c3 should have 1 parent"
    );
    assert_eq!(
        commits[2].parent_oids.len(),
        1,
        "Branch commit c2 should have 1 parent"
    );
    // Initial commit c4 has no parents
    assert_eq!(
        commits[3].parent_oids.len(),
        0,
        "Initial commit c4 should have 0 parents"
    );

    let rows = rgitui_git::compute_graph(&commits);
    assert_eq!(rows.len(), 4, "Graph should have 4 rows");

    // Verify no commit index is out of bounds
    for row in &rows {
        assert!(
            row.commit_index < commits.len(),
            "commit_index {} out of bounds for {} commits",
            row.commit_index,
            commits.len()
        );
    }

    // Verify all rows have a node position (lane assignment exists)
    for (i, row) in rows.iter().enumerate() {
        assert!(
            row.node_lane < 16,
            "row {} should have a sane node_lane (< 16), got {}",
            i,
            row.node_lane
        );
    }

    // Verify merge commit (index 0) appears in the rows and has at least 2 edges
    // (one from each parent branch), which is the structural signature of a merge
    let merge_row_index = rows
        .iter()
        .position(|r| r.commit_index == 0)
        .expect("merge commit (index 0) must appear in graph rows");
    let merge_row = &rows[merge_row_index];
    assert!(
        merge_row.edges.len() >= 2,
        "Merge commit should have at least 2 edges (one per parent branch), got {}",
        merge_row.edges.len()
    );
}
