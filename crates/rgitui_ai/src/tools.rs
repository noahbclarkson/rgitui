//! AI tool definitions and execution for commit message generation.
//!
//! Tools allow the AI model to request additional context about the repository
//! to generate more accurate commit messages.

use anyhow::Result;
use rgitui_git::git_command;
use std::path::Path;

/// Maximum number of commits to return for history tools.
const MAX_COMMITS: usize = 10;

/// Maximum file size to read (in bytes).
const MAX_FILE_SIZE: usize = 100_000;

/// Maximum diff size to return (in bytes).
const MAX_DIFF_SIZE: usize = 100_000;

/// Maximum directory depth for file tree.
const MAX_TREE_DEPTH: usize = 5;

/// Total tool output a single generation may accumulate.
///
/// The per-call caps above are not a budget: three iterations of `get_diff`
/// could add 300 KB on top of the base prompt. Once this is exhausted the
/// remaining calls are refused with a message the model can act on.
pub const MAX_TOOL_OUTPUT_BUDGET: usize = 200_000;

/// Filenames that must never be uploaded to a third-party API, regardless of
/// how the model asks for them.
///
/// The path-traversal check alone was not enough: `get_file_content("../.env")`
/// was correctly rejected, but `get_file_content(".env")` was accepted, and a
/// `DATABASE_URL=postgres://user:pass@…` went to the provider and was echoed
/// back into the conversation for every remaining iteration. There is no
/// consent step for that, so the only safe answer is not to read them.
const DENIED_FILE_NAMES: &[&str] = &[
    ".npmrc",
    ".netrc",
    "_netrc",
    ".pgpass",
    ".htpasswd",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];

/// Filename prefixes that are denied wherever they appear.
const DENIED_FILE_PREFIXES: &[&str] = &[".env", "credentials", "secrets", "id_rsa", "id_ed25519"];

/// Extensions that carry keys or certificates.
const DENIED_FILE_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "jks", "keystore", "asc"];

/// Why a path was refused. Each maps to a sentence the model can act on rather
/// than a bare I/O error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedReason {
    GitInternals,
    Credentials,
}

impl DeniedReason {
    fn message(self, path: &str) -> String {
        match self {
            DeniedReason::GitInternals => format!(
                "Refused: {} is inside .git/, which can contain remote URLs with embedded tokens.",
                path
            ),
            DeniedReason::Credentials => format!(
                "Refused: {} looks like a credentials file and is never sent to an AI provider.",
                path
            ),
        }
    }
}

/// Whether a repo-relative path is one the AI must never read.
///
/// Pure and case-insensitive, and it inspects every path component so a
/// denied file cannot be reached through a subdirectory.
pub fn denied_path(relative_path: &str) -> Option<DeniedReason> {
    let normalized = relative_path.replace('\\', "/");
    let components: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();

    if components
        .iter()
        .any(|part| part.eq_ignore_ascii_case(".git"))
    {
        return Some(DeniedReason::GitInternals);
    }

    let file_name = components.last()?.to_ascii_lowercase();

    if DENIED_FILE_NAMES.contains(&file_name.as_str()) {
        return Some(DeniedReason::Credentials);
    }
    if DENIED_FILE_PREFIXES
        .iter()
        .any(|prefix| file_name.starts_with(prefix))
    {
        return Some(DeniedReason::Credentials);
    }
    if let Some((_, extension)) = file_name.rsplit_once('.') {
        if DENIED_FILE_EXTENSIONS.contains(&extension) {
            return Some(DeniedReason::Credentials);
        }
    }

    None
}

