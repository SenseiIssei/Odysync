//! HTTP for the direct backend: HTTPS only, conditional requests, streamed
//! downloads.
//!
//! Three properties matter more than the rest:
//!
//! * **HTTPS on every hop.** The redirect policy refuses a redirect to
//!   `http://`, so a download that starts on `https://github.com` cannot be
//!   bounced onto plaintext where it could be swapped in transit.
//! * **A short memory for metadata.** GitHub allows 60 unauthenticated API
//!   calls an hour, and a scan makes one per installed catalog app. A
//!   `304 Not Modified` counts against that limit unless the request carries a
//!   token (measured: three of them, three calls gone), so a scan reuses an
//!   answer that is a few minutes old instead of asking again. Installing
//!   always asks afresh and never falls back to what is on disk.
//! * **A hard size cap on downloads**, checked while streaming, so a hostile or
//!   broken server cannot fill the disk.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use odysync_core::error::{Error, Result};

const USER_AGENT: &str = concat!(
    "Odysync/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/SenseiIssei/Odysync)"
);

/// Metadata requests are small; a slow answer is a broken answer.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);

/// Installers can be large, and home connections slow.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Nothing in the catalog is anywhere near this; a GPU driver is about 1 GB.
pub const MAX_DOWNLOAD_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// Network and server failures. Reported as transient so the runner's retry
/// logic treats them like any other flaky connection.
fn other(msg: impl Into<String>) -> Error {
    Error::Transient {
        backend: "direct".into(),
        detail: msg.into(),
        attempt: 0,
    }
}

/// How old an answer a scan may reuse without asking again.
const SCAN_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// How old an answer may stand in for one that cannot be had at all. Older
/// than this it says too little about what the publisher offers today.
const STALE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The proxy from the settings, for every client that is not given its own.
static PROXY: RwLock<Option<String>> = RwLock::new(None);

/// Use `proxy_url` for all downloads and lookups from here on.
///
/// The backends are built without a configuration; the setting used to be
/// shown, saved, and never reach any of them.
pub fn set_proxy(proxy_url: Option<&str>) {
    let value = proxy_url
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    *PROXY.write().unwrap_or_else(|p| p.into_inner()) = value;
}

/// What a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Listing what is available: a recent answer is good enough, and an older
    /// one better than none.
    Scan,
    /// Deciding what to install: only what the publisher says right now.
    Install,
}

/// A client that will only ever speak HTTPS, including across redirects.
pub fn client(proxy_url: Option<&str>, timeout: Duration) -> Result<reqwest::Client> {
    let configured = PROXY.read().unwrap_or_else(|p| p.into_inner()).clone();
    let proxy_url = proxy_url.or(configured.as_deref());
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 10 {
            attempt.error("too many redirects")
        } else if attempt.url().scheme() != "https" {
            attempt.error("refusing a redirect away from HTTPS")
        } else {
            attempt.follow()
        }
    });

    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(policy)
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout);
    if let Some(proxy) = proxy_url.filter(|p| !p.trim().is_empty()) {
        match reqwest::Proxy::all(proxy) {
            Ok(proxy) => builder = builder.proxy(proxy),
            Err(e) => tracing::warn!(error = %e, "ignoring an invalid proxy URL"),
        }
    }
    builder
        .build()
        .map_err(|e| other(format!("could not build an HTTP client: {e}")))
}

/// Where conditional-request state and downloads live.
fn cache_root() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| other("could not resolve the user cache directory"))?;
    Ok(dirs.cache_dir().join("direct"))
}

/// A fresh, empty directory for one download, removed again when dropped.
///
/// The installer runs from here. When Odysync is elevated the directory is
/// one that only administrators can write to: locking the installer file stops
/// anyone from swapping *it*, but a program looks for its libraries next to
/// itself first, and an unprivileged process could put a `version.dll` beside
/// an installer that is about to run with administrator rights.
pub struct DownloadDir(PathBuf);

impl DownloadDir {
    pub fn new(label: &str) -> Result<Self> {
        #[cfg(windows)]
        if odysync_core::platform::is_elevated() {
            return super::private_dir::create(label).map(Self);
        }

        let root = cache_root()?.join("downloads");
        std::fs::create_dir_all(&root)?;
        remove_leftovers(&root, "");
        let dir = root.join(format!(
            "{label}-{}",
            jiff::Timestamp::now().as_millisecond()
        ));
        // Not `create_dir_all`: a directory that is already there is not ours.
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for DownloadDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            tracing::debug!(dir = %self.0.display(), error = %e, "download not removed yet");
        }
    }
}

