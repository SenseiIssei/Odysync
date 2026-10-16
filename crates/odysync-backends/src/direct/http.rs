//! HTTP for the direct backend: HTTPS only, conditional requests, streamed
//! downloads.
//!
//! Three properties matter more than the rest:
//!
//! * **HTTPS on every hop.** The redirect policy refuses a redirect to
//!   `http://`, so a download that starts on `https://github.com` cannot be
//!   bounced onto plaintext where it could be swapped in transit.
//! * **Conditional requests for metadata.** GitHub allows 60 unauthenticated
//!   API calls an hour. A scan touches one release per installed catalog app,
//!   and a `304 Not Modified` answer to an `If-None-Match` request does not
//!   count against that limit — so repeated scans are effectively free.
//! * **A hard size cap on downloads**, checked while streaming, so a hostile or
//!   broken server cannot fill the disk.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A client that will only ever speak HTTPS, including across redirects.
pub fn client(proxy_url: Option<&str>, timeout: Duration) -> Result<reqwest::Client> {
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

/// The directory installers are downloaded into before they are verified.
pub fn download_dir() -> Result<PathBuf> {
    let dir = cache_root()?.join("downloads");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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

/// GET `url` as text, revalidating a cached copy instead of re-downloading it.
///
/// A network failure with a cached copy on disk returns the cached copy: a
/// stale "latest version" is still a correct one to compare against, and the
/// download it leads to is verified independently anyway.
pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    if !url.starts_with("https://") {
        return Err(Error::security(
            "direct",
            format!("refusing a non-HTTPS URL: {url}"),
        ));
    }
    let cache_path = cache_file_for(url).ok();
    let cached: Option<CachedResponse> = cache_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok());

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
    if let Some(c) = &cached {
        if let Some(etag) = &c.etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(lm) = &c.last_modified {
            request = request.header("If-Modified-Since", lm);
        }
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return match cached {
                Some(c) => {
                    tracing::debug!(url, error = %e, "network error; using the cached response");
                    Ok(c.body)
                }
                None => Err(other(format!("could not fetch {url}: {e}"))),
            };
        }
    };

    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED
        && let Some(c) = cached
    {
        return Ok(c.body);
    }
    if !status.is_success() {
        if let Some(c) = cached {
            tracing::debug!(url, %status, "server error; using the cached response");
            return Ok(c.body);
        }
        let hint = if status == reqwest::StatusCode::FORBIDDEN && url.contains("api.github.com") {
            " (GitHub's hourly limit for anonymous requests; set GITHUB_TOKEN to raise it)"
        } else {
            ""
        };
        return Err(other(format!("{url} answered {status}{hint}")));
    }

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let etag = header("etag");
    let last_modified = header("last-modified");
    let body = response
        .text()
        .await
        .map_err(|e| other(format!("could not read {url}: {e}")))?;

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

/// GET `url` and parse it as JSON.
pub async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    let text = get_text(client, url).await?;
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
