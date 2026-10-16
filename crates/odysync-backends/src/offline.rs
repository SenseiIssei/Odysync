//! Offline mode: cache manager for storing and applying updates without network.
//!
//! Everything lives under one directory (`<cache>/offline-cache`): the
//! downloaded installers plus a `manifest.json` describing them. The manifest
//! is the index the UI renders; the files next to it are the payload.
//!
//! Two invariants matter here:
//!
//!   * **Nothing panics.** These functions are reachable from Tauri commands,
//!     and a panic across the command boundary takes the app down. Directory
//!     resolution can genuinely fail (a stripped-down service account has no
//!     known cache dir), so it returns `Result` rather than `expect`.
//!   * **No filename escapes the cache directory.** Filenames are read back
//!     out of a JSON file on disk, so they are untrusted input: a manifest
//!     entry naming `..\..\Windows\System32\...` must not turn `remove()` into
//!     an arbitrary-file-delete. Every join goes through [`safe_cache_path`].

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Name of the manifest file inside the cache directory.
const MANIFEST_FILENAME: &str = "manifest.json";

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
    /// How the file's code signature checked out at download time.
    ///
    /// `#[serde(default)]` so a manifest written before this field existed
    /// still loads; those entries read as `Unknown`, which is honest — they
    /// were cached before anything looked.
    #[serde(default)]
    pub signature: CachedSignature,
}

/// What [`verify_installer`](odysync_verify::verify_installer) said about a
/// cached file when it was downloaded.
///
/// Recorded rather than recomputed so the UI can show it without re-hashing an
/// 800 MB installer, and so a file that was signed when it arrived and is not
/// signed now is a visible discrepancy rather than a silent one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum CachedSignature {
    /// Cached before signatures were checked, or on a platform without them.
    #[default]
    Unknown,
    /// Signed, and the signature verified. `subject` is the signing identity.
    Valid { subject: String },
    /// The file carries no code signature at all.
    Unsigned,
    /// A signature is present and did not verify.
    Invalid { detail: String },
}

/// Shorter alias for [`CacheManifestEntry`].
pub type CacheEntry = CacheManifestEntry;

/// The full cache manifest, stored as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheManifest {
    pub entries: Vec<CacheManifestEntry>,
}

// ── Directory resolution ────────────────────────────────────────────────────

/// Directory holding cached installers and the manifest.
///
/// Creates the directory if it does not exist. Returns an error instead of
/// panicking when the platform has no resolvable cache directory.
pub fn cache_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| anyhow::anyhow!("could not resolve the user cache directory"))?;
    let dir = dirs.cache_dir().join("offline-cache");
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Path of `manifest.json` inside `dir`.
fn manifest_path_in(dir: &Path) -> PathBuf {
    dir.join(MANIFEST_FILENAME)
}

/// Join `filename` onto `dir`, refusing anything that could escape it.
///
/// Filenames arrive from `manifest.json`, which is a plain file a user (or
/// anything else running as the user) can edit, so they are treated as hostile.
/// A single normal component is the only accepted shape: no separators, no
/// `..`, no drive letters, no root.
fn safe_cache_path(dir: &Path, filename: &str) -> Result<PathBuf> {
    if filename.is_empty() {
        anyhow::bail!("cache filename is empty");
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains(':') {
        anyhow::bail!("cache filename {filename:?} contains a path separator");
    }

    let candidate = Path::new(filename);
    let mut components = candidate.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => anyhow::bail!("cache filename {filename:?} is not a plain file name"),
    }

    Ok(dir.join(filename))
}

