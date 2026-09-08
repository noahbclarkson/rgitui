//! Prompt construction.
//!
//! The plain and tool-calling prompts used to duplicate the whole style match
//! and the whole truncation block verbatim; they now differ only by the one
//! paragraph that mentions tools.

use std::path::Path;

use crate::tools::safe_truncate;

/// Commit message style options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitStyle {
    /// Conventional Commits format: `feat(scope): description`.
    ///
    /// The default, matching `default_commit_style()` in settings. The two
    /// used to disagree, so an unrecognised string quietly produced a
    /// different style than a fresh install.
    #[default]
    Conventional,
    /// Plain English descriptive format.
    Descriptive,
    /// One-line brief format.
    Brief,
}

impl CommitStyle {
    pub const ALL: &'static [CommitStyle] = &[
        CommitStyle::Conventional,
        CommitStyle::Descriptive,
        CommitStyle::Brief,
    ];

    /// The persisted id.
    pub fn id(self) -> &'static str {
        match self {
            CommitStyle::Conventional => "conventional",
            CommitStyle::Descriptive => "descriptive",
            CommitStyle::Brief => "brief",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            CommitStyle::Conventional => "Conventional",
            CommitStyle::Descriptive => "Descriptive",
            CommitStyle::Brief => "Brief",
        }
    }

    /// A representative first line, so the three labels stop being guesses
    /// until the user has seen output.
    pub fn example(self) -> &'static str {
        match self {
            CommitStyle::Conventional => "feat(diff): add word-level intra-line highlighting",
            CommitStyle::Descriptive => "Add word-level highlighting inside changed diff lines",
            CommitStyle::Brief => "Highlight intra-line diff changes",
        }
    }

    /// Parse a persisted id. Unlike the old `FromStr` — whose `Infallible`
    /// error type made every caller's fallback branch unreachable — an
    /// unrecognised value is reported rather than silently becoming a style
    /// the user never chose.
    pub fn from_id(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|style| style.id() == normalized)
    }

    fn instruction(self) -> &'static str {
        match self {
            CommitStyle::Conventional => {
                "Use the Conventional Commits format: <type>(<scope>): <description>\n\
                 Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore\n\
                 Keep the first line under 72 characters.\n\
                 Add a blank line then a detailed body that explains what changed and why.\n\
                 List the key changes as bullet points if there are multiple distinct changes."
            }
            CommitStyle::Descriptive => {
                "Write a clear, descriptive commit message.\n\
                 First line: imperative mood summary under 72 characters.\n\
                 Add a blank line then a detailed body that explains what changed and why.\n\
                 List the key changes as bullet points if there are multiple distinct changes."
            }
            CommitStyle::Brief => {
                "Write a concise commit message in imperative mood.\n\
                 Keep it to a single line under 72 characters."
            }
        }
    }
}

impl std::str::FromStr for CommitStyle {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_id(s).ok_or(())
    }
}

/// Cap on the diff included in the prompt.
///
/// Lowered from 200 KB (~50-60k tokens). In the tool loop the whole prompt is
/// re-sent every iteration, so the old cap could cost tens of thousands of
/// input tokens per round trip for one commit message.
pub(crate) const MAX_DIFF_BYTES: usize = 40_000;

/// Truncate a diff at a line boundary, always leaving a marker so the model
/// knows it is reasoning about a partial change set.
pub(crate) fn truncate_diff(diff: &str, max_bytes: usize) -> String {
    if diff.len() <= max_bytes {
        return diff.to_string();
    }
    let truncated = safe_truncate(diff, max_bytes);
    let cut = truncated.rfind('\n').unwrap_or(truncated.len());
    format!(
        "{}\n\n[diff truncated -- showing {}/{} bytes]",
        &truncated[..cut],
        cut,
        diff.len()
    )
}

/// Cap on the changed-file list included in the prompt.
///
/// This was the one prompt input with no bound at all while the diff and the
/// project context both had one. A `git add .` that swept in a vendored tree
/// stages thousands of files, and one line each put the whole prompt past a
/// model's context window before the diff was even reached.
pub(crate) const MAX_SUMMARY_BYTES: usize = 8_000;

/// Truncate the changed-file list at a line boundary, saying how many files
/// are listed out of how many changed rather than stopping silently.
pub(crate) fn truncate_summary(summary: &str, max_bytes: usize) -> String {
    if summary.len() <= max_bytes {
        return summary.to_string();
    }
    let total = summary.lines().count();
    let truncated = safe_truncate(summary, max_bytes);
    let cut = truncated.rfind('\n').unwrap_or(truncated.len());
    let kept = &truncated[..cut];
    format!(
        "{}\n\n[showing {} of {} changed files]",
        kept,
        kept.lines().count(),
        total
    )
}

