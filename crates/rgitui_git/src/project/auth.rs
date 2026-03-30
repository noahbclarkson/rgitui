use anyhow::{Context as _, Result};
use std::path::Path;
use std::process::Command;

/// Run a git network command via system git.
///
/// System git handles all authentication (SSH agent, credential managers,
/// etc.) natively, providing reliable support across platforms and providers.
pub(crate) fn run_git_network_command(repo_path: &Path, args: &[&str]) -> Result<Option<String>> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .output()
        .with_context(|| format!("Failed to run system git command: git {}", args.join(" ")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stderr}\n{stdout}")
    };

    if output.status.success() {
        let detail = detail.trim().to_string();
        Ok((!detail.is_empty()).then_some(detail))
    } else {
        anyhow::bail!(
            "{}",
            if detail.is_empty() {
                format!(
                    "git {} failed with exit status {}",
                    args.join(" "),
                    output.status
                )
            } else {
                detail
            }
        );
    }
}
