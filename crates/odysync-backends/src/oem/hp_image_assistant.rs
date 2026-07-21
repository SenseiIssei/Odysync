//! HP Image Assistant (HPIA) backend.
//!
//! Wraps `HPImageAssistant.exe` — HP's enterprise tool for analyzing and
//! installing driver, BIOS, and firmware updates on HP business PCs.
//!
//! Commands used:
//!   - `HPImageAssistant.exe /Operation:Analyze /Action:List /Selection:All /Silent`
//!   - `HPImageAssistant.exe /Operation:Analyze /Action:Install /Selection:All /Silent`
//!
//! Reference: https://ftp.ext.hp.com/pub/caps-softpaq/cmit/whitepapers/HPIAUserGuide.pdf

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::oem;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(1800);

pub struct HpImageAssistantBackend {
    tool_path: std::path::PathBuf,
}

impl HpImageAssistantBackend {
    pub fn new() -> Self {
        let tool_path = oem::oem_tool_path(oem::OemManufacturer::Hp)
            .unwrap_or_else(|| std::path::PathBuf::from("HPImageAssistant.exe"));
        Self { tool_path }
    }
}

impl Default for HpImageAssistantBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for HpImageAssistantBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::HpImageAssistant
    }

    fn display_name(&self) -> &str {
        "HP Image Assistant"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        let oem_manufacturer = oem::OemManufacturer::detect().await;
        if oem_manufacturer != oem::OemManufacturer::Hp {
            return false;
        }
        oem::tool_exists(self.tool_path.to_str().unwrap_or(""))
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let out = proc::run(
            self.tool_path.to_str().unwrap_or("HPImageAssistant.exe"),
            &[
                "/Operation:Analyze",
                "/Action:List",
                "/Selection:All",
                "/Silent",
            ],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "HPIA analyze failed");
            return Ok(Vec::new());
        }

        Ok(parse_hpia_output(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // HPIA installs all recommended updates at once; we cannot target
        // a specific SoftPaq from the CLI.  Use /Selection to filter.
        let selection = match candidate.id.native.as_str() {
            "critical" => "/Selection:Critical",
            "recommended" => "/Selection:Recommended",
            _ => "/Selection:All",
        };

        let out = proc::run(
            self.tool_path.to_str().unwrap_or("HPImageAssistant.exe"),
            &[
                "/Operation:Analyze",
                "/Action:Install",
                selection,
                "/Silent",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: "HPImageAssistant /Action:Install".into(),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            })
        }
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        // HPIA doesn't expose per-driver versions after install.
        Ok(None)
    }
}

/// Parse HPIA `/Action:List` output.
///
/// HPIA output is typically XML or structured text.  We parse defensively
/// for SoftPaq IDs, names, and versions.
fn parse_hpia_output(output: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Look for SoftPaq references in the output.
        // HPIA lists recommendations with SoftPaq numbers like "SP12345".
        if let Some(softpaq) = extract_softpaq_id(line) {
            let name = line.to_string();
            candidates.push(UpdateCandidate {
                id: PackageId::new(BackendKind::HpImageAssistant, &softpaq),
                name,
                installed: Version::parse("0.0.0"),
                available: Version::parse("1.0.0"),
                size_bytes: None,
                expected_sha256: None,
            });
        }
    }

    candidates
}

fn extract_softpaq_id(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let pos = lower.find("sp")?;
    let rest = &line[pos..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if id.len() >= 3 && id.starts_with("sp") {
        Some(id.to_uppercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = HpImageAssistantBackend::new();
        assert_eq!(b.kind(), BackendKind::HpImageAssistant);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = HpImageAssistantBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = HpImageAssistantBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::HpImageAssistant, "SP12345"),
            name: "HP Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