/// Reduce an arbitrary string to something safe to use as a file name.
///
/// Keeps ASCII alphanumerics plus `-` and `.`; everything else (including every
/// path separator, `:`, and control characters) becomes `_`. Names consisting
/// only of dots — `.` and `..` — are the traversal case and are replaced
/// outright rather than passed through.
pub fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Take only the tail after any dot-only prefix collapse, then reject
    // pure-dot names outright.
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return "cached".to_string();
    }
    // A leading dot makes a hidden file on unix and confuses extension
    // handling on Windows; not dangerous, but not wanted either.
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        "cached".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Derive the on-disk name for a cached download.
///
/// Built from the backend, package id and version rather than from the URL:
/// a URL path is entirely attacker-controlled and the last thing that should
/// ever reach `Path::join`. The URL only contributes its file extension, and
/// only after sanitising.
pub fn derive_cache_filename(url: &str, backend: &str, package_id: &str, version: &str) -> String {
    let stem = sanitize_filename(&format!("{backend}_{package_id}_{version}"));
    match url_extension(url) {
        Some(ext) if !stem.to_ascii_lowercase().ends_with(&format!(".{ext}")) => {
            format!("{stem}.{ext}")
        }
        _ => stem,
    }
}

/// Extension of the last path segment of a URL, sanitised to ASCII
/// alphanumerics and capped in length. Query strings and fragments are dropped.
fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.rsplit(['/', '\\']).next()?;
    let ext = last.rsplit_once('.')?.1;
    if ext.is_empty() || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

// ── Manifest ────────────────────────────────────────────────────────────────

impl CacheManifest {
    /// Path of the manifest file.
    ///
    /// Returns `Result` rather than panicking: this is called from Tauri
    /// commands, where an `expect` would abort the process.
    pub fn path() -> Result<PathBuf> {
        Ok(manifest_path_in(&cache_dir()?))
    }

    /// The cache directory. See [`cache_dir`].
    pub fn cache_dir() -> Result<PathBuf> {
        cache_dir()
    }

    /// Load the manifest from disk, returning an empty one if it doesn't exist
    /// or cannot be read.
    pub fn load() -> Self {
        match cache_dir() {
            Ok(dir) => Self::load_from(&dir),
            Err(e) => {
                tracing::warn!(error = %e, "offline cache unavailable; treating as empty");
                CacheManifest::default()
            }
        }
    }

    /// [`CacheManifest::load`] against an explicit directory.
    fn load_from(dir: &Path) -> Self {
        match std::fs::read_to_string(manifest_path_in(dir)) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => CacheManifest::default(),
        }
    }

    /// Async counterpart of [`CacheManifest::load`], for use on the async path.
    pub async fn load_async() -> Self {
        let dir = match cache_dir() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "offline cache unavailable; treating as empty");
                return CacheManifest::default();
            }
        };
        match fs::read_to_string(manifest_path_in(&dir)).await {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => CacheManifest::default(),
        }
    }

    /// Save the manifest to disk.
    pub fn save(&self) -> Result<()> {
        self.save_to(&cache_dir()?)
    }

    /// [`CacheManifest::save`] against an explicit directory.
    fn save_to(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(manifest_path_in(dir), text)?;
        Ok(())
    }

    /// Async counterpart of [`CacheManifest::save`].
    pub async fn save_async(&self) -> Result<()> {
        let dir = cache_dir()?;
        let text = serde_json::to_string_pretty(self)?;
        fs::create_dir_all(&dir).await?;
        fs::write(manifest_path_in(&dir), text).await?;
        Ok(())
    }

    /// Add an entry to the manifest and save, replacing any existing entry for
    /// the same package and backend.
    pub fn add(&mut self, entry: CacheManifestEntry) -> Result<()> {
        self.add_in(&cache_dir()?, entry)
    }

    fn add_in(&mut self, dir: &Path, entry: CacheManifestEntry) -> Result<()> {
        self.entries
            .retain(|e| e.package_id != entry.package_id || e.backend != entry.backend);
        self.entries.push(entry);
        self.save_to(dir)
    }

    /// Remove an entry from the manifest and delete the cached file.
    pub fn remove(&mut self, package_id: &str, backend: &str) -> Result<()> {
        self.remove_in(&cache_dir()?, package_id, backend)
    }

    fn remove_in(&mut self, dir: &Path, package_id: &str, backend: &str) -> Result<()> {
        let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.package_id == package_id && e.backend == backend)
        else {
            return Ok(());
        };

        let entry = self.entries.remove(pos);
        // A manifest entry naming `..\..\something` must drop the index entry
        // without touching the filesystem, not delete an arbitrary file.
        match safe_cache_path(dir, &entry.filename) {
            Ok(path) => {
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
            }
            Err(e) => {
                tracing::warn!(
                    filename = %entry.filename,
                    error = %e,
                    "refusing to delete a cache entry with an unsafe filename"
                );
            }
        }
        self.save_to(dir)
    }

    /// Find an entry by package_id and backend.
    pub fn find(&self, package_id: &str, backend: &str) -> Option<&CacheManifestEntry> {
        self.entries
            .iter()
            .find(|e| e.package_id == package_id && e.backend == backend)
    }

    /// Total size claimed by the manifest entries.
    ///
    /// This is the *recorded* size. For what the cache actually occupies on
    /// disk, use [`total_size_bytes`].
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size_bytes).sum()
    }

    /// Drop entries whose backing file is gone, returning how many were
    /// dropped. Does not touch the filesystem beyond `stat` and the save.
    fn prune_missing_in(&mut self, dir: &Path) -> Result<usize> {
        let before = self.entries.len();
        self.entries.retain(|e| {
            safe_cache_path(dir, &e.filename)
                .map(|p| p.exists())
                .unwrap_or(false)
        });
        let dropped = before - self.entries.len();
        if dropped > 0 {
            self.save_to(dir)?;
        }
        Ok(dropped)
    }

    /// Clear all entries and delete all cached files.
    pub fn clear(&mut self) -> Result<()> {
        self.clear_in(&cache_dir()?)
    }

    fn clear_in(&mut self, dir: &Path) -> Result<()> {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir)?;
        self.entries.clear();
        self.save_to(dir)
    }
}