const TOOL_PARAGRAPH: &str ="You have access to tools to get more context about the repository. Use them if you need to:\n\
     - Understand what a changed file does (get_file_content)\n\
     - See the commit message style used in this project (get_recent_commits)\n\
     - Understand how a file has evolved (get_file_history)\n\n\
     Only use tools if the diff is unclear and you need more context. If the changes are self-explanatory, generate the commit message directly.\n\n";

/// Build the prompt. `with_tools` adds the paragraph describing the tools and
/// nothing else — everything else is shared, by construction.
pub(crate) fn build_prompt(
    diff: &str,
    summary: &str,
    commit_style: CommitStyle,
    project_context: Option<&str>,
    with_tools: bool,
) -> String {
    let style_instruction = commit_style.instruction();
    let diff_text = truncate_diff(diff, MAX_DIFF_BYTES);
    let summary = truncate_summary(summary, MAX_SUMMARY_BYTES);
    let context_section = match project_context {
        Some(context) => format!("Project Context:\n{context}\n\n"),
        None => String::new(),
    };
    let tool_section = if with_tools { TOOL_PARAGRAPH } else { "" };

    format!(
        "You are a Git commit message generator. Generate ONLY the commit message, nothing else.\n\
         No markdown formatting, no code blocks, no explanations.\n\n\
         {style_instruction}\n\n\
         {tool_section}\
         {context_section}\
         Files changed:\n{summary}\n\n\
         Diff:\n{diff_text}"
    )
}

pub(crate) const PROJECT_CONTEXT_FILES: &[&str] = &["README.md", "CLAUDE.md", "AGENTS.md"];
pub(crate) const MAX_PROJECT_CONTEXT_BYTES: usize = 50_000;

/// Appended to a context file that did not fit in the remaining budget, so
/// the model is told the file is partial rather than reading a sentence that
/// stops mid-word.
const TRUNCATION_MARKER: &str = "\n[project context truncated]";

/// Read the project-context files, if any exist. Blocking I/O — call it from a
/// background task.
///
/// Context injection is on by default, so this runs against whatever a freshly
/// cloned repository contains: every read is confined to the checkout and
/// bounded by the remaining budget before any bytes are taken.
pub(crate) fn collect_project_context(repo_path: &Path) -> Option<String> {
    let canonical_repo = repo_path.canonicalize().ok()?;
    let mut combined = String::new();

    for filename in PROJECT_CONTEXT_FILES {
        let header = format!("=== {filename} ===\n");
        // The header, the marker and the separator come out of the same
        // budget, so a file that fills it cannot push the total over.
        let overhead = header.len() + TRUNCATION_MARKER.len() + 2;
        let Some(remaining) = MAX_PROJECT_CONTEXT_BYTES
            .checked_sub(combined.len() + overhead)
            .filter(|remaining| *remaining > 0)
        else {
            break;
        };
        let Some((contents, truncated)) = read_context_file(&canonical_repo, filename, remaining)
        else {
            continue;
        };
        if contents.trim().is_empty() {
            continue;
        }
        combined.push_str(&header);
        combined.push_str(&contents);
        if truncated {
            combined.push_str(TRUNCATION_MARKER);
        }
        combined.push_str("\n\n");
    }

    (!combined.is_empty()).then_some(combined)
}

