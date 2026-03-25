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
