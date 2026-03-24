//! AI tool definitions and execution for commit message generation.
//!
//! Tools allow the AI model to request additional context about the repository
//! to generate more accurate commit messages.

use anyhow::Result;
use std::path::Path;

/// Maximum number of commits to return for history tools.
const MAX_COMMITS: usize = 10;

/// Maximum file size to read (in bytes).
const MAX_FILE_SIZE: usize = 100_000;

/// Maximum diff size to return (in bytes).
const MAX_DIFF_SIZE: usize = 100_000;

/// Maximum directory depth for file tree.
const MAX_TREE_DEPTH: usize = 5;

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
    pub name: String,
    pub result: Result<String, String>,
}

/// Execute a tool call and return the result.
pub fn execute_tool(call: &ToolCall, repo_path: &Path) -> ToolResult {
    let result = match call.name.as_str() {
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
    };

    ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        result,
    }
}

/// Get the content of a file in the repository.
fn execute_get_file_content(repo_path: &Path, relative_path: &str) -> Result<String, String> {
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

    // Read and return content
    let content = std::fs::read_to_string(&canonical_file)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    Ok(content)
}

/// Get recent commit messages from the repository.
fn execute_get_recent_commits(repo_path: &Path, count: usize) -> Result<String, String> {
    let output = std::process::Command::new("git")
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
    let output = std::process::Command::new("git")
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
            vec!["show", commit]
        }
        _ => {
            return Err(format!(
                "Invalid diff kind: {}. Use 'staged', 'unstaged', or 'commit'",
                kind
            ))
        }
    };

    let output = std::process::Command::new("git")
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
        let truncated = &stdout[..MAX_DIFF_SIZE];
        let last_newline = truncated.rfind('\n').unwrap_or(MAX_DIFF_SIZE);
        return Ok(format!(
            "{}\n\n[diff truncated — showing {}/{} bytes]",
            &stdout[..last_newline],
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

    let output = std::process::Command::new("git")
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
        assert_eq!(result.name, TOOL_GET_FILE_CONTENT);
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
