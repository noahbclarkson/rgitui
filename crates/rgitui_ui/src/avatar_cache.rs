use std::collections::{HashMap, HashSet};

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
    pub fn needs_fetch(&self, email: &str) -> bool {
        if self.pending.contains(email) {
            return false;
        }
        match self.cache.get(email) {
            None => true,
            Some(AvatarState::NotFound(retries)) => *retries < 3,
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
