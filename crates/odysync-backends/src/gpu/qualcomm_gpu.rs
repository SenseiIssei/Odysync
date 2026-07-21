//! Qualcomm Adreno GPU driver update backend.
//!
//! Detects Qualcomm Adreno GPUs (common in Windows on ARM devices) via
//! pnputil and uses winget to find and install driver updates.
//!
//! Qualcomm Adreno GPUs use PCI vendor ID 0x5143 and are found in Windows
//! on ARM devices like the Surface Pro X.

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


pub struct QualcommGpuBackend;

impl QualcommGpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for QualcommGpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for QualcommGpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::QualcommGpu
    }

    fn display_name(&self) -> &str {
        "Qualcomm Adreno GPU"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        enumerate_display_adapters()
            .await
            .iter()
            .any(|a| a.vendor == GpuVendor::Qualcomm)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let adapters = enumerate_display_adapters().await;
        let qualcomm_adapters: Vec<_> = adapters
            .iter()
            .filter(|a| a.vendor == GpuVendor::Qualcomm)
            .collect();

        if qualcomm_adapters.is_empty() {
            return Ok(Vec::new());
        }

        let installed_version = read_installed_driver_version().await;

        // Search winget for Qualcomm Adreno driver updates.
        let out = proc::run(
            "winget",
            &[
                "search",
                "--query",
                "Qualcomm Adreno driver",
                "--source",
                "winget",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "winget search for Qualcomm drivers failed");
            return Ok(Vec::new());
        }

        let candidates = super::super::winget::table::parse(&out.stdout);
        let mut results = Vec::with_capacity(candidates.len());

        for row in candidates {
            if row.name.to_lowercase().contains("qualcomm")
                || row.name.to_lowercase().contains("adreno")
            {
                results.push(UpdateCandidate {
                    id: PackageId::new(BackendKind::QualcommGpu, &row.id),
                    name: row.name.clone(),
                    installed: Version::parse(installed_version.as_deref().unwrap_or("")),
                    available: Version::parse(&row.available),
                    size_bytes: None,
                    expected_sha256: None,
                });
            }
        }

        Ok(results)
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
                "--source",
                "winget",
                "--exact",
                "--version",
                candidate.available.raw(),
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
                "--silent",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("winget install --id {} --version {}", candidate.id.native, candidate.available.raw()),
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
        let current = read_installed_driver_version().await;
        Ok(current.filter(|v| Version::parse(v) == candidate.available))
    }
}

/// Read the installed Qualcomm Adreno driver version from the Windows registry.
///
/// Qualcomm stores the driver version under:
/// - `HKLM\SYSTEM\CurrentControlSet\Services\qcomdisp\Global\DisplayVersion`
#[cfg(windows)]
async fn read_installed_driver_version() -> Option<String> {
    let script = r#"
        $paths = @(
            @{Path='HKLM:\SYSTEM\CurrentControlSet\Services\qcomdisp\Global'; Name='DisplayVersion'},
            @{Path='HKLM:\SYSTEM\CurrentControlSet\Services\qcomdisp'; Name='DisplayVersion'}
        )
        foreach ($p in $paths) {
            $val = (Get-ItemProperty -Path $p.Path -Name $p.Name -ErrorAction SilentlyContinue).($p.Name)
            if ($val) { Write-Output $val; break }
        }
    "#;

    let out = proc::powershell(script, Duration::from_secs(10)).await.ok()?;
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
        let b = QualcommGpuBackend::new();
        assert_eq!(b.kind(), BackendKind::QualcommGpu);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = QualcommGpuBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = QualcommGpuBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::QualcommGpu, "Qualcomm.AdrenoDriver"),
            name: "Qualcomm Adreno Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
