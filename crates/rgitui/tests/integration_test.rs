//! Integration tests for rgitui.
//!
//! These tests verify core functionality without requiring a display.
//! For visual testing, use scripts/screenshot.sh

use chrono::TimeZone;
use std::path::PathBuf;

/// Creates a temporary git repository with several commits and a branch.
///
/// Returns `(TempDir, PathBuf)` where `TempDir` keeps the directory alive and
/// `PathBuf` is the path to the repository root.
fn setup_test_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_path = tmp.path().to_path_buf();
    let repo = git2::Repository::init(&repo_path).expect("failed to init repo");

    let mut config = repo.config().expect("failed to get repo config");
    config
        .set_str("user.name", "Test User")
        .expect("failed to set user.name");
    config
        .set_str("user.email", "test@example.com")
        .expect("failed to set user.email");

    let sig =
        git2::Signature::now("Test User", "test@example.com").expect("failed to create signature");

    // Initial commit
    let file_path = repo_path.join("README.md");
    std::fs::write(&file_path, "# Test Repository\n").expect("failed to write README");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add README");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    let initial_commit = repo
        .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .expect("failed to create initial commit");
    let initial_commit = repo
        .find_commit(initial_commit)
        .expect("failed to find initial commit");

    // Create a feature branch
    repo.branch("feature-branch", &initial_commit, false)
        .expect("failed to create branch");

    // Second commit on main
    std::fs::write(&file_path, "# Test Repository\n\nUpdated content.\n")
        .expect("failed to write updated README");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add README");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    let second_commit = repo
        .commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Update README",
            &tree,
            &[&initial_commit],
        )
        .expect("failed to create second commit");
    let second_commit = repo
        .find_commit(second_commit)
        .expect("failed to find second commit");

    // Third commit
    let src_dir = repo_path.join("src");
    std::fs::create_dir_all(&src_dir).expect("failed to create src dir");
    std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("failed to write main.rs");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("src/main.rs"))
        .expect("failed to add main.rs");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Add main.rs source file",
        &tree,
        &[&second_commit],
    )
    .expect("failed to create third commit");

    (tmp, repo_path)
}

#[test]
fn test_repo_path_exists() {
    let (_tmp, path) = setup_test_repo();
    assert!(path.exists());
    assert!(path.is_dir());
}

#[test]
fn test_git_repo_can_be_opened() {
    let (_tmp, path) = setup_test_repo();

    let repo = git2::Repository::discover(&path).expect("failed to open git repo");
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
    let (_tmp, path) = setup_test_repo();

    let repo = git2::Repository::discover(&path).expect("failed to open repo");
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
    let (_tmp, path) = setup_test_repo();

    let repo = git2::Repository::discover(&path).expect("failed to open repo");
    let mut revwalk = repo.revwalk().expect("failed to create revwalk");
    revwalk.push_head().expect("failed to push HEAD");

    let commit_count = revwalk.count();
    assert_eq!(commit_count, 3, "Test repo should have exactly 3 commits");
}

#[test]
fn test_repo_has_feature_branch() {
    let (_tmp, path) = setup_test_repo();
    let repo = git2::Repository::discover(&path).expect("failed to open repo");

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
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_path = tmp.path();
    let mut init_opts = git2::RepositoryInitOptions::new();
    init_opts.initial_head("main");
    let repo = git2::Repository::init_opts(repo_path, &init_opts).expect("failed to init repo");
    let mut config = repo.config().expect("failed to get config");
    config
        .set_str("user.name", "Test")
        .expect("failed to set name");
    config
        .set_str("user.email", "test@test.com")
        .expect("failed to set email");
    drop(repo);

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
    drop(tmp);
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

    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_path = tmp.path().to_path_buf();
    let mut init_opts = git2::RepositoryInitOptions::new();
    init_opts.initial_head("main");
    let repo = git2::Repository::init_opts(&repo_path, &init_opts).expect("failed to init repo");

    let mut config = repo.config().expect("failed to get repo config");
    config
        .set_str("user.name", "Test User")
        .expect("failed to set user.name");
    config
        .set_str("user.email", "test@example.com")
        .expect("failed to set user.email");

    let sig =
        git2::Signature::now("Test User", "test@example.com").expect("failed to create signature");

    // C4: initial commit
    let file = repo_path.join("README.md");
    std::fs::write(&file, "# Test\n").expect("failed to write README");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    let c4 = repo
        .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .expect("failed to create c4");
    let c4 = repo.find_commit(c4).expect("failed to find c4");

    // Create a feature branch from C4
    repo.branch("feature", &c4, false)
        .expect("failed to create feature branch");

    // C3: commit on main (first parent of merge)
    std::fs::write(&file, "# Test\nMain change\n").expect("failed to update README");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    let c3 = repo
        .commit(Some("HEAD"), &sig, &sig, "Update on main", &tree, &[&c4])
        .expect("failed to create c3");
    let c3 = repo.find_commit(c3).expect("failed to find c3");

    // Checkout feature branch and make C2
    let _feature_branch = repo
        .find_branch("feature", BranchType::Local)
        .expect("failed to find feature branch");
    repo.set_head("refs/heads/feature")
        .expect("failed to checkout feature");
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .expect("failed to checkout feature");

    std::fs::write(&file, "# Test\nFeature change\n").expect("failed to update README");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    let c2 = repo
        .commit(Some("HEAD"), &sig, &sig, "Update on feature", &tree, &[&c4])
        .expect("failed to create c2");

    // Go back to main and make merge commit C1
    repo.set_head("refs/heads/main")
        .expect("failed to checkout main");
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .expect("failed to checkout main");

    let c2_commit = repo.find_commit(c2).expect("failed to find c2");
    let mut index = repo.index().expect("failed to get index");
    index
        .add_path(std::path::Path::new("README.md"))
        .expect("failed to add");
    index.write().expect("failed to write index");
    let tree_oid = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_oid).expect("failed to find tree");
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Merge feature into main",
        &tree,
        &[&c3, &c2_commit],
    )
    .expect("failed to create merge commit");

    // Collect all commits (HEAD = main with merge commit)
    let repo = git2::Repository::discover(&repo_path).expect("failed to open repo");
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

    drop(tmp); // explicitly drop to clean up
}
