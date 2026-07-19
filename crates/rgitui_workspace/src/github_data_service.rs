use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::Sender;
use gpui::{Context, Task};
use http_client::HttpClient;

use crate::github_api::{
    github_get_collection, GithubCollectionError, GithubCollectionResponse, GithubRateLimit,
};

const DEFAULT_CACHE_CAPACITY: usize = 128;
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);
const DEFAULT_CACHE_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GithubDataKey {
    pub owner: String,
    pub repo: String,
    pub resource: String,
    pub auth_fingerprint: Option<u64>,
}

impl GithubDataKey {
    pub fn new(owner: &str, repo: &str, resource: &str, token: Option<&str>) -> Self {
        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            resource: resource.to_string(),
            auth_fingerprint: token_fingerprint(token),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GithubFetchPolicy {
    UseFresh,
    Revalidate,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubCollectionPayload {
    pub bodies: Arc<[String]>,
    pub rate_limit: GithubRateLimit,
    pub from_cache: bool,
}

type CollectionResult = Result<GithubCollectionPayload, GithubCollectionError>;

#[derive(Clone, Debug)]
struct CacheEntry {
    bodies: Arc<[String]>,
    etag: Option<String>,
    rate_limit: GithubRateLimit,
    fetched_at: Instant,
}

impl CacheEntry {
    fn payload(&self, from_cache: bool) -> GithubCollectionPayload {
        GithubCollectionPayload {
            bodies: self.bodies.clone(),
            rate_limit: self.rate_limit.clone(),
            from_cache,
        }
    }

    fn source_bytes(&self) -> usize {
        self.bodies.iter().map(String::len).sum()
    }
}

#[derive(Debug)]
enum CacheLookup {
    Fresh(CacheEntry),
    Stale(CacheEntry),
    Miss,
}

#[derive(Debug)]
struct GithubMemoryCache {
    entries: HashMap<GithubDataKey, CacheEntry>,
    order: VecDeque<GithubDataKey>,
    capacity: usize,
    ttl: Duration,
    max_source_bytes: usize,
    source_bytes: usize,
}

impl GithubMemoryCache {
    fn new(capacity: usize, ttl: Duration, max_source_bytes: usize) -> Self {
        assert!(capacity > 0);
        assert!(max_source_bytes > 0);
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
            ttl,
            max_source_bytes,
            source_bytes: 0,
        }
    }

    fn lookup(&mut self, key: &GithubDataKey, now: Instant) -> CacheLookup {
        let Some(entry) = self.entries.get(key).cloned() else {
            return CacheLookup::Miss;
        };
        self.touch(key);
        if now.saturating_duration_since(entry.fetched_at) <= self.ttl {
            CacheLookup::Fresh(entry)
        } else {
            CacheLookup::Stale(entry)
        }
    }

    fn insert(&mut self, key: GithubDataKey, entry: CacheEntry) {
        let entry_bytes = entry.source_bytes();
        if entry_bytes > self.max_source_bytes {
            log::warn!(
                "GitHub cache entry is too large to retain: {} bytes (budget {})",
                entry_bytes,
                self.max_source_bytes
            );
            if let Some(replaced) = self.entries.remove(&key) {
                self.source_bytes = self.source_bytes.saturating_sub(replaced.source_bytes());
                self.order.retain(|candidate| candidate != &key);
            }
            return;
        }

        if let Some(replaced) = self.entries.remove(&key) {
            self.source_bytes = self.source_bytes.saturating_sub(replaced.source_bytes());
            self.order.retain(|candidate| candidate != &key);
        }
        while self.entries.len() >= self.capacity
            || self.source_bytes.saturating_add(entry_bytes) > self.max_source_bytes
        {
            if let Some(evicted) = self.order.pop_front() {
                if let Some(evicted) = self.entries.remove(&evicted) {
                    self.source_bytes = self.source_bytes.saturating_sub(evicted.source_bytes());
                }
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.source_bytes = self.source_bytes.saturating_add(entry_bytes);
        self.entries.insert(key, entry);
    }

    fn touch(&mut self, key: &GithubDataKey) {
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
    }
}

pub(crate) struct GithubDataService {
    cache: GithubMemoryCache,
    in_flight: HashMap<GithubDataKey, Vec<Sender<CollectionResult>>>,
    latest_rate_limits: HashMap<(String, String, Option<u64>), GithubRateLimit>,
}

impl GithubDataService {
    pub fn new() -> Self {
        Self {
            cache: GithubMemoryCache::new(
                DEFAULT_CACHE_CAPACITY,
                DEFAULT_CACHE_TTL,
                DEFAULT_CACHE_SOURCE_BYTES,
            ),
            in_flight: HashMap::new(),
            latest_rate_limits: HashMap::new(),
        }
    }

    pub fn fetch_collection(
        &mut self,
        key: GithubDataKey,
        url: String,
        token: Option<String>,
        http: Arc<dyn HttpClient>,
        policy: GithubFetchPolicy,
        cx: &mut Context<Self>,
    ) -> Task<CollectionResult> {
        let now = Instant::now();
        let cached = self.cache.lookup(&key, now);
        if policy == GithubFetchPolicy::UseFresh {
            if let CacheLookup::Fresh(entry) = &cached {
                return Task::ready(Ok(entry.payload(true)));
            }
        }

        let previous = match cached {
            CacheLookup::Fresh(entry) | CacheLookup::Stale(entry) => Some(entry),
            CacheLookup::Miss => None,
        };
        let (sender, receiver) = async_channel::bounded(1);
        let should_start = self.register_waiter(key.clone(), sender);

        if should_start {
            let owner = key.owner.clone();
            let repo = key.repo.clone();
            let etag = previous.as_ref().and_then(|entry| entry.etag.clone());
            let request_key = key.clone();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                let response = github_get_collection(
                    &http,
                    &url,
                    token.as_deref(),
                    &owner,
                    &repo,
                    etag.as_deref(),
                )
                .await;
                cx.update(|cx| {
                    this.update(cx, |service, _cx| {
                        service.finish_request(request_key, previous, response, Instant::now());
                    })
                    .ok();
                });
            })
            .detach();
        }

        cx.spawn(async move |_, _| {
            receiver.recv().await.unwrap_or_else(|_| {
                Err(GithubCollectionError::transport(
                    "GitHub request was cancelled".into(),
                ))
            })
        })
    }

    fn register_waiter(&mut self, key: GithubDataKey, sender: Sender<CollectionResult>) -> bool {
        match self.in_flight.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push(sender);
                false
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(vec![sender]);
                true
            }
        }
    }