/// Remove what an interrupted update left behind: directories in `root` that
/// start with `prefix` and have not been touched for a day.
pub(super) fn remove_leftovers(root: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE_MAX_AGE);
        if old
            && entry.file_name().to_string_lossy().starts_with(prefix)
            && entry.file_type().is_ok_and(|t| t.is_dir())
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CachedResponse {
    etag: Option<String>,
    last_modified: Option<String>,
    body: String,
}

fn cache_file_for(url: &str) -> Result<PathBuf> {
    let key = hex::encode(Sha256::digest(url.as_bytes()));
    Ok(cache_root()?.join("http").join(format!("{key}.json")))
}

/// The stored answer for `url` and how old it is.
fn read_cached(path: &Path) -> Option<(CachedResponse, Duration)> {
    let text = std::fs::read_to_string(path).ok()?;
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let age = SystemTime::now()
        .duration_since(modified)
        // Written "in the future": the clock was changed, so its age is
        // unknown and it counts as old.
        .unwrap_or(Duration::MAX);
    Some((serde_json::from_str(&text).ok()?, age))
}

/// GET `url` as text for listing what is available.
pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    get_text_for(client, url, Purpose::Scan).await
}

/// GET `url` as text.
///
/// For [`Purpose::Scan`] an answer from the last few minutes is reused, an
/// older one is revalidated, and one from the last day stands in when the
/// network fails: a slightly stale "latest version" is still a version to
/// compare against. For [`Purpose::Install`] nothing on disk is consulted.
pub async fn get_text_for(client: &reqwest::Client, url: &str, purpose: Purpose) -> Result<String> {
    if !url.starts_with("https://") {
        return Err(Error::security(
            "direct",
            format!("refusing a non-HTTPS URL: {url}"),
        ));
    }
    let cache_path = cache_file_for(url).ok();
    let cached = match purpose {
        Purpose::Scan => cache_path.as_deref().and_then(read_cached),
        Purpose::Install => None,
    };
    if let Some((c, age)) = &cached
        && *age < SCAN_MAX_AGE
    {
        return Ok(c.body.clone());
    }
    let fallback = |why: String| match &cached {
        Some((c, age)) if *age < STALE_MAX_AGE => {
            tracing::info!(url, why, "using the last known answer");
            Ok(c.body.clone())
        }
        _ => Err(other(why)),
    };

    let mut request = client.get(url).timeout(METADATA_TIMEOUT);
    if url.starts_with("https://api.github.com/") {
        request = request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        // A token, when the user has one, raises the limit from 60 to 5 000
        // requests an hour. Only ever sent to api.github.com.
        if let Ok(token) = std::env::var("GITHUB_TOKEN")
            && !token.trim().is_empty()
        {
            request = request.bearer_auth(token.trim());
        }
    }
    if let Some((c, _)) = &cached {
        if let Some(etag) = &c.etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(lm) = &c.last_modified {
            request = request.header("If-Modified-Since", lm);
        }
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => return fallback(format!("could not fetch {url}: {e}")),
    };

    let status = response.status();
    let (etag, last_modified, body) = if status == reqwest::StatusCode::NOT_MODIFIED
        && let Some((c, _)) = cached
    {
        (c.etag, c.last_modified, c.body)
    } else if status.is_success() {
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let (etag, last_modified) = (header("etag"), header("last-modified"));
        let body = response
            .text()
            .await
            .map_err(|e| other(format!("could not read {url}: {e}")))?;
        (etag, last_modified, body)
    } else {
        let hint = if status == reqwest::StatusCode::FORBIDDEN && url.contains("api.github.com") {
            " (GitHub's hourly limit for anonymous requests; set GITHUB_TOKEN to raise it)"
        } else {
            ""
        };
        return fallback(format!("{url} answered {status}{hint}"));
    };

    // Written after a 304 as well, so that the answer counts as fresh again.
    if let Some(path) = cache_path {
        let entry = CachedResponse {
            etag,
            last_modified,
            body: body.clone(),
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&entry) {
            let _ = std::fs::write(&path, text);
        }
    }

    Ok(body)
}

