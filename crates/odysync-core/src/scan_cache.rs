//! Scan result cache to avoid redundant backend scans within a short time window.
//!
//! Each backend's scan result is cached with a timestamp. If a subsequent scan
//! request arrives within the TTL, the cached result is returned instead of
//! re-running the scan. This is particularly useful for the GUI, which may
//! trigger scans from multiple UI events (window focus, timer, manual refresh).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::error::Result;
use crate::model::{BackendKind, UpdateCandidate};

/// Default cache TTL: 30 seconds. Scans are cheap enough to repeat after this.
const DEFAULT_TTL: Duration = Duration::from_secs(30);

/// Thread-safe scan result cache keyed by `BackendKind`.
pub struct ScanCache {
    inner: Arc<RwLock<HashMap<BackendKind, CacheEntry>>>,
    ttl: Duration,
}

struct CacheEntry {
    result: Result<Vec<UpdateCandidate>>,
    cached_at: Instant,
}

impl ScanCache {
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Get a cached scan result if it's still fresh.
    pub async fn get(&self, kind: BackendKind) -> Option<Result<Vec<UpdateCandidate>>> {
        let map = self.inner.read().await;
        map.get(&kind).and_then(|entry| {
            if entry.cached_at.elapsed() < self.ttl {
                Some(clone_result(&entry.result))
            } else {
                None
            }
        })
    }

    /// Store a scan result in the cache.
    pub async fn put(&self, kind: BackendKind, result: Result<Vec<UpdateCandidate>>) {
        let mut map = self.inner.write().await;
        map.insert(
            kind,
            CacheEntry {
                result,
                cached_at: Instant::now(),
            },
        );
    }

    /// Invalidate a specific backend's cached result.
    pub async fn invalidate(&self, kind: BackendKind) {
        let mut map = self.inner.write().await;
        map.remove(&kind);
    }

    /// Clear all cached results.
    pub async fn clear(&self) {
        let mut map = self.inner.write().await;
        map.clear();
    }

    /// Get a cached result or compute it via `f` and cache the result.
    pub async fn get_or_insert<F, Fut>(&self, kind: BackendKind, f: F) -> Result<Vec<UpdateCandidate>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<UpdateCandidate>>>,
    {
        if let Some(cached) = self.get(kind).await {
            tracing::debug!(backend = ?kind, "scan cache hit");
            return cached;
        }

        tracing::debug!(backend = ?kind, "scan cache miss, computing");
        let result = f().await;
        self.put(kind, clone_result(&result)).await;
        result
    }
}

impl Default for ScanCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Clone a `Result<Vec<UpdateCandidate>>` without requiring `T: Clone` on the error.
fn clone_result(result: &Result<Vec<UpdateCandidate>>) -> Result<Vec<UpdateCandidate>> {
    match result {
        Ok(candidates) => Ok(candidates.clone()),
        Err(e) => Err(clone_error(e)),
    }
}

/// Clone an error by converting it to a string and wrapping it.
fn clone_error(e: &crate::error::Error) -> crate::error::Error {
    crate::error::Error::Config(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PackageId;
    use crate::version::Version;

    #[tokio::test]
    async fn cache_returns_result_within_ttl() {
        let cache = ScanCache::with_ttl(Duration::from_secs(60));
        let kind = BackendKind::Apt;
        let candidates = vec![UpdateCandidate {
            id: PackageId::new(kind, "test"),
            name: "test".into(),
            installed: Version::parse("1.0"),
            available: Version::parse("2.0"),
            size_bytes: None,
            expected_sha256: None,
        }];

        cache.put(kind, Ok(candidates.clone())).await;

        let cached = cache.get(kind).await;
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert!(cached.is_ok());
        assert_eq!(cached.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cache_expires_after_ttl() {
        let cache = ScanCache::with_ttl(Duration::from_millis(10));
        let kind = BackendKind::Apt;

        cache.put(kind, Ok(vec![])).await;

        tokio::time::sleep(Duration::from_millis(20)).await;

        let cached = cache.get(kind).await;
        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn cache_invalidate_removes_entry() {
        let cache = ScanCache::new();
        let kind = BackendKind::Apt;

        cache.put(kind, Ok(vec![])).await;
        cache.invalidate(kind).await;

        assert!(cache.get(kind).await.is_none());
    }

    #[tokio::test]
    async fn cache_clear_removes_all() {
        let cache = ScanCache::new();

        cache.put(BackendKind::Apt, Ok(vec![])).await;
        cache.put(BackendKind::Dnf, Ok(vec![])).await;
        cache.clear().await;

        assert!(cache.get(BackendKind::Apt).await.is_none());
        assert!(cache.get(BackendKind::Dnf).await.is_none());
    }

    #[tokio::test]
    async fn get_or_insert_uses_cache_on_second_call() {
        let cache = ScanCache::with_ttl(Duration::from_secs(60));
        let kind = BackendKind::Apt;
        let mut call_count = 0u32;

        let result1 = cache
            .get_or_insert(kind, || async {
                call_count += 1;
                Ok::<_, crate::error::Error>(vec![])
            })
            .await;
        assert!(result1.is_ok());

        let result2 = cache
            .get_or_insert(kind, || async {
                call_count += 1;
                Ok::<_, crate::error::Error>(vec![])
            })
            .await;
        assert!(result2.is_ok());

        assert_eq!(call_count, 1);
    }

    #[tokio::test]
    async fn cache_stores_errors() {
        let cache = ScanCache::new();
        let kind = BackendKind::Apt;

        cache
            .put(
                kind,
                Err(crate::error::Error::CommandFailed {
                    command: "test".into(),
                    code: 1,
                    stderr: "error".into(),
                }),
            )
            .await;

        let cached = cache.get(kind).await;
        assert!(cached.is_some());
        assert!(cached.unwrap().is_err());
    }
}