// ── Disk usage / maintenance ────────────────────────────────────────────────

/// Bytes the cache actually occupies on disk.
///
/// Sums the real file sizes rather than the manifest's recorded ones, so a
/// half-written download or an orphaned file still shows up in the UI. Returns
/// 0 when the directory cannot be read.
pub fn total_size_bytes() -> u64 {
    let Ok(dir) = cache_dir() else {
        return 0;
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return 0;
    };
    read.filter_map(|e| e.ok())
        .filter(|e| e.file_name() != MANIFEST_FILENAME)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Remove manifest entries whose cached file no longer exists.
///
/// Returns the number of entries dropped.
pub async fn prune_missing() -> Result<usize> {
    let dir = cache_dir()?;
    tokio::task::spawn_blocking(move || {
        let mut manifest = CacheManifest::load_from(&dir);
        manifest.prune_missing_in(&dir)
    })
    .await?
}

/// Look up a cached entry by package and backend.
pub async fn cached_entry(package_id: &str, backend: &str) -> Option<CacheEntry> {
    CacheManifest::load_async()
        .await
        .find(package_id, backend)
        .cloned()
}

// ── Download ────────────────────────────────────────────────────────────────

/// Reject a download URL that is not something we are willing to fetch.
///
/// This is the only place Odysync downloads an executable itself, and the URL
/// arrives from the front-end, so it is untrusted input.
///
/// **HTTPS only.** A plaintext installer download is trivially replaced in
/// transit, and "we hash it afterwards" does not help when the hash is not
/// known in advance — which it usually is not, since `expected_sha256` is
/// optional. Refusing `http://` is the difference between a MITM having to
/// break TLS and a MITM having to be on the same coffee-shop network.
///
/// `file://` and every other scheme are refused too: this function's job is
/// fetching from the network, and a `file://` URL here would be a confusing
/// way to copy a local file into the cache with a "downloaded" label.
fn check_download_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    let scheme = trimmed
        .split_once("://")
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();

    match scheme.as_str() {
        "https" => {}
        "http" => anyhow::bail!(
            "refusing to download an installer over plaintext HTTP: {trimmed}. \
             An installer fetched over http:// can be replaced in transit."
        ),
        "" => anyhow::bail!("not a URL: {trimmed:?}"),
        other => anyhow::bail!("refusing to download from an unsupported scheme {other:?}"),
    }

    // "https://" with nothing after it, or with only a slash, is not a host.
    let rest = &trimmed[scheme.len() + 3..];
    if rest.is_empty() || rest.starts_with('/') {
        anyhow::bail!("URL has no host: {trimmed}");
    }

    Ok(())
}

