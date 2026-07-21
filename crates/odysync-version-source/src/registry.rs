//! Online registry source — queries a configurable JSON API endpoint.
//!
//! The default endpoint is `https://api.odysync.dev/v1/latest/{vendor}/{device}`,
//! but it can be overridden via `RegistrySource::with_url`.

use crate::{HardwareId, SourceError, VersionInfo, VersionSource};

/// Fetches latest version info from a hosted JSON API.
pub struct RegistrySource {
    base_url: String,
    client: reqwest::Client,
}

impl Default for RegistrySource {
    fn default() -> Self {
        Self::new("https://api.odysync.dev/v1/latest")
    }
}

impl RegistrySource {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Use a custom base URL (e.g. a self-hosted registry).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }
}

#[async_trait::async_trait]
impl VersionSource for RegistrySource {
    async fn fetch_latest(&self, id: &HardwareId) -> Result<VersionInfo, SourceError> {
        let url = format!("{}/{}/{}", self.base_url, id.vendor, id.device);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound);
        }

        if !resp.status().is_success() {
            return Err(SourceError::Network(format!(
                "registry returned {}",
                resp.status()
            )));
        }

        resp.json::<VersionInfo>()
            .await
            .map_err(|e| SourceError::Parse(format!("registry response parse: {e}")))
    }
}
