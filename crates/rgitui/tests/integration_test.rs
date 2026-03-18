//! Integration tests for rgitui.
//!
//! These tests verify core functionality without requiring a display.
//! For visual testing, use scripts/screenshot.sh

use chrono::TimeZone;
use std::path::PathBuf;

/// Returns the test repo path from environment variable or default.
fn test_repo_path() -> PathBuf {
    let path =
        std::env::var("RGITUI_TEST_REPO").unwrap_or_else(|_| "/home/noah/src/krypto".to_string());
    PathBuf::from(path)
}

#[test]
fn test_repo_path_exists() {
    let path = test_repo_path();
    if path.exists() {
        println!("Test repo found at: {}", path.display());
        assert!(path.is_dir(), "RGITUI_TEST_REPO must be a directory");
    } else {
        println!("Test repo not found at {} - skipping", path.display());
        // Not a failure - just skip if repo not present
    }
}

#[test]
fn test_git_repo_can_be_opened() {
    let path = test_repo_path();
    if !path.exists() {
        println!("Skipping - test repo not present");
        return;
    }

    match git2::Repository::discover(&path) {
        Ok(repo) => {
            println!("Opened repo: {:?}", repo.path());
            let head = repo.head().expect("Failed to get HEAD");
            println!("HEAD: {}", head.shorthand().unwrap_or("detached"));
        }
        Err(e) => {
            panic!("Failed to open git repo at {}: {}", path.display(), e);
        }
    }
}

#[test]
fn test_compute_graph_empty() {
    // Test the graph layout algorithm with empty input - no repo needed
    let rows = rgitui_git::compute_graph(&[]);
    assert!(rows.is_empty(), "Empty commits should produce empty graph");
}

#[test]
fn test_compute_graph_with_real_commits() {
    let path = test_repo_path();
    if !path.exists() {
        println!("Skipping - test repo not present");
        return;
    }

    let repo = match git2::Repository::discover(&path) {
        Ok(r) => r,
        Err(e) => {
            println!("Skipping - could not open repo: {}", e);
            return;
        }
    };

    // Walk commits manually using git2 and convert to rgitui_git types
    let mut revwalk = match repo.revwalk() {
        Ok(w) => w,
        Err(e) => {
            println!("Skipping - revwalk failed: {}", e);
            return;
        }
    };

    if revwalk.push_head().is_err() {
        println!("Skipping - no HEAD to walk from");
        return;
    }

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
            .unwrap_or_else(|| chrono::Utc::now());

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
        });
    }

    if commits.is_empty() {
        println!("Repo has no commits, skipping graph test");
        return;
    }

    let rows = rgitui_git::compute_graph(&commits);
    assert_eq!(
        rows.len(),
        commits.len(),
        "Graph rows should match commit count"
    );
    println!(
        "Graph computed: {} commits -> {} rows",
        commits.len(),
        rows.len()
    );
    for row in &rows {
        assert!(
            row.commit_index < commits.len(),
            "Row commit_index out of bounds"
        );
    }
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