/// Download a file from a URL and cache it, verifying it before it is kept.
///
/// Three checks, in the order that makes each one meaningful:
///
///   1. **The URL** must be HTTPS — see [`check_download_url`].
///   2. **The digest**, when the caller supplied one. A mismatch means the
///      bytes are not what was promised and nothing else matters.
///   3. **The code signature**, via `odysync-verify`. An installer with a
///      *broken* signature is refused outright: unsigned is a state the world
///      is genuinely in, but a signature that fails to validate means the file
///      was tampered with after signing, or its certificate was revoked.
///
/// The file is written to a temporary name and only moved into place once it
/// passes, so a rejected download cannot be left sitting in the cache looking
/// like a legitimate entry.
pub async fn download_and_cache(
    url: &str,
    package_id: &str,
    backend: &str,
    version: &str,
    expected_sha256: Option<&str>,
    proxy_url: Option<&str>,
) -> Result<CacheManifestEntry> {
    use sha2::{Digest, Sha256};

    check_download_url(url)?;

    let mut client_builder =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));
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

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = hex::encode(hasher.finalize());

    if let Some(expected) = expected_sha256 {
        let expected = expected.trim();
        // A malformed expected digest used to pass silently: `eq_ignore_ascii_case`
        // against "deadbeef" simply returned false and produced a mismatch
        // error, but against an empty string it would have compared against
        // nothing. Reject the shape explicitly so a caller bug reads as a
        // caller bug.
        if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("expected digest is not a SHA-256 hex string: {expected:?}");
        }
        if !sha256.eq_ignore_ascii_case(expected) {
            anyhow::bail!("SHA256 mismatch: expected {expected}, got {sha256}");
        }
    }

    let dir = cache_dir()?;
    let filename = derive_cache_filename(url, backend, package_id, version);
    // Belt and braces: the derivation already sanitises, but the join is the
    // thing that would actually escape, so it is checked at the join.
    let file_path = safe_cache_path(&dir, &filename)?;

    // Write to a sibling temporary name first. The signature check needs the
    // bytes on disk — Authenticode is a file-level check — and a file that
    // fails it must never have existed at the real path, or a later run would
    // find it in the cache directory and treat it as vetted.
    let staging = safe_cache_path(&dir, &format!("{filename}.partial"))?;
    let mut file = fs::File::create(&staging).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    drop(file);

    let signature = match odysync_verify::verify_signature(&staging).await {
        odysync_verify::SignatureStatus::Valid { subject } => {
            tracing::info!(package = package_id, %subject, "cached installer is signed");
            CachedSignature::Valid { subject }
        }
        odysync_verify::SignatureStatus::Unsigned => {
            // Not fatal. Plenty of legitimate installers are unsigned, and
            // refusing them would make the cache useless. It is recorded, so
            // the UI can say so rather than implying a check that passed.
            tracing::warn!(package = package_id, "cached installer is unsigned");
            CachedSignature::Unsigned
        }
        odysync_verify::SignatureStatus::Invalid { detail } => {
            let _ = fs::remove_file(&staging).await;
            anyhow::bail!(
                "refusing to cache {package_id}: its code signature does not validate ({detail}). \
                 A broken signature means the file changed after it was signed, or the \
                 certificate was revoked."
            );
        }
        odysync_verify::SignatureStatus::Unsupported => CachedSignature::Unknown,
    };

    fs::rename(&staging, &file_path).await?;

    let entry = CacheManifestEntry {
        package_id: package_id.to_string(),
        backend: backend.to_string(),
        version: version.to_string(),
        filename,
        sha256,
        size_bytes,
        cached_at: chrono::Utc::now().to_rfc3339(),
        signature,
    };

    let mut manifest = CacheManifest::load_async().await;
    manifest
        .entries
        .retain(|e| e.package_id != entry.package_id || e.backend != entry.backend);
    manifest.entries.push(entry.clone());
    manifest.save_async().await?;

    tracing::info!(
        package = package_id,
        backend,
        size = size_bytes,
        "cached offline installer"
    );

    Ok(entry)
}

