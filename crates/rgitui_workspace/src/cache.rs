//! Generic LRU cache with bounded capacity.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

/// A bounded Least-Recently-Used cache.
///
/// Evicts the least-recently-used entry when capacity is exceeded on insert.
/// The "most recently used" end is the back of the internal queue; the
/// "least recently used" end is the front.
#[derive(Debug)]
pub struct LruCache<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    cap: usize,
}

impl<K, V> LruCache<K, V>
where
    K: Hash + Eq + Clone,
{
    /// Create a new cache with the given capacity.
    ///
    /// Capacity must be > 0.  Panics otherwise.
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "LruCache capacity must be > 0");
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    /// Look up a key and mark it as most-recently-used.
    ///
    /// Returns the value if present, otherwise `None`.
    pub fn get(&mut self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        if self.map.contains_key(key) {
            log::debug!("LruCache::get hit: len={}/{}", self.map.len(), self.cap);
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
                self.order.push_back(key.clone());
            }
            self.map.get(key).cloned()
        } else {
            log::debug!("LruCache::get miss: len={}/{}", self.map.len(), self.cap);
            None
        }
    }

    /// Check whether the cache contains a key without updating LRU order.
    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Insert a key-value pair.
    ///
    /// If the key was already present, its value is updated and the entry
    /// is re-marked as most-recently-used (moved to back of LRU order).
    /// If the key is new and the cache is at capacity, the least-recently-used
    /// entry (oldest) is evicted first.
    pub fn insert(&mut self, key: K, value: V) {
        // If key already exists: update value, move to back of LRU order, done.
        // No eviction needed for an in-place update.
        if self.map.contains_key(&key) {
            // Remove key from its current position in the order queue.
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
            self.order.push_back(key.clone());
            self.map.insert(key, value);
            return;
        }

        // New key: evict LRU if at capacity.
        if self.map.len() >= self.cap {
            log::debug!(
                "LruCache::insert eviction: len={}/{}",
                self.map.len(),
                self.cap
            );
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }

        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    /// Returns the number of entries currently cached.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if the cache holds no entries.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns the configured capacity.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.cap
    }
}

impl<K, V> LruCache<K, Arc<V>>
where
    K: Hash + Eq + Clone,
{
    /// Variant of `get` that returns an `Arc<V>` directly (no clone needed).
    #[allow(dead_code)]
    pub fn get_arc(&mut self, key: &K) -> Option<Arc<V>> {
        if self.map.contains_key(key) {
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
                self.order.push_back(key.clone());
            }
            self.map.get(key).cloned()
        } else {
            None
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with_cap<K, V>(cap: usize) -> LruCache<K, V>
    where
        K: Hash + Eq + Clone,
    {
        LruCache::new(cap)
    }

    // ── Basic ops ────────────────────────────────────────────────────────────

    #[test]
    fn new_cache_is_empty() {
        let c: LruCache<i32, &str> = LruCache::new(10);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.capacity(), 10);
    }

    #[test]
    fn insert_and_get() {
        let mut c = cache_with_cap(3);
        c.insert(1, "a");
        assert_eq!(c.get(&1), Some("a"));
        assert!(c.contains(&1));
        assert!(!c.contains(&2));
    }

    #[test]
    fn get_updates_lru_order() {
        let mut c = cache_with_cap(3);
        c.insert(1, "a");
        c.insert(2, "b");
        c.insert(3, "c");

        // Access 1, making it most-recently-used.
        assert_eq!(c.get(&1), Some("a"));

        // Insert 4 — should evict 2 (LRU after 1 was touched).
        c.insert(4, "d");

        assert!(!c.contains(&2), "2 should be evicted");
        assert!(c.contains(&1), "1 should NOT be evicted");
        assert!(c.contains(&3), "3 should NOT be evicted");
        assert!(c.contains(&4), "4 should be present");
    }

    #[test]
    fn update_existing_key_does_not_evict() {
        let mut c = cache_with_cap(2);
        c.insert(1, "a");
        c.insert(2, "b");

        // Update key that is already in cache — should not affect LRU list
        // and should not cause eviction of other entries.
        c.insert(1, "a-updated");

        assert_eq!(c.get(&1), Some("a-updated"));
        assert_eq!(c.get(&2), Some("b"));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn lru_eviction_order() {
        let mut c = cache_with_cap(3);
        c.insert(1, "a");
        c.insert(2, "b");
        c.insert(3, "c");

        // Evict LRU: 1 is oldest.
        c.insert(4, "d");

        assert!(!c.contains(&1));
        assert!(c.contains(&2));
        assert!(c.contains(&3));
        assert!(c.contains(&4));

        // Evict next: 2 is now oldest.
        c.insert(5, "e");

        assert!(!c.contains(&2));
        assert!(c.contains(&3));
        assert!(c.contains(&4));
        assert!(c.contains(&5));
    }

    #[test]
    fn reinsert_evicted_key_at_end() {
        let mut c = cache_with_cap(2);
        c.insert(1, "a");
        c.insert(2, "b");

        // 1 is evicted when 3 is inserted.
        c.insert(3, "c");
        assert!(!c.contains(&1));
        assert!(c.contains(&2));
        assert!(c.contains(&3));

        // Re-insert 1 — it becomes MRU (back of order).
        c.insert(1, "a-again");
        assert!(c.contains(&1));
        // 3 is now LRU (it was inserted before 1).
        assert!(c.contains(&3));

        // 4 is new and cache is full → 3 (LRU) is evicted.
        c.insert(4, "d");
        assert!(c.contains(&1), "1 should be back in cache");
        assert!(!c.contains(&2), "2 was already evicted earlier");
        assert!(
            !c.contains(&3),
            "3 should be evicted (it became LRU after 1 was reinserted)"
        );
        assert!(c.contains(&4));
    }

    #[test]
    fn full_cache_at_capacity() {
        let mut c = cache_with_cap(3);
        c.insert(1, "a");
        c.insert(2, "b");
        c.insert(3, "c");
        assert_eq!(c.len(), 3);
        assert_eq!(c.capacity(), 3);

        // Adding one more evicts the LRU entry.
        c.insert(4, "d");
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn single_insert_then_evict() {
        let mut c: LruCache<i32, &str> = LruCache::new(2);
        c.insert(1, "one");
        assert_eq!(c.len(), 1);
        c.insert(2, "two");
        assert_eq!(c.len(), 2);
        c.insert(3, "three");
        // 1 should be evicted (oldest).
        assert!(!c.contains(&1));
        assert!(c.contains(&2));
        assert!(c.contains(&3));
    }

    #[test]
    fn contains_after_get() {
        let mut c = cache_with_cap(3);
        c.insert(1, "a");
        c.insert(2, "b");

        c.get(&1);

        c.insert(3, "c");
        c.insert(4, "d");

        // 1 was touched and is therefore NOT LRU — 2 should be evicted.
        assert!(!c.contains(&2));
        assert!(c.contains(&1));
    }

    #[test]
    fn get_arc_returns_cloned_arc() {
        let mut c: LruCache<i32, Arc<String>> = LruCache::new(2);
        c.insert(1, Arc::new("hello".into()));

        let first = c.get_arc(&1).unwrap();
        let second = c.get_arc(&1).unwrap();

        // Both should be the same Arc (no clone on get_arc).
        assert!(std::ptr::eq(&*first, &*second));
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn new_with_zero_capacity_panics() {
        let _: LruCache<i32, &str> = LruCache::new(0);
    }
}
