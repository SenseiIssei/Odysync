//! Offline mode: cache manager for storing and applying updates without network.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Manifest entry for a cached installer/update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheManifestEntry {
    pub package_id: String,
    pub backend: String,
    pub version: String,
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub cached_at: String,
}

/// The full cache manifest, stored as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheManifest {
    pub entries: Vec<CacheManifestEntry>,
}

impl CacheManifest {
    fn path() -> PathBuf {
        let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
            .expect("could not resolve data directory");
        let cache_dir = dirs.cache_dir().join("offline-cache");
        std::fs::create_dir_all(&cache_dir).ok();
        cache_dir.join("manifest.json")
    }

    /// Load the manifest from disk, returning an empty one if it doesn't exist.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => CacheManifest::default(),
        }
    }

    /// Save the manifest to disk.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)
    }

    /// Add an entry to the manifest and save.
    pub fn add(&mut self, entry: CacheManifestEntry) -> std::io::Result<()> {
        self.entries.retain(|e| e.package_id != entry.package_id || e.backend != entry.backend);
        self.entries.push(entry);
        self.save()
    }

    /// Remove an entry from the manifest and delete the cached file.
    pub fn remove(&mut self, package_id: &str, backend: &str) -> std::io::Result<()> {
        if let Some(pos) = self.entries.iter().position(|e| e.package_id == package_id && e.backend == backend) {
            let entry = self.entries.remove(pos);
            let manifest_path = Self::path();
            let cache_dir = manifest_path.parent().unwrap();
            let file_path = cache_dir.join(&entry.filename);
            if file_path.exists() {
                std::fs::remove_file(&file_path)?;
            }
            self.save()?;
        }
        Ok(())
    }

    /// Get the cache directory path.
    pub fn cache_dir() -> PathBuf {
        Self::path().parent().unwrap().to_path_buf()
    }

    /// Find an entry by package_id and backend.
    pub fn find(&self, package_id: &str, backend: &str) -> Option<&CacheManifestEntry> {
        self.entries.iter().find(|e| e.package_id == package_id && e.backend == backend)
    }

    /// Total size of all cached files.
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size_bytes).sum()
    }

    /// Clear all entries and delete all cached files.
    pub fn clear(&mut self) -> std::io::Result<()> {
        let cache_dir = Self::cache_dir();
        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir)?;
            std::fs::create_dir_all(&cache_dir)?;
        }
        self.entries.clear();
        self.save()
    }
}

/// Download a file from a URL and cache it with SHA256 verification.
pub async fn download_and_cache(
    url: &str,
    package_id: &str,
    backend: &str,
    version: &str,
    expected_sha256: Option<&str>,
    proxy_url: Option<&str>,
) -> anyhow::Result<CacheManifestEntry> {
    use sha2::{Sha256, Digest};

    let mut client_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300));
    if let Some(proxy) = proxy_url {
        if let Ok(proxy) = reqwest::Proxy::all(proxy) {
            client_builder = client_builder.proxy(proxy);
        }
    }
    let client = client_builder.build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {}", response.status(), url);
    }

    let bytes = response.bytes().await?;
    let size_bytes = bytes.len() as u64;

    // Compute SHA256
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = hasher.finalize();
    let sha256 = hex::encode(hash);

    // Verify if expected hash was provided
    if let Some(expected) = expected_sha256 {
        if sha256 != expected {
            anyhow::bail!("SHA256 mismatch: expected {expected}, got {sha256}");
        }
    }

    // Save to cache directory
    let cache_dir = CacheManifest::cache_dir();
    let filename = format!("{}_{}_{}", backend, package_id, version);
    let filename = filename.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '.', "_");
    let file_path = cache_dir.join(&filename);

    let mut file = fs::File::create(&file_path).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;

    let entry = CacheManifestEntry {
        package_id: package_id.to_string(),
        backend: backend.to_string(),
        version: version.to_string(),
        filename,
        sha256,
        size_bytes,
        cached_at: chrono::Utc::now().to_rfc3339(),
    };

    // Update manifest
    let mut manifest = CacheManifest::load();
    manifest.add(entry.clone())?;

    tracing::info!(package = package_id, backend, size = size_bytes, "cached offline installer");

    Ok(entry)
}

/// Get the path to a cached installer file.
pub fn cached_file_path(filename: &str) -> PathBuf {
    CacheManifest::cache_dir().join(filename)
}

/// Verify a cached file's integrity by checking its SHA256.
pub async fn verify_cached_file(entry: &CacheManifestEntry) -> anyhow::Result<bool> {
    use sha2::{Sha256, Digest};

    let path = cached_file_path(&entry.filename);
    if !path.exists() {
        return Ok(false);
    }

    let data = fs::read(&path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(hasher.finalize());

    Ok(hash == entry.sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips() {
        let mut manifest = CacheManifest::default();
        manifest.entries.push(CacheManifestEntry {
            package_id: "test.pkg".to_string(),
            backend: "winget".to_string(),
            version: "1.0.0".to_string(),
            filename: "winget_test_pkg_1_0_0".to_string(),
            sha256: "abc123".to_string(),
            size_bytes: 1024,
            cached_at: "2024-01-01T00:00:00Z".to_string(),
        });
        let json = serde_json::to_string(&manifest).unwrap();
        let loaded: CacheManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].package_id, "test.pkg");
    }

    #[test]
    fn find_entry_by_package_and_backend() {
        let mut manifest = CacheManifest::default();
        manifest.entries.push(CacheManifestEntry {
            package_id: "pkg.a".to_string(),
            backend: "winget".to_string(),
            version: "1.0".to_string(),
            filename: "a".to_string(),
            sha256: "hash".to_string(),
            size_bytes: 100,
            cached_at: "2024-01-01T00:00:00Z".to_string(),
        });
        manifest.entries.push(CacheManifestEntry {
            package_id: "pkg.b".to_string(),
            backend: "pip".to_string(),
            version: "2.0".to_string(),
            filename: "b".to_string(),
            sha256: "hash2".to_string(),
            size_bytes: 200,
            cached_at: "2024-01-01T00:00:00Z".to_string(),
        });

        assert!(manifest.find("pkg.a", "winget").is_some());
        assert!(manifest.find("pkg.a", "pip").is_none());
        assert!(manifest.find("pkg.b", "pip").is_some());
        assert!(manifest.find("pkg.c", "winget").is_none());
    }

    #[test]
    fn total_size_sums_all_entries() {
        let mut manifest = CacheManifest::default();
        manifest.entries.push(CacheManifestEntry {
            package_id: "a".to_string(),
            backend: "x".to_string(),
            version: "1".to_string(),
            filename: "a".to_string(),
            sha256: "h".to_string(),
            size_bytes: 100,
            cached_at: "t".to_string(),
        });
        manifest.entries.push(CacheManifestEntry {
            package_id: "b".to_string(),
            backend: "y".to_string(),
            version: "2".to_string(),
            filename: "b".to_string(),
            sha256: "h2".to_string(),
            size_bytes: 300,
            cached_at: "t".to_string(),
        });
        assert_eq!(manifest.total_size(), 400);
    }
}
