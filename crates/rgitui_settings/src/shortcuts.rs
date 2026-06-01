use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CustomShortcuts {
    #[serde(default)]
    pub overrides: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_shortcuts_default_is_empty() {
        let shortcuts = CustomShortcuts::default();
        assert!(shortcuts.overrides.is_empty());
    }

    #[test]
    fn custom_shortcuts_serde_roundtrip() {
        let mut shortcuts = CustomShortcuts::default();
        shortcuts.overrides.insert(
            "toggle_issues".to_string(),
            "Alt+5".to_string(),
        );
        shortcuts.overrides.insert(
            "toggle_stashes".to_string(),
            "Alt+8".to_string(),
        );

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
}