//! Offline cache for version info, stored as JSON in the platform's data directory.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::{HardwareId, SourceError, VersionInfo};

/// A persistent on-disk cache of version info keyed by `HardwareId`.
pub struct OfflineCache {
    cache_dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CacheFile {
    entries: HashMap<String, CacheEntry>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CacheEntry {
    info: VersionInfo,
    cached_at: chrono::DateTime<chrono::Utc>,
}

impl OfflineCache {
    /// Create a cache rooted at the platform's data directory.
    pub fn new() -> Result<Self, SourceError> {
        let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
            .ok_or_else(|| SourceError::Cache("could not resolve data directory".into()))?;
        let cache_dir = dirs.cache_dir().join("version-cache");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| SourceError::Cache(format!("could not create cache dir: {e}")))?;
        Ok(Self { cache_dir })
    }

    /// Create a cache at a specific path (for testing).
    pub fn with_dir(dir: PathBuf) -> Result<Self, SourceError> {
        std::fs::create_dir_all(&dir)
            .map_err(|e| SourceError::Cache(format!("could not create cache dir: {e}")))?;
        Ok(Self { cache_dir: dir })
    }

    fn cache_path(&self) -> PathBuf {
        self.cache_dir.join("version_cache.json")
    }

    fn load(&self) -> CacheFile {
        match std::fs::read_to_string(self.cache_path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => CacheFile::default(),
        }
    }

    fn save(&self, cache: &CacheFile) -> Result<(), SourceError> {
        let text = serde_json::to_string_pretty(cache)
            .map_err(|e| SourceError::Cache(format!("serialize: {e}")))?;
        let tmp = self.cache_path().with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(|e| SourceError::Cache(format!("write: {e}")))?;
        std::fs::rename(&tmp, self.cache_path())
            .map_err(|e| SourceError::Cache(format!("rename: {e}")))?;
        Ok(())
    }

    fn key(id: &HardwareId) -> String {
        format!("{}:{}", id.vendor, id.device)
    }

    /// Get a cached entry if it exists and is within `ttl`.
    pub fn get(&self, id: &HardwareId, ttl: chrono::Duration) -> Option<VersionInfo> {
        let cache = self.load();
        let entry = cache.entries.get(&Self::key(id))?;
        if chrono::Utc::now() - entry.cached_at > ttl {
            return None;
        }
        Some(entry.info.clone())
    }

    /// Store a version info entry in the cache.
    pub fn put(&self, id: &HardwareId, info: VersionInfo) -> Result<(), SourceError> {
        let mut cache = self.load();
        cache.entries.insert(
            Self::key(id),
            CacheEntry {
                info,
                cached_at: chrono::Utc::now(),
            },
        );
        self.save(&cache)
    }

    /// Remove all cached entries.
    pub fn clear(&self) -> Result<(), SourceError> {
        let cache = CacheFile::default();
        self.save(&cache)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.load().entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_within_ttl() {
        let dir = std::env::temp_dir().join("odysync-version-cache-test-put");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = OfflineCache::with_dir(dir).unwrap();

        let id = HardwareId::new("nvidia", "geforce-rtx-4090");
        let info = VersionInfo {
            version: "566.14".into(),
            download_url: Some("https://example.com/driver.exe".into()),
            release_date: Some("2024-11-05".into()),
            checksum: None,
            notes: None,
        };

        cache.put(&id, info.clone()).unwrap();
        let got = cache.get(&id, chrono::Duration::hours(24));
        assert!(got.is_some());
        assert_eq!(got.unwrap().version, "566.14");
    }

    #[test]
    fn expired_entry_returns_none() {
        let dir = std::env::temp_dir().join("odysync-version-cache-test-expired");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = OfflineCache::with_dir(dir).unwrap();

        let id = HardwareId::new("amd", "rx-7900xtx");
        let info = VersionInfo {
            version: "24.12.1".into(),
            download_url: None,
            release_date: None,
            checksum: None,
            notes: None,
        };

        cache.put(&id, info).unwrap();
        // TTL of zero means immediately expired
        let got = cache.get(&id, chrono::Duration::zero());
        assert!(got.is_none());
    }

    #[test]
    fn clear_empties_cache() {
        let dir = std::env::temp_dir().join("odysync-version-cache-test-clear");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = OfflineCache::with_dir(dir).unwrap();

        let id = HardwareId::new("intel", "arc-a770");
        cache
            .put(
                &id,
                VersionInfo {
                    version: "32.0.101.6083".into(),
                    download_url: None,
                    release_date: None,
                    checksum: None,
                    notes: None,
                },
            )
            .unwrap();

        assert_eq!(cache.len(), 1);
        cache.clear().unwrap();
        assert_eq!(cache.len(), 0);
    }
}
