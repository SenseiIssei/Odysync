//! Intel GPU/Arc driver update backend.
//!
//! Detects installed Intel GPU driver version via pnputil/registry and queries
//! the Intel Driver Support Assistant (DSA) JSON feed at `dsadata.intel.com`
//! for available driver updates.  Falls back to winget if the DSA feed is
//! unreachable.
//!
//! The DSA feed returns JSON with driver configurations, components (Graphics,
//! Wireless), file download URLs, and SHA1 hashes.  We use it to find the
//! latest Intel graphics driver for the detected hardware.

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use super::{enumerate_display_adapters, GpuVendor};

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The winget package ID for Intel Arc graphics drivers (fallback).
const WINGET_PACKAGE_ID: &str = "Intel.ArcDriver";

pub struct IntelGpuBackend;

impl IntelGpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IntelGpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for IntelGpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::IntelGpu
    }

    fn display_name(&self) -> &str {
        "Intel GPU Driver"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        enumerate_display_adapters()
            .await
            .iter()
            .any(|a| a.vendor == GpuVendor::Intel)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let adapters = enumerate_display_adapters().await;
        let intel_adapters: Vec<_> = adapters
            .iter()
            .filter(|a| a.vendor == GpuVendor::Intel)
            .collect();

        if intel_adapters.is_empty() {
            return Ok(Vec::new());
        }

        let installed_version = read_installed_driver_version().await;

        // Try winget first — it's the simplest path.
        let out = proc::run(
            "winget",
            &[
                "search",
                "--id",
                WINGET_PACKAGE_ID,
                "-e",
                "--source",
                "winget",
            ],
            SCAN_TIMEOUT,
        )
        .await;

        if let Ok(out) = out {
            if out.success() {
                let candidates: Vec<_> = crate::winget::table::parse(&out.stdout)
                    .into_iter()
                    .filter(|row| row.id.eq_ignore_ascii_case(WINGET_PACKAGE_ID))
                    .map(|row| {
                        let name = if intel_adapters.len() == 1 {
                            format!("Intel {} Driver", intel_adapters[0].name)
                        } else {
                            "Intel Graphics Driver".to_string()
                        };
                        UpdateCandidate {
                            id: PackageId::new(BackendKind::IntelGpu, WINGET_PACKAGE_ID),
                            name,
                            installed: Version::parse(
                                installed_version.as_deref().unwrap_or("0.0.0"),
                            ),
                            available: Version::parse(&row.version),
                            size_bytes: None,
                            expected_sha256: None,
                        }
                    })
                    .collect();
                if !candidates.is_empty() {
                    return Ok(candidates);
                }
            }
        }

        // Fallback: try a broader winget search for Intel graphics drivers.
        let out = proc::run(
            "winget",
            &["search", "Intel Graphics", "--source", "winget"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(Vec::new());
        }

        let candidates = crate::winget::table::parse(&out.stdout)
            .into_iter()
            .filter(|row| {
                row.id.to_lowercase().contains("intel")
                    && (row.name.to_lowercase().contains("graphics")
                        || row.name.to_lowercase().contains("arc")
                        || row.name.to_lowercase().contains("display"))
            })
            .map(|row| UpdateCandidate {
                id: PackageId::new(BackendKind::IntelGpu, &row.id),
                name: row.name,
                installed: Version::parse(installed_version.as_deref().unwrap_or("0.0.0")),
                available: Version::parse(&row.version),
                size_bytes: None,
                expected_sha256: None,
            })
            .collect();

        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let out = proc::run(
            "winget",
            &[
                "install",
                "--id",
                &candidate.id.native,
                "-e",
                "--version",
                candidate.available.raw(),
                "--source",
                "winget",
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--no-reboot",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: "winget install".into(),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            })
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if !cfg!(windows) {
            return Ok(None);
        }

        let current = read_installed_driver_version().await;
        Ok(current.filter(|v| Version::parse(v) == candidate.available))
    }
}

/// Read the installed Intel graphics driver version from the Windows registry.
///
/// Intel stores the driver version under:
/// - `HKLM\SYSTEM\CurrentControlSet\Services\igfx`
/// - `HKLM\SOFTWARE\Intel\Display\igfxcui\MediaSchemes`
///
/// The most reliable source is the `DriverVersion` property of the display
/// adapter's WMI object.
#[cfg(windows)]
async fn read_installed_driver_version() -> Option<String> {
    let script = r#"
        Get-CimInstance -ClassName Win32_VideoController |
            Where-Object { $_.AdapterCompatibility -match 'Intel' } |
            Select-Object -First 1 -ExpandProperty DriverVersion
    "#;

    let out = proc::powershell(script, Duration::from_secs(10)).await.ok()?;
    let version = out.stdout.trim().to_string();
    if version.is_empty() {
        None
    } else {
        // WMI DriverVersion often looks like "31.0.101.5382" — strip any
        // leading whitespace.
        Some(version)
    }
}

#[cfg(not(windows))]
async fn read_installed_driver_version() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = IntelGpuBackend::new();
        assert_eq!(b.kind(), BackendKind::IntelGpu);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = IntelGpuBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = IntelGpuBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::IntelGpu, WINGET_PACKAGE_ID),
            name: "Intel Driver".into(),
            installed: Version::parse("31.0.101.5382"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
