//! Git grep — global content search.

use anyhow::{Context as _, Result};
use gpui::{AsyncApp, Context, Task, WeakEntity};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::GitProject;
use crate::types::SearchResult;

impl GitProject {
    /// Run `git grep` synchronously.
    pub fn git_grep(&self, pattern: &str) -> Result<Vec<SearchResult>> {
        git_grep(self.repo_path(), pattern)
    }

    /// Run `git grep` asynchronously.
    pub fn git_grep_async(
        &self,
        pattern: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<SearchResult>>> {
        let repo_path = self.repo_path().to_path_buf();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .spawn(async move { git_grep(&repo_path, &pattern) })
                .await
        })
    }
}

/// Run `git grep` in the given repository.
pub fn git_grep(repo_path: &Path, pattern: &str) -> Result<Vec<SearchResult>> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["grep", "-n", pattern])
        .output()
        .context("Failed to execute git grep")?;

    if !output.status.success() {
        // Exit code 1 means "no matches" — not an error, return empty.
        if output.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git grep failed: {}", stderr.trim());
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let results = parse_grep_output(&raw);
    Ok(results)
}

/// Parse git grep output in the format "file:lineno:content".
/// Path may contain colons (e.g., "src/foo:bar.rs"), so we split on the
/// first colon for path, then the next colon for line number.
fn parse_grep_output(raw: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for line in raw.lines() {
        // Output format: "file:lineno:content"
        let Some((file_part, rest)) = line.split_once(':') else {
            continue;
        };
        let path = PathBuf::from(file_part);

        let Some((line_str, content)) = rest.split_once(':') else {
            // Malformed line — skip.
            continue;
        };

        let line_number = line_str.parse::<usize>().unwrap_or(1);

        results.push(SearchResult {
            path,
            line_number,
            content: content.to_string(),
        });
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_test_repo() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Alice").unwrap();
        cfg.set_str("user.email", "alice@example.com").unwrap();
        drop(cfg);

        let sig = git2::Signature::now("Alice", "alice@example.com").unwrap();
        let msg = "Initial commit\n";

        // Create a file with search content.
        fs::write(
            path.join("hello.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        fs::write(path.join("other.txt"), "nothing here\n").unwrap();

        // Add files to index and commit.
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("hello.rs")).unwrap();
        idx.add_path(std::path::Path::new("other.txt")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[])
            .unwrap();

        (dir, path)
    }

    #[test]
    fn git_grep_finds_match() {
        let (_dir, path) = make_test_repo();
        let results = git_grep(&path, "println").unwrap();
        assert_eq!(results.len(), 1, "expected 1 match, got {:?}", results);
        assert_eq!(
            results[0].line_number, 2,
            "expected line 2, got {}",
            results[0].line_number
        );
        assert!(
            results[0].content.contains("println"),
            "expected 'println' in content, got: {}",
            results[0].content
        );
    }

    #[test]
    fn git_grep_multiple_matches() {
        let (_dir, path) = make_test_repo();
        let results = git_grep(&path, "fn").unwrap();
        assert_eq!(results.len(), 1, "expected 1 match, got {:?}", results);
    }

    #[test]
    fn git_grep_no_matches() {
        let (_dir, path) = make_test_repo();
        let results = git_grep(&path, "xyzzy").unwrap();
        assert_eq!(results.len(), 0);
    }
}
