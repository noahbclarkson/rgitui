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

/// Tool names used by the AI.
pub const TOOL_GET_FILE_CONTENT: &str = "get_file_content";
pub const TOOL_GET_RECENT_COMMITS: &str = "get_recent_commits";
pub const TOOL_GET_FILE_HISTORY: &str = "get_file_history";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definitions_not_empty() {
        assert!(!anthropic_tool_definitions().is_empty());
        assert!(!openai_tool_definitions().is_empty());
        assert!(!gemini_tool_definitions()["function_declarations"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
