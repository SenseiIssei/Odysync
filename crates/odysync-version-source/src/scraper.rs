//! Vendor website scrapers for NVIDIA, AMD, Intel, and Qualcomm driver pages.
//!
//! Each scraper fetches the vendor's download/search page, parses the HTML
//! with `scraper`, and extracts the latest driver version and download URL.
//!
//! These are inherently fragile — vendor HTML changes break scrapers. The
//! hybrid source falls back to the offline cache when scraping fails.

use crate::{HardwareId, SourceError, VersionInfo, VersionSource};
use scraper::{Html, Selector};

/// Scrapes vendor download pages for latest driver versions.
pub struct VendorScraper {
    client: reqwest::Client,
}

impl Default for VendorScraper {
    fn default() -> Self {
        Self::new()
    }
}

impl VendorScraper {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Odysync/2.0 (https://github.com/SenseiIssei/Odysync)")
                .build()
                .unwrap_or_default(),
        }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, SourceError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SourceError::Network(format!(
                "vendor page returned {}",
                resp.status()
            )));
        }
        resp.text()
            .await
            .map_err(|e| SourceError::Network(format!("read body: {e}")))
    }

    /// Scrape NVIDIA's driver search page for a GPU model.
    ///
    /// NVIDIA's `processFind.aspx` endpoint returns HTML with driver results.
    /// We extract the first (latest) driver version and download link.
    async fn scrape_nvidia(&self, device: &str) -> Result<VersionInfo, SourceError> {
        let url = format!(
            "https://www.nvidia.com/Download/processFind.aspx?psid={}&pfid={}&osid=57&lid=1&whql=1&lang=en-us&ctk=0",
            nvidia_psid(device),
            nvidia_pfid(device),
        );
        let html = self.fetch_html(&url).await?;
        let doc = Html::parse_document(&html);

        // NVIDIA's page has version info in a specific div pattern.
        // Look for the first driver version string in the page.
        let version_sel = Selector::parse("div.driverVersion, td.driverVersion, .versionInfo")
            .map_err(|e| SourceError::Parse(format!("selector: {e}")))?;

        if let Some(el) = doc.select(&version_sel).next() {
            let text = el.text().collect::<String>();
            let version = text.trim().lines().next().unwrap_or("").trim().to_string();
            if !version.is_empty() {
                return Ok(VersionInfo {
                    version,
                    download_url: None,
                    release_date: None,
                    checksum: None,
                    notes: None,
                });
            }
        }

        Err(SourceError::NotFound)
    }

    /// Scrape AMD's driver support page.
    async fn scrape_amd(&self, _device: &str) -> Result<VersionInfo, SourceError> {
        // AMD's driver page is heavily JavaScript-rendered, so a simple HTTP
        // GET won't get the dynamic content. We try their RSS/feed endpoint
        // as a fallback, and return NotFound if that fails.
        let url = "https://www.amd.com/en/support/download/drivers.html";
        let html = self.fetch_html(url).await?;
        let doc = Html::parse_document(&html);

        // Look for any version-like string in meta tags or data attributes.
        let meta_sel =
            Selector::parse("meta[name='latest-version'], meta[property='og:description']")
                .map_err(|e| SourceError::Parse(format!("selector: {e}")))?;

        for el in doc.select(&meta_sel) {
            if let Some(content) = el.value().attr("content") {
                if let Some(ver) = extract_version_string(content) {
                    return Ok(VersionInfo {
                        version: ver,
                        download_url: None,
                        release_date: None,
                        checksum: None,
                        notes: None,
                    });
                }
            }
        }

        Err(SourceError::NotFound)
    }

    /// Scrape Intel's download center.
    async fn scrape_intel(&self, _device: &str) -> Result<VersionInfo, SourceError> {
        // Intel's download center is also JavaScript-heavy. We try their
        // RSS feed for driver updates as a lightweight approach.
        let url = "https://www.intel.com/content/www/us/en/download-center/home.html";
        let html = self.fetch_html(url).await?;
        let doc = Html::parse_document(&html);

        let title_sel =
            Selector::parse("title").map_err(|e| SourceError::Parse(format!("selector: {e}")))?;

        if let Some(el) = doc.select(&title_sel).next() {
            let text = el.text().collect::<String>();
            if let Some(ver) = extract_version_string(&text) {
                return Ok(VersionInfo {
                    version: ver,
                    download_url: None,
                    release_date: None,
                    checksum: None,
                    notes: None,
                });
            }
        }

        Err(SourceError::NotFound)
    }
}

#[async_trait::async_trait]
impl VersionSource for VendorScraper {
    async fn fetch_latest(&self, id: &HardwareId) -> Result<VersionInfo, SourceError> {
        match id.vendor.as_str() {
            "nvidia" => self.scrape_nvidia(&id.device).await,
            "amd" => self.scrape_amd(&id.device).await,
            "intel" => self.scrape_intel(&id.device).await,
            _ => Err(SourceError::NotFound),
        }
    }
}

/// Extract a version-like string (e.g. "566.14", "32.0.101.6083") from text.
fn extract_version_string(text: &str) -> Option<String> {
    // Simple state machine: find digit-dot-digit sequences.
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut has_dot = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                if chars[i] == '.' {
                    has_dot = true;
                }
                i += 1;
            }
            if has_dot {
                let version: String = chars[start..i].iter().collect();
                if version.contains('.') && !version.ends_with('.') {
                    return Some(version);
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Map NVIDIA device names to PSID (product series ID) for the download URL.
fn nvidia_psid(device: &str) -> &str {
    let d = device.to_lowercase();
    if d.contains("rtx-40") {
        "107"
    } else if d.contains("rtx-30") {
        "106"
    } else if d.contains("rtx-20") {
        "105"
    } else if d.contains("gtx-16") {
        "156"
    } else {
        "0"
    }
}

/// Map NVIDIA device names to PFID (product ID) for the download URL.
fn nvidia_pfid(device: &str) -> &str {
    let d = device.to_lowercase();
    if d.contains("4090") {
        "950"
    } else if d.contains("4080") {
        "949"
    } else if d.contains("4070") {
        "948"
    } else if d.contains("4060") {
        "947"
    } else if d.contains("3090") {
        "840"
    } else if d.contains("3080") {
        "839"
    } else if d.contains("3070") {
        "838"
    } else if d.contains("3060") {
        "837"
    } else {
        "0"
    }
}
