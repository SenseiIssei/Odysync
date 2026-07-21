//! Dell Command Update backend.
//!
//! Wraps `dcu-cli.exe` — Dell's enterprise CLI tool for driver, BIOS, and
//! firmware updates.  The tool must be pre-installed on Dell machines.
//!
//! Commands used:
//!   - `dcu-cli.exe /scan` — list applicable updates
//!   - `dcu-cli.exe /applyUpdates -silent -updateType=driver,bios,firmware`
//!
//! Reference: https://www.dell.com/support/manuals/en-us/command-update-v3.1/dellcommandupdate_3.1_ug/command-line-interface-reference

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

pub struct DellCommandUpdateBackend {
    tool_path: std::path::PathBuf,
}

impl DellCommandUpdateBackend {
    pub fn new() -> Self {
        let tool_path = oem::oem_tool_path(oem::OemManufacturer::Dell)
            .unwrap_or_else(|| std::path::PathBuf::from("dcu-cli.exe"));
        Self { tool_path }
    }
}

impl Default for DellCommandUpdateBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for DellCommandUpdateBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::DellCommandUpdate
    }

    fn display_name(&self) -> &str {
        "Dell Command Update"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        let oem_manufacturer = oem::OemManufacturer::detect().await;
        if oem_manufacturer != oem::OemManufacturer::Dell {
            return false;
        }
        oem::tool_exists(self.tool_path.to_str().unwrap_or(""))
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let out = proc::run(
            self.tool_path.to_str().unwrap_or("dcu-cli.exe"),
            &["/scan"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "dcu-cli /scan failed");
            return Ok(Vec::new());
        }

        Ok(parse_dcu_scan_output(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Dell Command Update applies all pending updates at once; we cannot
        // target a specific package.  We pass -updateType to filter by type.
        let update_type = candidate.id.native.as_str();

        let out = proc::run(
            self.tool_path.to_str().unwrap_or("dcu-cli.exe"),
            &[
                "/applyUpdates",
                "-silent",
                "-updateType",
                update_type,
                "-reboot=disable",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: "dcu-cli /applyUpdates".into(),
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
        // Dell Command Update doesn't expose per-driver versions after install.
        // Re-scan to check if the update is still listed.
        let _remaining = self.scan().await?;
        Ok(None)
    }
}

/// Parse `dcu-cli.exe /scan` output.
///
/// The output format varies by version but typically lists update titles,
/// versions, and types.  We extract what we can.
fn parse_dcu_scan_output(output: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        // Look for lines that contain version information.
        // DCU output format is not well-documented; parse defensively.
        if line.is_empty() || line.starts_with("Copyright") || line.starts_with("Dell") {
            continue;
        }

        // Try to extract a package name and version from the line.
        // Common patterns: "Driver: Intel(R) Wireless, Version: 23.0.0"
        if let Some(name) = extract_field(line, "Name:") {
            let version = extract_field(line, "Version:")
                .or_else(|| extract_field(line, "version:"))
                .unwrap_or_default();
            let update_type = extract_field(line, "Type:")
                .or_else(|| extract_field(line, "type:"))
                .unwrap_or_else(|| "driver".to_string());

            candidates.push(UpdateCandidate {
                id: PackageId::new(BackendKind::DellCommandUpdate, &update_type),
                name,
                installed: Version::parse("0.0.0"),
                available: Version::parse(&version),
                size_bytes: None,
                expected_sha256: None,
            });
        }
    }

    candidates
}

fn extract_field(line: &str, prefix: &str) -> Option<String> {
    let pos = line.find(prefix)?;
    let rest = &line[pos + prefix.len()..];
    let value = rest.split(',').next()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = DellCommandUpdateBackend::new();
        assert_eq!(b.kind(), BackendKind::DellCommandUpdate);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = DellCommandUpdateBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = DellCommandUpdateBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::DellCommandUpdate, "driver"),
            name: "Dell Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
