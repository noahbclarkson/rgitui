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
