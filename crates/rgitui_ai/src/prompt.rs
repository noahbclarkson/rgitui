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
/// re-sent every iteration, so the old cap could cost ~180k input tokens for
/// one commit message across three round trips.
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

const TOOL_PARAGRAPH: &str = "You have access to tools to get more context about the repository. Use them if you need to:\n\
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

/// Read the project-context files, if any exist. Blocking I/O — call it from a
/// background task.
pub(crate) fn collect_project_context(repo_path: &Path) -> Option<String> {
    let mut combined = String::new();

    for filename in PROJECT_CONTEXT_FILES {
        let file_path = repo_path.join(filename);
        if let Ok(contents) = std::fs::read_to_string(&file_path) {
            if !contents.trim().is_empty() {
                combined.push_str(&format!("=== {filename} ===\n{contents}\n\n"));
            }
        }
    }

    if combined.is_empty() {
        return None;
    }

    if combined.len() > MAX_PROJECT_CONTEXT_BYTES {
        let cut = {
            let safe = safe_truncate(&combined, MAX_PROJECT_CONTEXT_BYTES);
            safe.rfind('\n').unwrap_or(safe.len())
        };
        combined.truncate(cut);
        combined.push_str("\n\n[project context truncated]");
    }

    Some(combined)
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
        assert!(context.ends_with("[project context truncated]"));
        assert!(context.len() < MAX_PROJECT_CONTEXT_BYTES + 100);
    }
}
