//! Hybrid version discovery system.
//!
//! Provides the latest available versions for hardware drivers and software
//! through a layered approach:
//!
//! 1. **RegistrySource** — queries a configurable online JSON API
//! 2. **VendorScraper** — scrapes vendor download pages (NVIDIA, AMD, Intel)
//! 3. **OfflineCache** — serves cached results when the network is unavailable
//!
//! The `HybridSource` orchestrates these in order and caches every successful
//! result for offline use.

mod cache;
mod hybrid;
mod registry;
mod scraper;

pub use cache::OfflineCache;
pub use hybrid::HybridSource;
pub use registry::RegistrySource;
pub use scraper::VendorScraper;

use serde::{Deserialize, Serialize};

/// Identifies a piece of hardware or software to look up versions for.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct HardwareId {
    /// Vendor: "nvidia", "amd", "intel", "qualcomm", etc.
    pub vendor: String,
    /// Device or product family: "geforce-rtx-4090", "ryzen-9-7950x", etc.
    pub device: String,
}

impl HardwareId {
    pub fn new(vendor: impl Into<String>, device: impl Into<String>) -> Self {
        Self {
            vendor: vendor.into(),
            device: device.into(),
        }
    }
}

/// Latest version info for a hardware device or software package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Latest available version string.
    pub version: String,
    /// Direct download URL for the installer/driver, if known.
    pub download_url: Option<String>,
    /// ISO 8601 release date, if known.
    pub release_date: Option<String>,
    /// Expected SHA-256 checksum of the download, if known.
    pub checksum: Option<String>,
    /// Human-readable release notes or changelog summary.
    pub notes: Option<String>,
}

/// A source of latest version information.
#[async_trait::async_trait]
pub trait VersionSource: Send + Sync {
    /// Fetch the latest version info for `id`.
    async fn fetch_latest(&self, id: &HardwareId) -> Result<VersionInfo, SourceError>;
}

/// Errors that can occur during version discovery.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("network error: {0}")]
    Network(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not found in source")]
    NotFound,
    #[error("cache error: {0}")]
    Cache(String),
}