/// Path to a cached installer file.
///
/// Errors when `filename` is not a plain file name — see [`safe_cache_path`].
pub fn cached_file_path(filename: &str) -> Result<PathBuf> {
    safe_cache_path(&cache_dir()?, filename)
}

/// Verify a cached file's integrity by checking its SHA256.
pub async fn verify_cached_file(entry: &CacheManifestEntry) -> Result<bool> {
    use sha2::{Digest, Sha256};

    let path = match cached_file_path(&entry.filename) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(filename = %entry.filename, error = %e, "unsafe cache filename");
            return Ok(false);
        }
    };

    if fs::metadata(&path).await.is_err() {
        return Ok(false);
    }

    let data = fs::read(&path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(hasher.finalize());

    Ok(hash.eq_ignore_ascii_case(&entry.sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Download URL hardening ──────────────────────────────────────────────

    #[test]
    fn https_urls_are_accepted() {
        assert!(check_download_url("https://example.com/setup.exe").is_ok());
        assert!(check_download_url("  https://example.com/a.msi  ").is_ok());
        // Scheme comparison is case-insensitive; URLs are not case-normalised
        // before they reach us.
        assert!(check_download_url("HTTPS://example.com/setup.exe").is_ok());
    }

    /// The check that matters. An installer fetched over plaintext can be
    /// swapped in transit, and `expected_sha256` is optional, so there is
    /// frequently nothing to catch the swap afterwards.
    #[test]
    fn plaintext_http_is_refused() {
        let err = check_download_url("http://example.com/setup.exe").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("plaintext"), "unhelpful message: {msg}");
        assert!(msg.contains("http://example.com/setup.exe"));
    }

    #[test]
    fn other_schemes_and_malformed_urls_are_refused() {
        for url in [
            "file:///C:/Windows/System32/calc.exe",
            "ftp://example.com/setup.exe",
            "javascript:alert(1)",
            "example.com/setup.exe",
            "",
            "https://",
            "https:///no-host",
        ] {
            assert!(
                check_download_url(url).is_err(),
                "should have been refused: {url:?}"
            );
        }
    }

    // ── Manifest compatibility ──────────────────────────────────────────────

    /// A manifest written before signatures were recorded must still load, and
    /// must read as `Unknown` rather than as anything reassuring.
    #[test]
    fn an_older_manifest_entry_loads_with_an_unknown_signature() {
        let json = r#"{
            "package_id": "Mozilla.Firefox",
            "backend": "winget",
            "version": "1.0.0",
            "filename": "firefox.exe",
            "sha256": "abc",
            "size_bytes": 1,
            "cached_at": "2024-01-01T00:00:00Z"
        }"#;
        let entry: CacheManifestEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.signature, CachedSignature::Unknown);
    }

    #[test]
    fn a_signature_verdict_round_trips_through_the_manifest() {
        for original in [
            CachedSignature::Unknown,
            CachedSignature::Unsigned,
            CachedSignature::Valid {
                subject: "CN=Mozilla Corporation".into(),
            },
            CachedSignature::Invalid {
                detail: "chain broken".into(),
            },
        ] {
            let text = serde_json::to_string(&original).unwrap();
            let back: CachedSignature = serde_json::from_str(&text).unwrap();
            assert_eq!(back, original);
        }
    }

    fn entry(package_id: &str, backend: &str, filename: &str, size: u64) -> CacheManifestEntry {
        CacheManifestEntry {
            package_id: package_id.to_string(),
            backend: backend.to_string(),
            version: "1.0.0".to_string(),
            filename: filename.to_string(),
            sha256: "abc123".to_string(),
            size_bytes: size,
            cached_at: "2024-01-01T00:00:00Z".to_string(),
            signature: CachedSignature::Unknown,
        }
    }

    #[test]
    fn manifest_round_trips() {
        let mut manifest = CacheManifest::default();
        manifest
            .entries
            .push(entry("test.pkg", "winget", "winget_test_pkg_1_0_0", 1024));
        let json = serde_json::to_string(&manifest).unwrap();
        let loaded: CacheManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].package_id, "test.pkg");
    }

    #[test]
    fn find_entry_by_package_and_backend() {
        let mut manifest = CacheManifest::default();
        manifest.entries.push(entry("pkg.a", "winget", "a", 100));
        manifest.entries.push(entry("pkg.b", "pip", "b", 200));

        assert!(manifest.find("pkg.a", "winget").is_some());
        assert!(manifest.find("pkg.a", "pip").is_none());
        assert!(manifest.find("pkg.b", "pip").is_some());
        assert!(manifest.find("pkg.c", "winget").is_none());
    }

    #[test]
    fn total_size_sums_all_entries() {
        let mut manifest = CacheManifest::default();
        manifest.entries.push(entry("a", "x", "a", 100));
        manifest.entries.push(entry("b", "y", "b", 300));
        assert_eq!(manifest.total_size(), 400);
    }

    // ── filename derivation ─────────────────────────────────────────────────

    #[test]
    fn derived_filenames_are_plain_names() {
        let name = derive_cache_filename(
            "https://example.com/setup.exe",
            "winget",
            "Vendor.Tool",
            "1.2.3",
        );
        assert_eq!(name, "winget_Vendor.Tool_1.2.3.exe");
        assert!(safe_cache_path(Path::new("/cache"), &name).is_ok());
    }

    #[test]
    fn traversal_attempts_never_escape_the_cache_directory() {
        let dir = Path::new("/cache");
        let hostile = [
            "../../evil.exe",
            r"..\..\Windows\System32\evil.dll",
            "/etc/passwd",
            r"C:\Windows\System32\evil.dll",
            "..",
            ".",
            "",
            "sub/dir/file",
            "stream.exe:ads",
        ];
        for raw in hostile {
            // Anything hostile is rejected outright at the join...
            assert!(
                safe_cache_path(dir, raw).is_err(),
                "safe_cache_path accepted {raw:?}"
            );
            // ...and sanitising it produces a name that IS accepted and stays
            // directly inside the cache directory.
            let safe = sanitize_filename(raw);
            let path = safe_cache_path(dir, &safe)
                .unwrap_or_else(|e| panic!("sanitize_filename({raw:?}) -> {safe:?}: {e}"));
            assert_eq!(path.parent(), Some(dir), "{raw:?} escaped via {safe:?}");
        }
    }

    #[test]
    fn traversal_in_package_metadata_is_neutralised() {
        for (backend, package_id, version) in [
            ("..", "..", ".."),
            (r"..\..", "evil", "1.0"),
            ("winget", "../../../../etc/passwd", "1.0"),
            ("", "", ""),
        ] {
            let name = derive_cache_filename("https://x/y", backend, package_id, version);
            let path = safe_cache_path(Path::new("/cache"), &name)
                .unwrap_or_else(|e| panic!("{backend}/{package_id}/{version} -> {name:?}: {e}"));
            assert_eq!(path.parent(), Some(Path::new("/cache")));
        }
    }

    #[test]
    fn url_supplied_extension_cannot_carry_a_path() {
        // A URL whose "extension" is junk contributes nothing.
        for url in [
            "https://example.com/a.exe/../../evil",
            "https://example.com/file.",
            "https://example.com/file.verylongextension",
            "https://example.com/noext",
            "https://example.com/a.ex e",
        ] {
            let name = derive_cache_filename(url, "winget", "Pkg", "1.0");
            assert!(safe_cache_path(Path::new("/cache"), &name).is_ok(), "{url}");
        }
        // Query strings and fragments are dropped before the extension is read.
        assert_eq!(
            derive_cache_filename("https://x/setup.msi?token=a/b", "w", "P", "1"),
            "w_P_1.msi"
        );
    }

    // ── manifest round-trips on disk ────────────────────────────────────────

    #[test]
    fn add_and_remove_round_trip_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("winget_pkg_1"), b"payload").unwrap();

        let mut manifest = CacheManifest::default();
        manifest
            .add_in(dir, entry("pkg", "winget", "winget_pkg_1", 7))
            .unwrap();

        let reloaded = CacheManifest::load_from(dir);
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].filename, "winget_pkg_1");

        let mut reloaded = reloaded;
        reloaded.remove_in(dir, "pkg", "winget").unwrap();
        assert!(reloaded.entries.is_empty());
        assert!(!dir.join("winget_pkg_1").exists());
        assert!(CacheManifest::load_from(dir).entries.is_empty());
    }

    #[test]
    fn add_replaces_an_entry_for_the_same_package_and_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let mut manifest = CacheManifest::default();
        manifest
            .add_in(dir, entry("pkg", "winget", "old", 1))
            .unwrap();
        manifest
            .add_in(dir, entry("pkg", "winget", "new", 2))
            .unwrap();
        manifest
            .add_in(dir, entry("pkg", "pip", "other", 3))
            .unwrap();

        let reloaded = CacheManifest::load_from(dir);
        assert_eq!(reloaded.entries.len(), 2);
        assert_eq!(reloaded.find("pkg", "winget").unwrap().filename, "new");
    }

    #[test]
    fn remove_refuses_to_delete_outside_the_cache_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, b"do not delete me").unwrap();

        let dir = tmp.path().join("cache");
        std::fs::create_dir_all(&dir).unwrap();

        // A hand-edited manifest.json pointing at a file outside the cache.
        let mut manifest = CacheManifest::default();
        manifest
            .entries
            .push(entry("evil", "winget", "../outside.txt", 1));
        manifest.save_to(&dir).unwrap();

        manifest.remove_in(&dir, "evil", "winget").unwrap();

        assert!(manifest.entries.is_empty(), "index entry should be dropped");
        assert!(outside.exists(), "file outside the cache was deleted");
    }

    #[test]
    fn prune_drops_only_entries_whose_file_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("present"), b"x").unwrap();

        let mut manifest = CacheManifest::default();
        manifest.entries.push(entry("a", "winget", "present", 1));
        manifest.entries.push(entry("b", "winget", "gone", 1));
        manifest.entries.push(entry("c", "winget", "../escape", 1));
        manifest.save_to(dir).unwrap();

        assert_eq!(manifest.prune_missing_in(dir).unwrap(), 2);
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].package_id, "a");

        // Persisted, and a second prune is a no-op.
        let mut reloaded = CacheManifest::load_from(dir);
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.prune_missing_in(dir).unwrap(), 0);
    }

    #[test]
    fn clear_empties_the_directory_and_the_index() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), b"aaa").unwrap();

        let mut manifest = CacheManifest::default();
        manifest.add_in(&dir, entry("a", "winget", "a", 3)).unwrap();
        manifest.clear_in(&dir).unwrap();

        assert!(manifest.entries.is_empty());
        assert!(!dir.join("a").exists());
        assert!(CacheManifest::load_from(&dir).entries.is_empty());
    }

    #[test]
    fn load_from_a_missing_or_corrupt_manifest_yields_an_empty_one() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(CacheManifest::load_from(tmp.path()).entries.is_empty());
        std::fs::write(tmp.path().join(MANIFEST_FILENAME), b"{not json").unwrap();
        assert!(CacheManifest::load_from(tmp.path()).entries.is_empty());
    }
}
