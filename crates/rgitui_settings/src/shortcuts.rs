use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A command identifier as stored in shortcut override keys.
/// Matches the `CommandId::as_str()` variant names from command_palette.rs.
pub const COMMAND_IDS: &[&str] = &[
    "fetch",
    "pull",
    "push",
    "push_all",
    "pull_all",
    "force_push",
    "commit",
    "stage_all",
    "unstage_all",
    "stash_save",
    "stash_pop",
    "stash_apply",
    "stash_drop",
    "create_branch",
    "delete_branch",
    "rename_branch",
    "merge_branch",
    "create_tag",
    "create_worktree",
    "create_pr",
    "cherry_pick",
    "revert_commit",
    "interactive_rebase",
    "discard_all",
    "clean_untracked",
    "reset_hard",
    "abort_operation",
    "continue_merge",
    "toggle_diff_mode",
    "search",
    "ai_message",
    "refresh",
    "settings",
    "open_repo",
    "workspace_home",
    "restore_last_workspace",
    "shortcuts",
    "switch_branch",
    "blame",
    "undo",
    "file_history",
    "reflog",
    "submodules",
    "bisect",
    "bisect_start",
    "bisect_good",
    "bisect_bad",
    "bisect_reset",
    "bisect_skip",
    "global_search",
    "toggle_issues",
    "toggle_pull_requests",
    "toggle_branch_health",
    "toggle_stashes",
    "stash_branch",
    "open_theme_editor",
];

/// Parsed keyboard shortcut — extracted modifier flags and key label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedShortcut {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub platform: bool,
    pub key: String,
}

impl std::fmt::Display for ParsedShortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.platform {
            parts.push("Cmd");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

impl ParsedShortcut {
    /// Parse a shortcut string like "Ctrl+Shift+R" or "Alt+5" into a `ParsedShortcut`.
    /// Returns `None` for empty strings or strings with no valid key component.
    pub fn parse(s: &str) -> Option<ParsedShortcut> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let tokens: Vec<&str> = s.split('+').map(|t| t.trim()).collect();
        if tokens.is_empty() {
            return None;
        }
        let key = tokens.last()?.to_string();
        if key.is_empty() {
            return None;
        }
        let mut shortcut = ParsedShortcut {
            control: false,
            shift: false,
            alt: false,
            platform: false,
            key,
        };
        for token in tokens.iter().take(tokens.len().saturating_sub(1)) {
            match token.to_lowercase().as_str() {
                "ctrl" | "control" => shortcut.control = true,
                "shift" => shortcut.shift = true,
                "alt" | "option" => shortcut.alt = true,
                "cmd" | "command" | "super" | "meta" => shortcut.platform = true,
                _ => {}
            }
        }
        Some(shortcut)
    }

    /// Returns true if this shortcut has no modifiers (key only, e.g. "j", "/", "?").
    pub fn is_key_only(&self) -> bool {
        !self.control && !self.shift && !self.alt && !self.platform
    }
}

/// A keyboard shortcut override for a specific command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CustomShortcuts {
    /// Command shortcut overrides. Keys are `CommandId::as_str()` names (e.g.
    /// `"toggle_stashes"`), values are shortcut strings (e.g. `"Alt+8"`).
    #[serde(default)]
    pub overrides: HashMap<String, String>,
}

impl CustomShortcuts {
    /// Returns the shortcut string for the given command, or `None` if no custom
    /// override is configured.
    pub fn shortcut_for(&self, command: &str) -> Option<&str> {
        self.overrides.get(command).map(|s| s.as_str())
    }

    /// Returns the parsed custom shortcut for `command`, or `None` if no override
    /// is set or the stored string is unparseable.
    pub fn parsed_shortcut_for(&self, command: &str) -> Option<ParsedShortcut> {
        self.shortcut_for(command).and_then(ParsedShortcut::parse)
    }