/// GET `url` and parse it as JSON, for listing what is available.
pub async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    get_json_for(client, url, Purpose::Scan).await
}

/// GET `url` and parse it as JSON.
pub async fn get_json_for(
    client: &reqwest::Client,
    url: &str,
    purpose: Purpose,
) -> Result<serde_json::Value> {
    let text = get_text_for(client, url, purpose).await?;
    serde_json::from_str(&text).map_err(|e| other(format!("{url} did not return JSON: {e}")))
}

/// Stream `url` to `dest`, returning the SHA-256 of what was written.
///
/// `on_progress` receives `(bytes so far, total if known)`. The file is
/// removed again if anything fails, so a half-written installer is never left
/// where it could be mistaken for a finished one.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<String> {
    let result = async {
        let mut response = client
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| other(format!("download failed: {e}")))?;
        if !response.status().is_success() {
            return Err(other(format!(
                "download answered {} for {url}",
                response.status()
            )));
        }
        let total = response.content_length();
        if total.is_some_and(|t| t > MAX_DOWNLOAD_BYTES) {
            return Err(other("the download is larger than Odysync accepts"));
        }

        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| other(format!("download interrupted: {e}")))?
        {
            written += chunk.len() as u64;
            if written > MAX_DOWNLOAD_BYTES {
                return Err(other("the download exceeded the size limit"));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            on_progress(written, total);
        }
        file.flush().await?;
        if let Some(total) = total
            && written != total
        {
            return Err(other(format!(
                "the download ended early ({written} of {total} bytes)"
            )));
        }
        Ok(hex::encode(hasher.finalize()))
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// A safe local file name for a download, derived from its URL.
pub fn file_name_for(url: &str, fallback: &str) -> String {
    let last = url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .unwrap_or("");
    let decoded = last.replace("%20", " ");
    let cleaned: String = decoded
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_file_names_are_reduced_to_a_safe_leaf() {
        assert_eq!(
            file_name_for(
                "https://github.com/o/r/releases/download/v1/npp.8.9.8.1.Installer.x64.exe",
                "x"
            ),
            "npp.8.9.8.1.Installer.x64.exe"
        );
        assert_eq!(
            file_name_for("https://h/a/Firefox%20Setup%201.0.exe?x=1", "x"),
            "Firefox Setup 1.0.exe"
        );
        // Traversal and separators cannot survive.
        assert_eq!(
            file_name_for("https://h/..%2F..%2Fevil.exe", "x"),
            "..2F..2Fevil.exe".trim_start_matches('.')
        );
        assert_eq!(file_name_for("https://h/", "fallback.bin"), "fallback.bin");
        assert_eq!(
            file_name_for("https://h/...", "fallback.bin"),
            "fallback.bin"
        );
    }

    #[test]
    fn a_stored_answer_knows_its_age() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        assert!(read_cached(&path).is_none());
        std::fs::write(&path, r#"{"etag":null,"last_modified":null,"body":"hi"}"#).unwrap();
        let (c, age) = read_cached(&path).expect("stored");
        assert_eq!(c.body, "hi");
        assert!(age < SCAN_MAX_AGE);

        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let (_, age) = read_cached(&path).expect("stored");
        assert!(age > SCAN_MAX_AGE && age < STALE_MAX_AGE);

        // A clock that was set back must not make an answer fresh for ever.
        let future = SystemTime::now() + Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(future)
            .unwrap();
        assert_eq!(read_cached(&path).expect("stored").1, Duration::MAX);
    }

    #[test]
    fn a_download_directory_is_new_empty_and_gone_afterwards() {
        let dir = DownloadDir::new("unit-test").expect("create");
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        std::fs::write(path.join("installer.exe"), b"x").unwrap();
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn only_old_leftovers_with_the_prefix_are_removed() {
        let root = tempfile::tempdir().unwrap();
        for name in ["mine-new", "theirs"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        remove_leftovers(root.path(), "mine-");
        assert!(root.path().join("mine-new").exists(), "still in use");
        assert!(root.path().join("theirs").exists(), "not ours");
    }

    #[test]
    fn the_client_refuses_plain_http() {
        let c = client(None, Duration::from_secs(5)).expect("client");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(get_text(&c, "http://example.com/"))
            .expect_err("plain HTTP must be refused");
        assert!(err.to_string().contains("non-HTTPS"));
    }
}
