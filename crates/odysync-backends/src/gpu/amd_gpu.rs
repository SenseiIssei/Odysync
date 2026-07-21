//! AMD GPU driver update backend.
//!
//! Detects installed AMD GPU driver version via the Windows registry and uses
//! winget to find and install the latest Radeon Software Adrenalin driver.
//!
//! AMD uses two versioning schemes:
//!   - `radeonSoftwareVersion` (e.g. "23.12.1") — the user-facing version
//!   - `driverVersion` (e.g. "23.30.13.01-...") — the internal driver version
//!
//! We use the radeonSoftwareVersion for comparison since that's what winget
//! packages report.

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

/// The winget package ID for AMD Radeon Software.
const WINGET_PACKAGE_ID: &str = "AMD.RadeonSoftware";

pub struct AmdGpuBackend;

impl AmdGpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AmdGpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AmdGpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::AmdGpu
    }

    fn display_name(&self) -> &str {
        "AMD GPU Driver"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        enumerate_display_adapters()
            .await
            .iter()
            .any(|a| a.vendor == GpuVendor::Amd)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let adapters = enumerate_display_adapters().await;
        let amd_adapters: Vec<_> = adapters
            .iter()
            .filter(|a| a.vendor == GpuVendor::Amd)
            .collect();

        if amd_adapters.is_empty() {
            return Ok(Vec::new());
        }

        let installed_version = read_installed_driver_version().await;

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
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "winget search for AMD driver failed");
            return Ok(Vec::new());
        }

        let candidates = crate::winget::table::parse(&out.stdout)
            .into_iter()
            .filter(|row| row.id.eq_ignore_ascii_case(WINGET_PACKAGE_ID))
            .map(|row| {
                let name = if amd_adapters.len() == 1 {
                    format!("AMD {} Driver", amd_adapters[0].name)
                } else {
                    "AMD Radeon Software".to_string()
                };
                UpdateCandidate {
                    id: PackageId::new(BackendKind::AmdGpu, WINGET_PACKAGE_ID),
                    name,
                    installed: Version::parse(installed_version.as_deref().unwrap_or("0.0.0")),
                    available: Version::parse(&row.version),
                    size_bytes: None,
                    expected_sha256: None,
                }
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
                WINGET_PACKAGE_ID,
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

/// Read the installed AMD Radeon Software version from the Windows registry.
///
/// AMD stores the driver version under:
/// - `HKLM\SYSTEM\CurrentControlSet\Services\amdkmdag\Global\DisplayVersion`
/// - `HKLM\SOFTWARE\AMD\CN\DriverVersion`
#[cfg(windows)]
async fn read_installed_driver_version() -> Option<String> {
    let script = r#"
        $paths = @(
            @{Path='HKLM:\SYSTEM\CurrentControlSet\Services\amdkmdag\Global'; Name='DisplayVersion'},
            @{Path='HKLM:\SOFTWARE\AMD\CN'; Name='DriverVersion'},
            @{Path='HKLM:\SOFTWARE\AMD\CN'; Name='SoftwareVersion'}
        )
        foreach ($p in $paths) {
            $val = (Get-ItemProperty -Path $p.Path -Name $p.Name -ErrorAction SilentlyContinue).($p.Name)
            if ($val) { Write-Output $val; break }
        }
    "#;

    let out = proc::powershell(script, Duration::from_secs(10))
        .await
        .ok()?;
    let version = out.stdout.trim().to_string();
    if version.is_empty() {
        None
    } else {
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
        let b = AmdGpuBackend::new();
        assert_eq!(b.kind(), BackendKind::AmdGpu);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = AmdGpuBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = AmdGpuBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::AmdGpu, WINGET_PACKAGE_ID),
            name: "AMD Driver".into(),
            installed: Version::parse("23.10.1"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
