use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use git2::Repository;
use std::path::Path;

/// A single blame annotation for a line or range of lines.
#[derive(Debug, Clone)]
pub struct BlameEntry {
    pub oid: git2::Oid,
    pub short_id: String,
    pub author: String,
    pub email: String,
    pub time: DateTime<Utc>,
    pub start_line: usize,
    pub line_count: usize,
}

/// A single line with its blame annotation.
#[derive(Debug, Clone)]
pub struct BlameLine {
    pub line_no: usize,
    pub content: String,
    pub entry: BlameEntry,
}

/// Compute blame for a file at a given commit (or HEAD if None).
pub fn compute_blame(
    repo_path: &Path,
    file_path: &Path,
    commit_oid: Option<git2::Oid>,
) -> Result<Vec<BlameLine>> {
    let repo = Repository::open(repo_path)?;

    let mut opts = git2::BlameOptions::new();
    if let Some(oid) = commit_oid {
        opts.newest_commit(oid);
    }

    let blame = repo.blame_file(file_path, Some(&mut opts))?;

    let content = if let Some(oid) = commit_oid {
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let entry = tree.get_path(file_path)?;
        let blob = repo.find_blob(entry.id())?;
        String::from_utf8_lossy(blob.content()).to_string()
    } else {
        let full_path = repo
            .workdir()
            .ok_or_else(|| anyhow::anyhow!("Bare repository"))?
            .join(file_path);
        std::fs::read_to_string(full_path)?
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());

    for (i, line_content) in lines.iter().enumerate() {
        let line_no = i + 1;
        if let Some(hunk) = blame.get_line(line_no) {
            let sig = hunk.final_signature();
            let author = String::from_utf8_lossy(sig.name_bytes()).to_string();
            let email = String::from_utf8_lossy(sig.email_bytes()).to_string();
            let time = Utc
                .timestamp_opt(sig.when().seconds(), 0)
                .single()
                .unwrap_or_default();

            let oid = hunk.final_commit_id();
            let oid_str = oid.to_string();
            let short_id = oid_str[..7.min(oid_str.len())].to_string();

            result.push(BlameLine {
                line_no,
                content: line_content.to_string(),
                entry: BlameEntry {
                    oid,
                    short_id,
                    author,
                    email,
                    time,
                    start_line: hunk.final_start_line(),
                    line_count: hunk.lines_in_hunk(),
                },
            });
        }
    }

    Ok(result)
}

use gpui::{AsyncApp, Context, Task, WeakEntity};

use super::GitProject;

impl GitProject {
    /// Compute blame for a file, optionally at a specific commit.
    pub fn blame_file(
        &self,
        file_path: &Path,
        commit_oid: Option<git2::Oid>,
    ) -> Result<Vec<BlameLine>> {
        compute_blame(self.repo_path(), file_path, commit_oid)
    }

    /// Compute blame asynchronously on a background thread.
    pub fn blame_file_async(
        &self,
        file_path: &Path,
        commit_oid: Option<git2::Oid>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<BlameLine>>> {
        let repo_path = self.repo_path().to_path_buf();
        let file_path = file_path.to_path_buf();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .spawn(async move { compute_blame(&repo_path, &file_path, commit_oid) })
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Create a repo with two commits both touching the same file.
    /// commit A: adds "line1\nline2\n"
    /// commit B: appends "line3\n"
    fn make_blame_repo() -> (TempDir, std::path::PathBuf, git2::Oid, git2::Oid) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = git2::Repository::init(&path).unwrap();

        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Alice").unwrap();
        cfg.set_str("user.email", "alice@example.com").unwrap();
        drop(cfg);

        let sig = git2::Signature::now("Alice", "alice@example.com").unwrap();

        // Commit A
        let file = path.join("code.txt");
        std::fs::write(&file, "line1\nline2\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("code.txt")).unwrap();
        idx.write().unwrap();
        let tree_a = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let oid_a = repo
            .commit(Some("HEAD"), &sig, &sig, "initial commit", &tree_a, &[])
            .unwrap();

        // Commit B — authored by Bob
        let sig_b = git2::Signature::now("Bob", "bob@example.com").unwrap();
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();
        let mut idx2 = repo.index().unwrap();
        idx2.add_path(Path::new("code.txt")).unwrap();
        idx2.write().unwrap();
        let tree_b = repo.find_tree(idx2.write_tree().unwrap()).unwrap();
        let commit_a = repo.find_commit(oid_a).unwrap();
        let oid_b = repo
            .commit(
                Some("HEAD"),
                &sig_b,
                &sig_b,
                "add line3",
                &tree_b,
                &[&commit_a],
            )
            .unwrap();

        (dir, path, oid_a, oid_b)
    }

    #[test]
    fn blame_head_returns_all_lines() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        assert_eq!(
            lines.len(),
            3,
            "expected 3 blame lines, got {}",
            lines.len()
        );
    }

    #[test]
    fn blame_line_numbers_are_one_indexed() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        for (i, bl) in lines.iter().enumerate() {
            assert_eq!(bl.line_no, i + 1, "line_no should be {}", i + 1);
        }
    }

    #[test]
    fn blame_line_contents_match_file() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        assert_eq!(lines[0].content, "line1");
        assert_eq!(lines[1].content, "line2");
        assert_eq!(lines[2].content, "line3");
    }

    #[test]
    fn blame_first_two_lines_authored_by_alice() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        assert_eq!(lines[0].entry.author, "Alice");
        assert_eq!(lines[1].entry.author, "Alice");
    }

    #[test]
    fn blame_last_line_authored_by_bob() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        assert_eq!(lines[2].entry.author, "Bob");
        assert_eq!(lines[2].entry.email, "bob@example.com");
    }

    #[test]
    fn blame_short_ids_are_seven_chars() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        for bl in &lines {
            assert_eq!(
                bl.entry.short_id.len(),
                7,
                "short_id should be 7 chars, got {:?}",
                bl.entry.short_id
            );
        }
    }

    #[test]
    fn blame_at_commit_a_shows_only_two_lines() {
        let (_dir, path, oid_a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), Some(oid_a)).unwrap();
        // At commit A the file only had 2 lines
        assert_eq!(lines.len(), 2, "commit A had 2 lines, got {}", lines.len());
    }

    #[test]
    fn blame_at_commit_a_all_lines_attributed_to_alice() {
        let (_dir, path, oid_a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), Some(oid_a)).unwrap();
        for bl in &lines {
            assert_eq!(
                bl.entry.author, "Alice",
                "all lines at commit A should be Alice"
            );
        }
    }

    #[test]
    fn blame_oid_matches_commit() {
        let (_dir, path, _a, oid_b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        // line3 was added in commit B
        assert_eq!(lines[2].entry.oid, oid_b);
    }

    #[test]
    fn blame_time_is_nonzero() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let lines = compute_blame(&path, Path::new("code.txt"), None).unwrap();
        for bl in &lines {
            assert!(
                bl.entry.time.timestamp() > 0,
                "time should be nonzero for line {}",
                bl.line_no
            );
        }
    }

    #[test]
    fn blame_bad_repo_path_returns_error() {
        let result = compute_blame(Path::new("/nonexistent/path"), Path::new("code.txt"), None);
        assert!(result.is_err(), "expected error for bad repo path");
    }

    #[test]
    fn blame_missing_file_returns_error() {
        let (_dir, path, _a, _b) = make_blame_repo();
        let result = compute_blame(&path, Path::new("does_not_exist.txt"), None);
        assert!(result.is_err(), "expected error for nonexistent file");
    }
}