    fn finish_request(
        &mut self,
        key: GithubDataKey,
        previous: Option<CacheEntry>,
        response: Result<GithubCollectionResponse, GithubCollectionError>,
        now: Instant,
    ) {
        let result = match response {
            Ok(response) => {
                self.remember_rate_limit(&key, response.rate_limit.clone());
                match resolve_response(previous, response, now) {
                    Ok(entry) => {
                        let payload = entry.payload(false);
                        self.cache.insert(key.clone(), entry);
                        Ok(payload)
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => {
                self.remember_rate_limit(&key, error.rate_limit.clone());
                Err(error)
            }
        };

        if let Some(waiters) = self.in_flight.remove(&key) {
            for waiter in waiters {
                let _ = waiter.try_send(result.clone());
            }
        }
    }

    fn remember_rate_limit(&mut self, key: &GithubDataKey, rate_limit: GithubRateLimit) {
        if rate_limit.remaining.is_some()
            || rate_limit.reset_epoch_seconds.is_some()
            || rate_limit.retry_after_seconds.is_some()
        {
            if rate_limit
                .remaining
                .is_some_and(|remaining| remaining <= 10)
            {
                log::warn!(
                    "GitHub API quota low for {}/{}: remaining={:?}, reset={:?}",
                    key.owner,
                    key.repo,
                    rate_limit.remaining,
                    rate_limit.reset_epoch_seconds
                );
            }
            self.latest_rate_limits.insert(
                (key.owner.clone(), key.repo.clone(), key.auth_fingerprint),
                rate_limit,
            );
        }
    }
}

fn resolve_response(
    previous: Option<CacheEntry>,
    response: GithubCollectionResponse,
    now: Instant,
) -> Result<CacheEntry, GithubCollectionError> {
    if response.not_modified {
        let Some(mut previous) = previous else {
            return Err(GithubCollectionError::transport(
                "GitHub returned 304 without a cached response".into(),
            ));
        };
        previous.fetched_at = now;
        previous.etag = response.etag.or(previous.etag);
        previous.rate_limit = response.rate_limit;
        return Ok(previous);
    }
    Ok(CacheEntry {
        bodies: response.bodies,
        etag: response.etag,
        rate_limit: response.rate_limit,
        fetched_at: now,
    })
}

fn token_fingerprint(token: Option<&str>) -> Option<u64> {
    token.filter(|token| !token.is_empty()).map(|token| {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        hasher.finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(resource: &str) -> GithubDataKey {
        GithubDataKey::new("owner", "repo", resource, Some("token"))
    }

    fn entry(body: &[&str], fetched_at: Instant) -> CacheEntry {
        CacheEntry {
            bodies: body.iter().map(|body| (*body).to_string()).collect(),
            etag: Some("etag-1".into()),
            rate_limit: GithubRateLimit::default(),
            fetched_at,
        }
    }

    #[test]
    fn cache_keeps_successful_empty_collection_and_expires_by_ttl() {
        let now = Instant::now();
        let mut cache = GithubMemoryCache::new(2, Duration::from_secs(60), 1024);
        let request_key = key("issues-open");
        cache.insert(request_key.clone(), entry(&["[]"], now));

        assert!(matches!(
            cache.lookup(&request_key, now),
            CacheLookup::Fresh(_)
        ));
        assert!(matches!(
            cache.lookup(&request_key, now + Duration::from_secs(61)),
            CacheLookup::Stale(_)
        ));
    }

    #[test]
    fn cache_evicts_least_recently_used_entry() {
        let now = Instant::now();
        let mut cache = GithubMemoryCache::new(2, Duration::from_secs(60), 1024);
        let first = key("first");
        let second = key("second");
        let third = key("third");
        cache.insert(first.clone(), entry(&["[1]"], now));
        cache.insert(second.clone(), entry(&["[2]"], now));
        let _ = cache.lookup(&first, now);
        cache.insert(third, entry(&["[3]"], now));

        assert!(cache.entries.contains_key(&first));
        assert!(!cache.entries.contains_key(&second));
    }

    #[test]
    fn cache_evicts_entries_to_stay_within_source_byte_budget() {
        let now = Instant::now();
        let mut cache = GithubMemoryCache::new(8, Duration::from_secs(60), 10);
        let first = key("first");
        let second = key("second");
        cache.insert(first.clone(), entry(&["123456"], now));
        cache.insert(second.clone(), entry(&["abcdef"], now));

        assert!(!cache.entries.contains_key(&first));
        assert!(cache.entries.contains_key(&second));
        assert_eq!(cache.source_bytes, 6);
    }

    #[test]
    fn cache_does_not_retain_single_oversized_entry() {
        let now = Instant::now();
        let mut cache = GithubMemoryCache::new(8, Duration::from_secs(60), 5);
        let request_key = key("oversized");
        cache.insert(request_key.clone(), entry(&["123456"], now));

        assert!(!cache.entries.contains_key(&request_key));
        assert_eq!(cache.source_bytes, 0);
    }

    #[test]
    fn request_key_partitions_auth_and_resource() {
        let issues = GithubDataKey::new("owner", "repo", "issues", Some("token-a"));
        let prs = GithubDataKey::new("owner", "repo", "prs", Some("token-a"));
        let other_auth = GithubDataKey::new("owner", "repo", "issues", Some("token-b"));
        assert_ne!(issues, prs);
        assert_ne!(issues, other_auth);
    }

    #[test]
    fn singleflight_registers_only_the_first_waiter_as_starter() {
        let mut service = GithubDataService::new();
        let (first, _first_rx) = async_channel::bounded(1);
        let (second, _second_rx) = async_channel::bounded(1);
        let request_key = key("issues");
        assert!(service.register_waiter(request_key.clone(), first));
        assert!(!service.register_waiter(request_key.clone(), second));
        assert_eq!(service.in_flight[&request_key].len(), 2);
    }

    #[test]
    fn singleflight_completion_delivers_one_response_to_all_waiters() {
        let mut service = GithubDataService::new();
        let (first, first_rx) = async_channel::bounded(1);
        let (second, second_rx) = async_channel::bounded(1);
        let request_key = key("issues");
        assert!(service.register_waiter(request_key.clone(), first));
        assert!(!service.register_waiter(request_key.clone(), second));

        service.finish_request(
            request_key.clone(),
            None,
            Ok(GithubCollectionResponse {
                bodies: vec!["[]".to_string()].into(),
                etag: Some("etag-1".into()),
                rate_limit: GithubRateLimit::default(),
                not_modified: false,
            }),
            Instant::now(),
        );

        assert_eq!(&*first_rx.try_recv().unwrap().unwrap().bodies, &["[]"]);
        assert_eq!(&*second_rx.try_recv().unwrap().unwrap().bodies, &["[]"]);
        assert!(!service.in_flight.contains_key(&request_key));
    }

    #[test]
    fn not_modified_reuses_cached_payload_and_refreshes_timestamp() {
        let old = Instant::now();
        let now = old + Duration::from_secs(120);
        let previous = entry(&["[1,2,3]"], old);
        let response = GithubCollectionResponse {
            bodies: Vec::new().into(),
            etag: Some("etag-1".into()),
            rate_limit: GithubRateLimit {
                remaining: Some(4999),
                ..Default::default()
            },
            not_modified: true,
        };
        let resolved = resolve_response(Some(previous), response, now).unwrap();
        assert_eq!(&*resolved.bodies, &["[1,2,3]".to_string()]);
        assert_eq!(resolved.fetched_at, now);
        assert_eq!(resolved.rate_limit.remaining, Some(4999));
    }
}