    /// Returns `true` if any command has a custom shortcut override.
    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Validates that all override keys are recognized `CommandId` names.
    /// Returns a list of unknown command names found.
    pub fn validate(&self) -> Vec<&str> {
        self.overrides
            .keys()
            .filter(|k| !COMMAND_IDS.contains(&k.as_str()))
            .map(|k| k.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_shortcuts_default_is_empty() {
        let shortcuts = CustomShortcuts::default();
        assert!(shortcuts.overrides.is_empty());
        assert!(!shortcuts.has_overrides());
    }

    #[test]
    fn custom_shortcuts_serde_roundtrip() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_issues".to_string(), "Alt+5".to_string());
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "Alt+8".to_string());

        let json = serde_json::to_string(&shortcuts).unwrap();
        let parsed: CustomShortcuts = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.overrides.len(), 2);
        assert_eq!(
            parsed.overrides.get("toggle_issues"),
            Some(&"Alt+5".to_string())
        );
    }

    #[test]
    fn custom_shortcuts_empty_json() {
        let json = "{}";
        let shortcuts: CustomShortcuts = serde_json::from_str(json).unwrap();
        assert!(shortcuts.overrides.is_empty());
    }

    #[test]
    fn parsed_shortcut_ctrl_shift_r() {
        let p = ParsedShortcut::parse("Ctrl+Shift+R").unwrap();
        assert!(p.control);
        assert!(p.shift);
        assert!(!p.alt);
        assert!(!p.platform);
        assert_eq!(p.key, "R");
    }

    #[test]
    fn parsed_shortcut_alt_5() {
        let p = ParsedShortcut::parse("Alt+5").unwrap();
        assert!(!p.control);
        assert!(!p.shift);
        assert!(p.alt);
        assert!(!p.platform);
        assert_eq!(p.key, "5");
    }

    #[test]
    fn parsed_shortcut_case_insensitive() {
        let p = ParsedShortcut::parse("CTRL+SHIFT+R").unwrap();
        assert!(p.control);
        assert!(p.shift);
    }

    #[test]
    fn parsed_shortcut_platform_key() {
        let p = ParsedShortcut::parse("Cmd+K").unwrap();
        assert!(!p.control);
        assert!(!p.shift);
        assert!(!p.alt);
        assert!(p.platform);
        assert_eq!(p.key, "K");
    }

    #[test]
    fn parsed_shortcut_simple_key() {
        let p = ParsedShortcut::parse("j").unwrap();
        assert!(!p.control);
        assert!(!p.shift);
        assert!(!p.alt);
        assert!(!p.platform);
        assert_eq!(p.key, "j");
        assert!(p.is_key_only());
    }

    #[test]
    fn parsed_shortcut_question_mark() {
        let p = ParsedShortcut::parse("?").unwrap();
        assert!(p.is_key_only());
        assert_eq!(p.key, "?");
    }

    #[test]
    fn parsed_shortcut_empty_string() {
        assert!(ParsedShortcut::parse("").is_none());
        assert!(ParsedShortcut::parse("   ").is_none());
    }

    #[test]
    fn parsed_shortcut_display() {
        let p = ParsedShortcut::parse("Ctrl+Shift+R").unwrap();
        assert_eq!(format!("{}", p), "Ctrl+Shift+R");
    }

    #[test]
    fn shortcut_for_returns_override() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "Alt+8".to_string());

        assert_eq!(shortcuts.shortcut_for("toggle_stashes"), Some("Alt+8"));
        assert_eq!(shortcuts.shortcut_for("nonexistent"), None);
    }

    #[test]
    fn parsed_shortcut_for_valid() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "Alt+8".to_string());

        let p = shortcuts.parsed_shortcut_for("toggle_stashes").unwrap();
        assert!(p.alt);
        assert_eq!(p.key, "8");
    }

    #[test]
    fn parsed_shortcut_for_invalid_string() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "".to_string());

        assert!(shortcuts.parsed_shortcut_for("toggle_stashes").is_none());
    }

    #[test]
    fn validate_rejects_unknown_commands() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "Alt+8".to_string());
        shortcuts
            .overrides
            .insert("fake_command".to_string(), "Ctrl+X".to_string());

        let invalid = shortcuts.validate();
        assert_eq!(invalid, vec!["fake_command"]);
    }

    #[test]
    fn validate_accepts_known_commands() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts
            .overrides
            .insert("toggle_stashes".to_string(), "Alt+8".to_string());
        shortcuts
            .overrides
            .insert("fetch".to_string(), "F".to_string());

        assert!(shortcuts.validate().is_empty());
    }

    #[test]
    fn parsed_shortcut_aliases() {
        // control = ctrl
        let a = ParsedShortcut::parse("ctrl+K").unwrap();
        let b = ParsedShortcut::parse("control+K").unwrap();
        assert_eq!(a, b);

        // alt = option
        let a = ParsedShortcut::parse("Alt+K").unwrap();
        let b = ParsedShortcut::parse("Option+K").unwrap();
        assert_eq!(a, b);

        // platform = cmd = command = super = meta
        for alias in ["Cmd", "Command", "Super", "Meta"] {
            let p = ParsedShortcut::parse(&format!("{}+K", alias)).unwrap();
            assert!(p.platform, "alias {} should set platform=true", alias);
        }
    }
}
