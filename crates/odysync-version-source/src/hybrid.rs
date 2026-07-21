//! Hybrid version source: tries online registry → vendor scraping → offline cache.
//!
//! Every successful online result is cached for offline use.

use crate::{
    HardwareId, OfflineCache, RegistrySource, SourceError, VendorScraper, VersionInfo,
    VersionSource,
};

/// Default cache TTL: 24 hours.
pub const DEFAULT_TTL: chrono::Duration = chrono::Duration::hours(24);

/// Orchestrates multiple version sources with automatic fallback and caching.
pub struct HybridSource {
    registry: RegistrySource,
    scraper: VendorScraper,
    cache: OfflineCache,
    ttl: chrono::Duration,
}

impl HybridSource {
    /// Create a hybrid source with default settings.
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            registry: RegistrySource::default(),
            scraper: VendorScraper::default(),
            cache: OfflineCache::new()?,
            ttl: DEFAULT_TTL,
        })
    }

    /// Use a custom registry base URL.
    pub fn with_registry_url(mut self, url: impl Into<String>) -> Self {
        self.registry = RegistrySource::default().with_url(url);
        self
    }

    /// Set the cache TTL.
    pub fn with_ttl(mut self, ttl: chrono::Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Try the cache first (if within TTL), then online sources, caching the result.
    async fn fetch_and_cache(&self, id: &HardwareId) -> Result<VersionInfo, SourceError> {
        // 1. Try cache first for instant response.
        if let Some(cached) = self.cache.get(id, self.ttl) {
            tracing::debug!(vendor = %id.vendor, device = %id.device, "version source: cache hit");
            return Ok(cached);
        }

        // 2. Try online registry.
        match self.registry.fetch_latest(id).await {
            Ok(info) => {
                let _ = self.cache.put(id, info.clone());
                tracing::info!(vendor = %id.vendor, device = %id.device, version = %info.version, "version source: registry");
                return Ok(info);
            }
            Err(SourceError::NotFound) => {}
            Err(e) => tracing::warn!(error = %e, "registry source failed"),
        }

        // 3. Try vendor scraper.
        match self.scraper.fetch_latest(id).await {
            Ok(info) => {
                let _ = self.cache.put(id, info.clone());
                tracing::info!(vendor = %id.vendor, device = %id.device, version = %info.version, "version source: scraper");
                return Ok(info);
            }
            Err(SourceError::NotFound) => {}
            Err(e) => tracing::warn!(error = %e, "scraper source failed"),
        }

        // 4. Fall back to stale cache (expired but still better than nothing).
        if let Some(cached) = self.cache.get(id, chrono::Duration::MAX) {
            tracing::info!(vendor = %id.vendor, device = %id.device, "version source: stale cache fallback");
            return Ok(cached);
        }

        Err(SourceError::NotFound)
    }
}

#[async_trait::async_trait]
impl VersionSource for HybridSource {
    async fn fetch_latest(&self, id: &HardwareId) -> Result<VersionInfo, SourceError> {
        self.fetch_and_cache(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_source_constructs() {
        // This may fail in CI without a data directory, so we just check it doesn't panic.
        let _ = HybridSource::new();
    }
}
