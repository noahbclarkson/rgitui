use anyhow::Result;
use chrono::{TimeZone, Utc};
use git2::Repository;
use std::path::Path;

/// A single reflog entry.
#[derive(Debug, Clone)]
pub struct ReflogEntryInfo {
    /// The new OID after the operation.
    pub new_oid: git2::Oid,
    /// The old OID before the operation (may be zero for initial entries).
    pub old_oid: git2::Oid,
    /// Short form of new OID.
    pub new_short_id: String,
    /// Short form of old OID (empty string if zero).
    pub old_short_id: String,
    /// Operation message (e.g., "commit: Add feature").
    pub message: Option<String>,
    /// Who performed the operation.
    pub committer: String,
    /// When the operation occurred.
    pub time: chrono::DateTime<Utc>,
}

/// Maximum number of reflog entries to return.
const MAX_REFLOG_ENTRIES: usize = 200;

/// Get reflog entries for a reference (typically "HEAD").
pub fn compute_reflog(repo_path: &Path, ref_name: &str) -> Result<Vec<ReflogEntryInfo>> {
    let repo = Repository::open(repo_path)?;
    let reflog = repo.reflog(ref_name)?;

    let mut entries = Vec::new();
    for entry in reflog.iter() {
        if entries.len() >= MAX_REFLOG_ENTRIES {
            break;
        }

        let new_oid = entry.id_new();
        let old_oid = entry.id_old();

        // Check for zero OID (initial entry or special cases)
        let zero_oid = git2::Oid::zero();
        let old_short_id = if old_oid == zero_oid {
            String::new()
        } else {
            format!("{:.7}", old_oid)
        };

        let committer_sig = entry.committer();
        let time = Utc
            .timestamp_opt(committer_sig.when().seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        entries.push(ReflogEntryInfo {
            new_oid,
            old_oid,
            new_short_id: format!("{:.7}", new_oid),
            old_short_id,
            message: entry.message().map(str::to_string),
            committer: committer_sig.name().unwrap_or("unknown").to_string(),
            time,
        });
    }

    // Reflog entries are returned newest-first, which is what we want
    Ok(entries)
}

use gpui::{AsyncApp, Context, Task, WeakEntity};

use super::GitProject;

impl GitProject {
    /// Get reflog entries for a reference.
    pub fn reflog(&self, ref_name: &str) -> Result<Vec<ReflogEntryInfo>> {
        compute_reflog(self.repo_path(), ref_name)
    }

    /// Get reflog entries asynchronously.
    pub fn reflog_async(
        &self,
        ref_name: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<ReflogEntryInfo>>> {
        let repo_path = self.repo_path().to_path_buf();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .spawn(async move { compute_reflog(&repo_path, &ref_name) })
                .await
        })
    }
}
