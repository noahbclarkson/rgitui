use std::collections::{HashMap, HashSet};
use std::fs;
use std::path;

use gpui::Global;

/// Resolved avatar state for an email address.
#[derive(Clone, Debug)]
pub enum AvatarState {
    /// Successfully resolved to a URL.
    Resolved(String),
    /// Tried all sources, no avatar found. Tracks retry count.
    NotFound(u8),
}

/// Global cache mapping email addresses to avatar image URLs.
///
/// Render code reads from this via `cx.try_global::<AvatarCache>()`.
/// Resolution tasks (spawned by the workspace) write to it and notify views.
///
/// Optionally persisted to disk at `$CONFIG_DIR/rgitui/avatar_cache.txt`
/// (one line per resolved entry: `email=url`). NotFound entries are not
/// persisted — they are re-attempted on next startup up to 3 times.
pub struct AvatarCache {
    cache: HashMap<String, AvatarState>,
    pending: HashSet<String>,
}

impl Global for AvatarCache {}

impl Default for AvatarCache {
    fn default() -> Self {
        Self::new()
    }
}

impl AvatarCache {
    /// Returns the path where the avatar disk cache is stored.
    fn disk_cache_path() -> path::PathBuf {
        rgitui_settings::config_dir()
            .join("rgitui")
            .join("avatar_cache.txt")
    }

    /// Load resolved avatar entries from the disk cache.
    /// Returns a HashMap of email → url for entries that were successfully resolved.
    /// Silently returns an empty map if the file does not exist or is unreadable.
    pub fn load_from_disk() -> HashMap<String, String> {
        let path = Self::disk_cache_path();
        let Ok(content) = fs::read_to_string(&path) else {
            return HashMap::new();
        };
        Self::parse_entries(&content)
    }