/// Whether git ignores this path. A file the repository deliberately excludes
/// is not part of the change being described, and is the usual home for local
/// secrets that no denylist can enumerate.
fn is_git_ignored(repo_path: &Path, relative_path: &str) -> bool {
    git_command()
        .args(["check-ignore", "-q", "--no-index", "--"])
        .arg(relative_path)
        .current_dir(repo_path)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Truncate a string to at most `max` bytes without splitting a multi-byte
/// UTF-8 character. Returns a prefix whose length is the largest char boundary
/// at or below `max`, so slicing never panics on repo-controlled content.
pub(crate) fn safe_truncate(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Tool names used by the AI.
pub const TOOL_GET_FILE_CONTENT: &str = "get_file_content";
pub const TOOL_GET_RECENT_COMMITS: &str = "get_recent_commits";
pub const TOOL_GET_FILE_HISTORY: &str = "get_file_history";
pub const TOOL_GET_DIFF: &str = "get_diff";
pub const TOOL_GET_BRANCH_LIST: &str = "get_branch_list";
pub const TOOL_GET_FILE_TREE: &str = "get_file_tree";

/// Tool definitions for Anthropic's tool-calling API.
pub fn anthropic_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": TOOL_GET_FILE_CONTENT,
            "description": "Get the content of a file in the repository. Use this to understand the context of changes in specific files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file from the repository root"
                    }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": TOOL_GET_RECENT_COMMITS,
            "description": "Get recent commit messages from the repository. Use this to understand the commit message style and patterns used in this project.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of recent commits to retrieve (default: 5, max: 10)"
                    }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": TOOL_GET_FILE_HISTORY,
            "description": "Get the commit history for a specific file. Use this to understand how a file has evolved and what kinds of changes are typically made.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file from the repository root"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of commits to retrieve (default: 5, max: 10)"
                    }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": TOOL_GET_DIFF,
            "description": "Get the diff for staged changes, unstaged changes, or a specific commit. Use this to understand the exact changes made.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["staged", "unstaged", "commit"],
                        "description": "Type of diff: 'staged' for staged changes, 'unstaged' for unstaged changes, 'commit' for a specific commit"
                    },
                    "commit": {
                        "type": "string",
                        "description": "Commit SHA (required when kind='commit', can be short SHA)"
                    }
                },
                "required": ["kind"]
            }
        }),
        serde_json::json!({
            "name": TOOL_GET_BRANCH_LIST,
            "description": "Get a list of all branches in the repository. Use this to understand the branching structure.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "include_remote": {
                        "type": "boolean",
                        "description": "Include remote branches (default: true)"
                    }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": TOOL_GET_FILE_TREE,
            "description": "Get the file tree structure of the repository. Use this to understand the project layout.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to start from (default: root)"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum depth to traverse (default: 3, max: 5)"
                    }
                },
                "required": []
            }
        }),
    ]
}

/// Tool definitions for OpenAI's function calling API.
pub fn openai_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_FILE_CONTENT,
                "description": "Get the content of a file in the repository. Use this to understand the context of changes in specific files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the file from the repository root"
                        }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_RECENT_COMMITS,
                "description": "Get recent commit messages from the repository. Use this to understand the commit message style and patterns used in this project.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "count": {
                            "type": "integer",
                            "description": "Number of recent commits to retrieve (default: 5, max: 10)"
                        }
                    },
                    "required": []
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_FILE_HISTORY,
                "description": "Get the commit history for a specific file. Use this to understand how a file has evolved and what kinds of changes are typically made.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the file from the repository root"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of commits to retrieve (default: 5, max: 10)"
                        }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_DIFF,
                "description": "Get the diff for staged changes, unstaged changes, or a specific commit. Use this to understand the exact changes made.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["staged", "unstaged", "commit"],
                            "description": "Type of diff: 'staged' for staged changes, 'unstaged' for unstaged changes, 'commit' for a specific commit"
                        },
                        "commit": {
                            "type": "string",
                            "description": "Commit SHA (required when kind='commit', can be short SHA)"
                        }
                    },
                    "required": ["kind"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_BRANCH_LIST,
                "description": "Get a list of all branches in the repository. Use this to understand the branching structure.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "include_remote": {
                            "type": "boolean",
                            "description": "Include remote branches (default: true)"
                        }
                    },
                    "required": []
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": TOOL_GET_FILE_TREE,
                "description": "Get the file tree structure of the repository. Use this to understand the project layout.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to start from (default: root)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum depth to traverse (default: 3, max: 5)"
                        }
                    },
                    "required": []
                }
            }
        }),
    ]
}