/// Read at most `limit` bytes of one project-context file, and only from
/// inside `canonical_repo`. Reports whether the file was cut short.
///
/// The canonical check is what stops a cloned repository shipping `README.md`
/// as a symlink to a credential file and having the first generated commit
/// message upload it — the same rule `get_file_content` already applies to
/// paths the model asks for. The limit is applied while reading rather than
/// after, so a huge file cannot be pulled into memory in full only to be
/// truncated.
fn read_context_file(
    canonical_repo: &Path,
    filename: &str,
    limit: usize,
) -> Option<(String, bool)> {
    use std::io::Read as _;

    let canonical_file = canonical_repo.join(filename).canonicalize().ok()?;
    if !canonical_file.starts_with(canonical_repo) || !canonical_file.is_file() {
        return None;
    }

    let file = std::fs::File::open(&canonical_file).ok()?;
    // A few bytes past the limit, so a character straddling the boundary still
    // has all of its bytes present and the overshoot reveals a longer file.
    let mut bytes = Vec::new();
    file.take(limit as u64 + 4).read_to_end(&mut bytes).ok()?;
    let truncated = bytes.len() > limit;

    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        // `error_len() == None` means the input ended mid-character, which is
        // this function's own doing. Genuinely invalid bytes are rejected, as
        // they are everywhere else the model is shown a file.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    Some((safe_truncate(text, limit).to_string(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── CommitStyle ───────────────────────────────────────────────

    #[test]
    fn commit_style_round_trips_through_its_id() {
        for style in CommitStyle::ALL {
            assert_eq!(CommitStyle::from_id(style.id()), Some(*style));
        }
    }

    /// The settings default was `"conventional"` while the enum default was
    /// `Descriptive`, so a typo silently produced a style the user never
    /// chose. The two now agree, and an unknown value is reported.
    #[test]
    fn the_enum_default_matches_the_settings_default() {
        assert_eq!(CommitStyle::default().id(), "conventional");
    }

    #[test]
    fn an_unknown_style_is_reported_rather_than_silently_substituted() {
        assert_eq!(
            CommitStyle::from_id("Conventional"),
            Some(CommitStyle::Conventional)
        );
        assert_eq!(CommitStyle::from_id("verbose"), None);
        assert_eq!("verbose".parse::<CommitStyle>(), Err(()));
    }

    #[test]
    fn every_style_has_a_distinct_instruction_and_example() {
        let mut instructions: Vec<&str> =
            CommitStyle::ALL.iter().map(|s| s.instruction()).collect();
        instructions.sort_unstable();
        instructions.dedup();
        assert_eq!(instructions.len(), CommitStyle::ALL.len());

        for style in CommitStyle::ALL {
            assert!(!style.example().is_empty());
            assert!(!style.display_name().is_empty());
        }
    }

    // ── truncation ────────────────────────────────────────────────

    #[test]
    fn a_diff_at_exactly_the_cap_is_not_truncated() {
        let diff = "a".repeat(MAX_DIFF_BYTES);
        let out = truncate_diff(&diff, MAX_DIFF_BYTES);
        assert_eq!(out, diff);
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn one_byte_over_the_cap_truncates_and_says_so() {
        let diff = format!("{}\nx", "a".repeat(MAX_DIFF_BYTES));
        let out = truncate_diff(&diff, MAX_DIFF_BYTES);
        assert!(out.contains("[diff truncated"));
        assert!(out.contains(&format!("/{} bytes]", diff.len())));
    }

    #[test]
    fn truncation_never_splits_a_multi_byte_character() {
        // A 3-byte character straddling the cut point used to be the panic
        // case for naive slicing.
        let mut diff = "a".repeat(MAX_DIFF_BYTES - 1);
        diff.push('☃');
        diff.push_str("tail");
        let out = truncate_diff(&diff, MAX_DIFF_BYTES);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn an_empty_diff_produces_an_empty_body_not_a_marker() {
        assert_eq!(truncate_diff("", MAX_DIFF_BYTES), "");
    }

    /// The changed-file list was the one prompt input with no bound at all.
    /// A `git add .` over a vendored tree stages thousands of files, and one
    /// line each pushed the prompt past a model's context window on its own.
    #[test]
    fn a_huge_changed_file_list_is_capped_and_says_how_many_it_shows() {
        let summary = (0..4_000)
            .map(|index| format!("M crates/rgitui_workspace/src/generated/file_{index:04}.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(summary.len() > MAX_SUMMARY_BYTES);

        let out = truncate_summary(&summary, MAX_SUMMARY_BYTES);
        assert!(out.len() < summary.len());
        assert!(out.contains("of 4000 changed files]"), "{out}");
    }

    #[test]
    fn a_short_changed_file_list_is_passed_through_untouched() {
        let summary = "M src/main.rs\nA src/lib.rs";
        assert_eq!(truncate_summary(summary, MAX_SUMMARY_BYTES), summary);
    }

    #[test]
    fn the_prompt_carries_the_capped_summary_not_the_raw_one() {
        let summary = "M a.rs\n".repeat(MAX_SUMMARY_BYTES);
        let prompt = build_prompt("diff", &summary, CommitStyle::Brief, None, false);
        assert!(prompt.len() < summary.len());
        assert!(prompt.contains("changed files]"));
    }

    // ── prompts ───────────────────────────────────────────────────

    #[test]
    fn the_tool_prompt_differs_from_the_plain_one_only_by_the_tool_paragraph() {
        let plain = build_prompt("D", "S", CommitStyle::Conventional, None, false);
        let with_tools = build_prompt("D", "S", CommitStyle::Conventional, None, true);
        assert_eq!(with_tools.replace(TOOL_PARAGRAPH, ""), plain);
    }

    #[test]
    fn the_style_instruction_reaches_the_prompt() {
        for style in CommitStyle::ALL {
            let prompt = build_prompt("D", "S", *style, None, false);
            assert!(prompt.contains(style.instruction()));
        }
    }

    #[test]
    fn project_context_is_included_only_when_present() {
        let without = build_prompt("D", "S", CommitStyle::Brief, None, false);
        assert!(!without.contains("Project Context:"));
        let with = build_prompt("D", "S", CommitStyle::Brief, Some("CTX"), false);
        assert!(with.contains("Project Context:\nCTX"));
    }

    #[test]
    fn an_oversize_diff_is_truncated_inside_the_prompt() {
        let diff = format!("{}\ntail", "a".repeat(MAX_DIFF_BYTES + 10));
        let prompt = build_prompt(&diff, "S", CommitStyle::Brief, None, false);
        assert!(prompt.contains("[diff truncated"));
        assert!(prompt.len() < diff.len() + 2_000);
    }

    // ── project context collection ────────────────────────────────

    #[test]
    fn no_context_files_yields_none() {
        let dir = TempDir::new().unwrap();
        assert!(collect_project_context(dir.path()).is_none());
    }

    #[test]
    fn one_context_file_is_wrapped_with_its_filename() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        let context = collect_project_context(dir.path()).unwrap();
        assert!(context.contains("=== README.md ==="));
        assert!(context.contains("hello"));
    }

    #[test]
    fn an_empty_context_file_is_skipped() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "   \n ").unwrap();
        assert!(collect_project_context(dir.path()).is_none());
    }

    #[test]
    fn every_context_file_is_collected_in_order() {
        let dir = TempDir::new().unwrap();
        for name in PROJECT_CONTEXT_FILES {
            std::fs::write(dir.path().join(name), format!("body of {name}")).unwrap();
        }
        let context = collect_project_context(dir.path()).unwrap();
        let readme = context.find("README.md").unwrap();
        let claude = context.find("CLAUDE.md").unwrap();
        assert!(readme < claude);
    }

    #[test]
    fn an_oversize_context_is_truncated_with_a_marker() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "x".repeat(MAX_PROJECT_CONTEXT_BYTES + 1_000),
        )
        .unwrap();
        let context = collect_project_context(dir.path()).unwrap();
        assert!(context.contains(TRUNCATION_MARKER));
        assert!(context.len() <= MAX_PROJECT_CONTEXT_BYTES);
    }

    /// Context injection is on by default, so a cloned repository could ship
    /// `README.md` as a symlink to a credential file and have the first
    /// generated commit message upload it.
    #[cfg(unix)]
    #[test]
    fn a_context_file_symlinked_outside_the_repo_is_not_read() {
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("credentials");
        std::fs::write(&secret, "AWS_SECRET_ACCESS_KEY=hunter2").unwrap();

        let dir = TempDir::new().unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("README.md")).unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "in-repo guidance").unwrap();

        let context = collect_project_context(dir.path()).unwrap();
        assert!(!context.contains("hunter2"));
        assert!(!context.contains("README.md"));
        assert!(context.contains("in-repo guidance"));
    }

    /// The budget used to be applied after every file had been read in full,
    /// so one huge file allocated its whole size before being thrown away.
    #[test]
    fn no_single_context_file_is_read_beyond_the_budget() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "x".repeat(MAX_PROJECT_CONTEXT_BYTES * 4),
        )
        .unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "y".repeat(1_000)).unwrap();

        let context = collect_project_context(dir.path()).unwrap();
        assert!(context.len() <= MAX_PROJECT_CONTEXT_BYTES);
        // The second file is skipped rather than read: the budget was already
        // spent by the first.
        assert!(!context.contains("CLAUDE.md"));
    }

    /// A multi-byte character straddling the read boundary must not make the
    /// whole file vanish.
    #[test]
    fn a_character_split_by_the_budget_does_not_discard_the_file() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "é".repeat(MAX_PROJECT_CONTEXT_BYTES),
        )
        .unwrap();
        let context = collect_project_context(dir.path()).unwrap();
        assert!(context.contains("README.md"));
        assert!(context.contains('é'));
    }
}