    /// Parse email=url entries from cache file content.
    fn parse_entries(content: &str) -> HashMap<String, String> {
        let mut entries = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(eq) = line.find('=') {
                let email = line[..eq].to_string();
                let url = line[eq + 1..].to_string();
                if !email.is_empty() && !url.is_empty() {
                    entries.insert(email, url);
                }
            }
        }
        entries
    }

    /// Save a resolved email→url mapping to the disk cache.
    /// Reads the existing file, merges the new entry, writes back.
    /// Silently does nothing if the file cannot be read or written.
    pub fn save_entry_to_disk(email: &str, url: &str) {
        let path = Self::disk_cache_path();
        // Read existing entries to avoid overwriting concurrent writes from other tasks
        let existing_content = fs::read_to_string(&path).unwrap_or_default();
        let mut entries = Self::parse_entries(&existing_content);
        // Create parent dir if needed
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Merge + write
        entries.insert(email.to_string(), url.to_string());
        let content: String = entries
            .iter()
            .map(|(e, u)| format!("{e}={u}"))
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = fs::write(&path, content) {
            eprintln!("rgitui: failed to write avatar disk cache: {e}");
        }
    }

    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
        }
    }

    /// Get the cached avatar state for an email, if resolved.
    pub fn get(&self, email: &str) -> Option<&AvatarState> {
        self.cache.get(email)
    }

    /// Check if a fetch is already in flight for this email.
    pub fn is_pending(&self, email: &str) -> bool {
        self.pending.contains(email)
    }

    /// Returns true if we should start a fetch (not cached, not pending, or retry limit not hit).
    /// Allows up to 5 total attempts (1 initial + 4 retries) before giving up on an email.
    pub fn needs_fetch(&self, email: &str) -> bool {
        if self.pending.contains(email) {
            return false;
        }
        match self.cache.get(email) {
            None => true,
            Some(AvatarState::NotFound(retries)) => *retries < 5,
            Some(AvatarState::Resolved(_)) => false,
        }
    }

    /// Mark an email as having an in-flight fetch.
    pub fn mark_pending(&mut self, email: &str) {
        self.pending.insert(email.to_string());
    }

    /// Store a resolved avatar URL.
    pub fn set_resolved(&mut self, email: String, url: String) {
        self.pending.remove(&email);
        self.cache.insert(email, AvatarState::Resolved(url));
    }

    /// Mark an email as having no avatar, incrementing the retry count.
    pub fn set_not_found(&mut self, email: String) {
        self.pending.remove(&email);
        let retries = match self.cache.get(&email) {
            Some(AvatarState::NotFound(n)) => n + 1,
            _ => 1,
        };
        self.cache.insert(email, AvatarState::NotFound(retries));
    }

    /// Get the avatar URL for an email if resolved, None otherwise.
    pub fn avatar_url(&self, email: &str) -> Option<&str> {
        match self.cache.get(email)? {
            AvatarState::Resolved(url) => Some(url.as_str()),
            AvatarState::NotFound(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: write content to a temp file and return its path.
    fn temp_file(content: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "rgitui_avatar_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, content).unwrap();
        tmp
    }

    /// Parse entries from a given path (used to test the parser in isolation
    /// without hitting the real disk cache path).
    fn parse_from_path(path: &std::path::Path) -> HashMap<String, String> {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        AvatarCache::parse_entries(&content)
    }

    #[test]
    fn test_parse_entries_single() {
        let path = temp_file("alice@example.com=https://avatars.githubusercontent.com/u/1\n");
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries.get("alice@example.com"),
            Some(&"https://avatars.githubusercontent.com/u/1".to_string())
        );
    }

    #[test]
    fn test_parse_entries_multiple() {
        let path = temp_file(
            "alice@example.com=https://a.com\n\
             bob@example.com=https://b.com\n",
        );
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.get("alice@example.com"),
            Some(&"https://a.com".to_string())
        );
        assert_eq!(
            entries.get("bob@example.com"),
            Some(&"https://b.com".to_string())
        );
    }

    #[test]
    fn test_parse_entries_empty_file() {
        let path = temp_file("");
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_entries_skips_empty_lines() {
        let path =
            temp_file("alice@example.com=https://a.com\n\n  \nbob@example.com=https://b.com\n");
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_parse_entries_skips_lines_without_equals() {
        let path = temp_file("not-valid\nalice@example.com=https://a.com\nalso-invalid\n");
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("alice@example.com"));
    }

    #[test]
    fn test_parse_entries_skips_empty_email_or_url() {
        let path =
            temp_file("=https://a.com\nalice@example.com=\nvalid@example.com=https://b.com\n");
        let entries = parse_from_path(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("valid@example.com"));
    }

    // --- AvatarCache in-memory behaviour tests ---

    #[test]
    fn test_cache_set_and_get_resolved() {
        let mut cache = AvatarCache::new();
        cache.set_resolved("alice@example.com".to_string(), "https://a.com".to_string());
        assert_eq!(cache.avatar_url("alice@example.com"), Some("https://a.com"));
        assert_eq!(cache.avatar_url("unknown@example.com"), None);
    }

    #[test]
    fn test_needs_fetch_false_for_resolved() {
        let mut cache = AvatarCache::new();
        cache.set_resolved("alice@example.com".to_string(), "https://a.com".to_string());
        assert!(!cache.needs_fetch("alice@example.com"));
    }

    #[test]
    fn test_not_found_retry_limit() {
        let mut cache = AvatarCache::new();
        // 1st-4th failure → still retry (initial + up to 4 retries = 5 total attempts)
        for i in 1..=4 {
            cache.set_not_found("alice@example.com".to_string());
            assert!(
                cache.needs_fetch("alice@example.com"),
                "should retry on failure #{i}"
            );
        }
        // 5th failure → give up
        cache.set_not_found("alice@example.com".to_string());
        assert!(!cache.needs_fetch("alice@example.com"));
    }

    #[test]
    fn test_pending_blocks_fetch() {
        let mut cache = AvatarCache::new();
        cache.mark_pending("alice@example.com");
        assert!(!cache.needs_fetch("alice@example.com"));
        assert!(cache.is_pending("alice@example.com"));
    }

    #[test]
    fn test_resolve_clears_pending() {
        let mut cache = AvatarCache::new();
        cache.mark_pending("alice@example.com");
        assert!(cache.is_pending("alice@example.com"));
        cache.set_resolved("alice@example.com".to_string(), "https://a.com".to_string());
        assert!(!cache.is_pending("alice@example.com"));
        assert_eq!(cache.avatar_url("alice@example.com"), Some("https://a.com"));
    }

    #[test]
    fn test_resolve_overwrites_not_found() {
        let mut cache = AvatarCache::new();
        cache.set_not_found("alice@example.com".to_string());
        cache.set_resolved("alice@example.com".to_string(), "https://a.com".to_string());
        assert!(!cache.needs_fetch("alice@example.com"));
        assert_eq!(cache.avatar_url("alice@example.com"), Some("https://a.com"));
    }
}