/// Tool definitions for Gemini's function calling API.
pub fn gemini_tool_definitions() -> serde_json::Value {
    serde_json::json!({
        "function_declarations": [
            {
                "name": TOOL_GET_FILE_CONTENT,
                "description": "Get the content of a file in the repository. Use this to understand the context of changes in specific files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the file from the repository root"
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": TOOL_GET_RECENT_COMMITS,
                "description": "Get recent commit messages from the repository. Use this to understand the commit message style and patterns used in this project.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "count": {
                            "type": "integer",
                            "description": "Number of recent commits to retrieve (default: 5, max: 10)"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": TOOL_GET_FILE_HISTORY,
                "description": "Get the commit history for a specific file. Use this to understand how a file has evolved and what kinds of changes are typically made.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the file from the repository root"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of commits to retrieve (default: 5, max: 10)"
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": TOOL_GET_DIFF,
                "description": "Get the diff for staged changes, unstaged changes, or a specific commit. Use this to understand the exact changes made.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["staged", "unstaged", "commit"],
                            "description": "Type of diff: 'staged' for staged changes, 'unstaged' for unstaged changes, 'commit' for a specific commit"
                        },
                        "commit": {
                            "type": "string",
                            "description": "Commit SHA (required when kind='commit', can be short SHA)"
                        }
                    },
                    "required": ["kind"]
                }
            },
            {
                "name": TOOL_GET_BRANCH_LIST,
                "description": "Get a list of all branches in the repository. Use this to understand the branching structure.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "include_remote": {
                            "type": "boolean",
                            "description": "Include remote branches (default: true)"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": TOOL_GET_FILE_TREE,
                "description": "Get the file tree structure of the repository. Use this to understand the project layout.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to start from (default: root)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum depth to traverse (default: 3, max: 5)"
                        }
                    },
                    "required": []
                }
            }
        ]
    })
}

/// A tool call from the AI model.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub result: Result<String, String>,
}

/// Tracks how much tool output a single generation has accumulated, so the
/// per-call caps add up to a bounded whole.
#[derive(Debug, Default)]
pub struct ToolBudget {
    used: usize,
}

impl ToolBudget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn remaining(&self) -> usize {
        MAX_TOOL_OUTPUT_BUDGET.saturating_sub(self.used)
    }

    /// Charge `output` against the budget, trimming it to what is left. The
    /// model is told when a result was cut short so it does not treat a
    /// truncated listing as complete.
    pub fn charge(&mut self, output: String) -> String {
        let remaining = self.remaining();
        if remaining == 0 {
            return "Tool output budget exhausted for this generation. Answer with what you \
                    already have."
                .to_string();
        }
        if output.len() <= remaining {
            self.used += output.len();
            return output;
        }
        self.used = MAX_TOOL_OUTPUT_BUDGET;
        let trimmed = safe_truncate(&output, remaining);
        let cut = trimmed.rfind('\n').unwrap_or(trimmed.len());
        format!(
            "{}\n\n[tool output truncated -- the budget for this generation is spent]",
            &trimmed[..cut]
        )
    }
}

/// Execute a tool call and return the result.
pub fn execute_tool(call: &ToolCall, repo_path: &Path) -> ToolResult {
    execute_tool_within(call, repo_path, &mut ToolBudget::new())
}

/// Execute a tool call, charging its output against a per-generation budget.
pub fn execute_tool_within(
    call: &ToolCall,
    repo_path: &Path,
    budget: &mut ToolBudget,
) -> ToolResult {
    let result = match execute_tool_uncharged(call, repo_path) {
        Ok(output) => Ok(budget.charge(output)),
        Err(error) => Err(error),
    };

    ToolResult {
        call_id: call.id.clone(),
        result,
    }
}

fn execute_tool_uncharged(call: &ToolCall, repo_path: &Path) -> Result<String, String> {
    match call.name.as_str() {
        TOOL_GET_FILE_CONTENT => {
            let path = call.arguments["path"].as_str().unwrap_or("");
            execute_get_file_content(repo_path, path)
        }
        TOOL_GET_RECENT_COMMITS => {
            let count = call.arguments["count"].as_u64().unwrap_or(5) as usize;
            execute_get_recent_commits(repo_path, count.min(MAX_COMMITS))
        }
        TOOL_GET_FILE_HISTORY => {
            let path = call.arguments["path"].as_str().unwrap_or("");
            let count = call.arguments["count"].as_u64().unwrap_or(5) as usize;
            execute_get_file_history(repo_path, path, count.min(MAX_COMMITS))
        }
        TOOL_GET_DIFF => {
            let kind = call.arguments["kind"].as_str().unwrap_or("staged");
            let commit = call.arguments["commit"].as_str().unwrap_or("");
            execute_get_diff(repo_path, kind, commit)
        }
        TOOL_GET_BRANCH_LIST => {
            let include_remote = call.arguments["include_remote"].as_bool().unwrap_or(true);
            execute_get_branch_list(repo_path, include_remote)
        }
        TOOL_GET_FILE_TREE => {
            let path = call.arguments["path"].as_str().unwrap_or("");
            let max_depth = call.arguments["max_depth"].as_u64().unwrap_or(3) as usize;
            execute_get_file_tree(repo_path, path, max_depth.min(MAX_TREE_DEPTH))
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

/// Get the content of a file in the repository.
///
/// Denial happens before the read, in order: git internals, then the
/// credentials denylist, then git-ignored files, then the traversal check.
/// Only after all four does anything touch the file.
fn execute_get_file_content(repo_path: &Path, relative_path: &str) -> Result<String, String> {
    if let Some(reason) = denied_path(relative_path) {
        return Err(reason.message(relative_path));
    }
    if is_git_ignored(repo_path, relative_path) {
        return Err(format!(
            "Refused: {} is git-ignored, so it is not part of the change and may hold local secrets.",
            relative_path
        ));
    }

    let file_path = repo_path.join(relative_path);

    // Security check: ensure path is within repo
    let canonical_repo = repo_path
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize repo path: {}", e))?;
    let canonical_file = file_path
        .canonicalize()
        .map_err(|_e| format!("File not found: {}", relative_path))?;

    if !canonical_file.starts_with(&canonical_repo) {
        return Err(format!("Path outside repository: {}", relative_path));
    }

    // The canonical path is what is actually read, so re-check it: a symlink
    // inside the repo that resolves onto `.git/config` would otherwise pass
    // the name check above.
    if let Ok(resolved) = canonical_file.strip_prefix(&canonical_repo) {
        if let Some(reason) = denied_path(&resolved.to_string_lossy()) {
            return Err(reason.message(relative_path));
        }
    }

    // Check file size
    let metadata = std::fs::metadata(&canonical_file)
        .map_err(|e| format!("Failed to read file metadata: {}", e))?;

    if metadata.len() as usize > MAX_FILE_SIZE {
        return Err(format!(
            "File too large ({} bytes, max {})",
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }

    let bytes =
        std::fs::read(&canonical_file).map_err(|e| format!("Failed to read file: {}", e))?;

    // Say "not text" explicitly rather than surfacing a raw `read_to_string`
    // encoding error, which reads like a bug in rgitui.
    String::from_utf8(bytes).map_err(|_| format!("{} is not a UTF-8 text file.", relative_path))
}

/// Get recent commit messages from the repository.
fn execute_get_recent_commits(repo_path: &Path, count: usize) -> Result<String, String> {
    let output = git_command()
        .args([
            "log",
            &format!("-{}", count),
            "--pretty=format:%h %s",
            "--no-merges",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git log: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.into_owned())
}

/// Get commit history for a specific file.
fn execute_get_file_history(
    repo_path: &Path,
    relative_path: &str,
    count: usize,
) -> Result<String, String> {
    let output = git_command()
        .args([
            "log",
            &format!("-{}", count),
            "--pretty=format:%h %s (%cr)",
            "--no-merges",
            "--",
            relative_path,
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git log: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.is_empty() {
        return Ok(format!("No commits found for file: {}", relative_path));
    }

    Ok(stdout.into_owned())
}

/// Get diff for staged, unstaged, or a specific commit.
fn execute_get_diff(repo_path: &Path, kind: &str, commit: &str) -> Result<String, String> {
    let args = match kind {
        "staged" => vec!["diff", "--cached"],
        "unstaged" => vec!["diff"],
        "commit" => {
            if commit.is_empty() {
                return Err("Commit SHA required when kind='commit'".to_string());
            }
            if commit.starts_with('-') {
                return Err(format!("Invalid commit ref: {}", commit));
            }
            // `--end-of-options` stops git from interpreting the revision as an
            // option flag while still treating it as a revision (not a pathspec,
            // which is what a bare `--` would force).
            vec!["show", "--end-of-options", commit]
        }
        _ => {
            return Err(format!(
                "Invalid diff kind: {}. Use 'staged', 'unstaged', or 'commit'",
                kind
            ))
        }
    };

    let output = git_command()
        .args(&args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git diff: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.is_empty() {
        return Ok(format!("No {} changes", kind));
    }

    // Truncate if too large
    if stdout.len() > MAX_DIFF_SIZE {
        let truncated = safe_truncate(&stdout, MAX_DIFF_SIZE);
        let last_newline = truncated.rfind('\n').unwrap_or(truncated.len());
        return Ok(format!(
            "{}\n\n[diff truncated -- showing {}/{} bytes]",
            &truncated[..last_newline],
            last_newline,
            stdout.len()
        ));
    }

    Ok(stdout.into_owned())
}

/// Get list of branches.
fn execute_get_branch_list(repo_path: &Path, include_remote: bool) -> Result<String, String> {
    let mut args = vec!["branch"];

    if include_remote {
        args.push("-a");
    }

    let output = git_command()
        .args(&args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git branch: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git branch failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.is_empty() {
        return Ok("No branches found".to_string());
    }

    // Clean up output: remove leading whitespace and markers
    let branches: Vec<String> = stdout
        .lines()
        .map(|line| {
            let line = line.trim_start();
            // Remove current branch marker and worktree markers
            line.strip_prefix("* ")
                .or_else(|| line.strip_prefix("+ "))
                .unwrap_or(line)
                .to_string()
        })
        .collect();

    Ok(branches.join("\n"))
}

/// Get file tree structure.
fn execute_get_file_tree(
    repo_path: &Path,
    relative_path: &str,
    max_depth: usize,
) -> Result<String, String> {
    let base_path = if relative_path.is_empty() {
        repo_path.to_path_buf()
    } else {
        repo_path.join(relative_path)
    };

    // Security check: ensure path is within repo
    let canonical_repo = repo_path
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize repo path: {}", e))?;

    let canonical_base = base_path
        .canonicalize()
        .map_err(|_| format!("Path not found: {}", relative_path))?;

    if !canonical_base.starts_with(&canonical_repo) {
        return Err(format!("Path outside repository: {}", relative_path));
    }

    fn build_tree(path: &Path, prefix: String, current_depth: usize, max_depth: usize) -> String {
        if current_depth > max_depth {
            return format!("{}...\n", prefix);
        }

        let mut result = String::new();

        let Ok(read_dir) = std::fs::read_dir(path) else {
            return result;
        };
        let entries: Vec<_> = read_dir
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                // Skip hidden files and common ignore patterns
                !name.starts_with('.')
                    && name != "target"
                    && name != "node_modules"
                    && name != "__pycache__"
            })
            .collect();

        let mut entries = entries;
        entries.sort_by_key(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            // Directories first, then alphabetically
            (!is_dir, e.file_name())
        });

        for (i, entry) in entries.iter().enumerate() {
            let is_last = i == entries.len() - 1;
            let name = entry.file_name().to_string_lossy().to_string();
            let connector = if is_last { "└── " } else { "├── " };
            let extension = if is_last { "    " } else { "│   " };

            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

            if is_dir {
                result.push_str(&format!("{}{}{}/\n", prefix, connector, name));
                if current_depth < max_depth {
                    result.push_str(&build_tree(
                        &entry.path(),
                        format!("{}{}", prefix, extension),
                        current_depth + 1,
                        max_depth,
                    ));
                }
            } else {
                result.push_str(&format!("{}{}{}\n", prefix, connector, name));
            }
        }

        result
    }

    Ok(build_tree(&canonical_base, String::new(), 0, max_depth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── tool definitions ──────────────────────────────────────────

    #[test]
    fn test_tool_definitions_not_empty() {
        assert!(!anthropic_tool_definitions().is_empty());
        assert!(!openai_tool_definitions().is_empty());
        assert!(!gemini_tool_definitions()["function_declarations"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn all_providers_expose_same_tool_names() {
        let anthropic_defs = anthropic_tool_definitions();
        let anthropic: Vec<&str> = anthropic_defs
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let openai_defs = openai_tool_definitions();
        let openai: Vec<&str> = openai_defs
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        let gemini_arr = gemini_tool_definitions();
        let gemini: Vec<&str> = gemini_arr["function_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();

        assert_eq!(anthropic.len(), openai.len());
        assert_eq!(anthropic.len(), gemini.len());
        for name in &anthropic {
            assert!(openai.contains(name), "OpenAI missing tool: {}", name);
            assert!(gemini.contains(name), "Gemini missing tool: {}", name);
        }
    }

    // ── execute_get_file_content ──────────────────────────────────

    fn make_repo_with_file(content: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, content).unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn file_content_reads_existing_file() {
        let (_dir, repo) = make_repo_with_file("hello world");
        let result = execute_get_file_content(&repo, "hello.txt");
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn file_content_err_on_missing_file() {
        let (_dir, repo) = make_repo_with_file("x");
        let result = execute_get_file_content(&repo, "does_not_exist.txt");
        assert!(result.is_err());
    }

    // ── the H4 sandbox ────────────────────────────────────────────

    /// The traversal check alone was not enough. `../.env` was correctly
    /// rejected while a plain `.env` was read and uploaded to the provider,
    /// with no consent step and nothing in the UI but "Reading .env".
    #[test]
    fn credentials_files_are_denied_by_name() {
        for path in [
            ".env",
            ".env.local",
            ".env.production",
            "app/.env",
            "config/credentials.yml",
            "secrets.yaml",
            "deploy/id_rsa",
            "certs/server.pem",
            "certs/server.key",
            "keys/bundle.p12",
            ".npmrc",
            ".netrc",
        ] {
            assert_eq!(
                denied_path(path),
                Some(DeniedReason::Credentials),
                "{path} was not denied"
            );
        }
    }

    #[test]
    fn git_internals_are_denied_at_any_depth_and_in_any_case() {
        for path in [
            ".git/config",
            ".git/hooks/pre-commit",
            "sub/.git/config",
            ".GIT/config",
            r".git\config",
        ] {
            assert_eq!(
                denied_path(path),
                Some(DeniedReason::GitInternals),
                "{path} was not denied"
            );
        }
    }

    #[test]
    fn ordinary_source_files_are_not_denied() {
        for path in [
            "src/main.rs",
            "crates/rgitui_ai/src/lib.rs",
            "README.md",
            "environment.rs",
            "docs/keyboard.md",
            "src/env_utils.rs",
        ] {
            assert_eq!(denied_path(path), None, "{path} was wrongly denied");
        }
    }

    #[test]
    fn a_denied_file_is_refused_before_it_is_ever_read() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".env"),
            "DATABASE_URL=postgres://user:pass@host/db",
        )
        .unwrap();
        let result = execute_get_file_content(dir.path(), ".env");
        let error = result.unwrap_err();
        assert!(error.contains("Refused"));
        // The refusal must not itself leak the content it refused to read.
        assert!(!error.contains("postgres://"));
    }

    #[test]
    fn non_utf8_content_is_reported_as_such_rather_than_as_an_io_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let error = execute_get_file_content(dir.path(), "blob.bin").unwrap_err();
        assert!(error.contains("not a UTF-8 text file"));
    }

    // ── tool output budget ────────────────────────────────────────

    #[test]
    fn the_budget_passes_small_outputs_through_untouched() {
        let mut budget = ToolBudget::new();
        assert_eq!(budget.charge("hello".into()), "hello");
        assert_eq!(budget.remaining(), MAX_TOOL_OUTPUT_BUDGET - 5);
    }

    #[test]
    fn the_budget_truncates_once_it_is_spent_and_says_so() {
        let mut budget = ToolBudget::new();
        let big = format!("{}\ntail", "x".repeat(MAX_TOOL_OUTPUT_BUDGET));
        let charged = budget.charge(big);
        assert!(charged.contains("[tool output truncated"));
        assert_eq!(budget.remaining(), 0);

        // Three iterations of `get_diff` can no longer add 300 KB on top of
        // the base prompt.
        let next = budget.charge("more output".into());
        assert!(next.contains("budget exhausted"));
    }

    #[test]
    fn a_charged_execution_reports_the_call_id_it_was_given() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "body").unwrap();
        let call = ToolCall {
            id: "call-1".into(),
            name: TOOL_GET_FILE_CONTENT.into(),
            arguments: serde_json::json!({ "path": "a.txt" }),
        };
        let mut budget = ToolBudget::new();
        let result = execute_tool_within(&call, dir.path(), &mut budget);
        assert_eq!(result.call_id, "call-1");
        assert_eq!(result.result.unwrap(), "body");
    }

    #[test]
    fn file_content_rejects_path_traversal() {
        let (_dir, repo) = make_repo_with_file("x");
        // Create a file outside the repo dir so canonicalize works
        let outer = TempDir::new().unwrap();
        let outer_file = outer.path().join("secret.txt");
        fs::write(&outer_file, "secret").unwrap();
        let traversal = format!("../../{}", outer_file.display());
        let result = execute_get_file_content(&repo, &traversal);
        assert!(result.is_err());
    }

    // ── execute_get_file_tree ─────────────────────────────────────

    #[test]
    fn file_tree_lists_files_at_root() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        fs::write(dir.path().join("b.rs"), "").unwrap();
        let result = execute_get_file_tree(dir.path(), "", 1).unwrap();
        assert!(result.contains("a.txt"));
        assert!(result.contains("b.rs"));
    }

    #[test]
    fn file_tree_respects_max_depth() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let subsub = sub.join("deep");
        fs::create_dir(&subsub).unwrap();
        fs::write(subsub.join("deep.txt"), "").unwrap();
        // max_depth=0: should show sub/ but not its contents
        let result = execute_get_file_tree(dir.path(), "", 0).unwrap();
        assert!(result.contains("sub/"));
        assert!(!result.contains("deep.txt"));
    }

    #[test]
    fn file_tree_skips_hidden_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".hidden"), "").unwrap();
        fs::write(dir.path().join("visible.txt"), "").unwrap();
        let result = execute_get_file_tree(dir.path(), "", 1).unwrap();
        assert!(!result.contains(".hidden"));
        assert!(result.contains("visible.txt"));
    }

    #[test]
    fn file_tree_skips_target_directory() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("artifact"), "").unwrap();
        fs::write(dir.path().join("src.rs"), "").unwrap();
        let result = execute_get_file_tree(dir.path(), "", 2).unwrap();
        assert!(!result.contains("artifact"));
        assert!(result.contains("src.rs"));
    }

    #[test]
    fn file_tree_rejects_path_outside_repo() {
        let dir = TempDir::new().unwrap();
        let result = execute_get_file_tree(dir.path(), "../..", 1);
        assert!(result.is_err());
    }

    // ── execute_tool dispatch ─────────────────────────────────────

    #[test]
    fn execute_tool_unknown_returns_err() {
        let dir = TempDir::new().unwrap();
        let call = ToolCall {
            id: "1".into(),
            name: "nonexistent_tool".into(),
            arguments: serde_json::json!({}),
        };
        let result = execute_tool(&call, dir.path());
        assert!(result.result.is_err());
        assert!(result.result.unwrap_err().contains("Unknown tool"));
    }

    #[test]
    fn execute_tool_get_file_content_roundtrip() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("notes.txt"), "test content").unwrap();
        let call = ToolCall {
            id: "42".into(),
            name: TOOL_GET_FILE_CONTENT.into(),
            arguments: serde_json::json!({ "path": "notes.txt" }),
        };
        let result = execute_tool(&call, dir.path());
        assert_eq!(result.call_id, "42");
        assert_eq!(result.result.unwrap(), "test content");
    }

    #[test]
    fn execute_tool_get_file_tree_roundtrip() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let call = ToolCall {
            id: "7".into(),
            name: TOOL_GET_FILE_TREE.into(),
            arguments: serde_json::json!({ "path": "", "max_depth": 1 }),
        };
        let result = execute_tool(&call, dir.path());
        assert!(result.result.is_ok());
        assert!(result.result.unwrap().contains("main.rs"));
    }

    // ── build_tree: unreadable dir doesn't panic ──────────────────

    #[test]
    fn file_tree_on_nonexistent_subpath_returns_err() {
        let dir = TempDir::new().unwrap();
        let result = execute_get_file_tree(dir.path(), "no_such_dir", 1);
        assert!(result.is_err());
    }
}
