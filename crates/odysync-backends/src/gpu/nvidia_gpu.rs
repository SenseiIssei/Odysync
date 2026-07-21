//! NVIDIA GPU driver update backend.
//!
//! Detects installed NVIDIA GPU driver version via the Windows registry and
//! uses winget to find and install the latest driver.  NVAPI could also be
//! used for version detection, but the registry is simpler and doesn't require
//! FFI bindings.
//!
//! NVIDIA does not publish a public "latest driver" API, so we rely on winget
//! packages (e.g. `Nvidia.GeForceDriver`) for available version information.

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::{ApplyPhase, ApplyProgress, Backend};
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use super::{enumerate_display_adapters, GpuVendor};

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The winget package ID for NVIDIA GeForce drivers.
const WINGET_PACKAGE_ID: &str = "Nvidia.GeForceDriver";

pub struct NvidiaGpuBackend;

impl NvidiaGpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NvidiaGpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NvidiaGpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::NvidiaGpu
    }

    fn display_name(&self) -> &str {
        "NVIDIA GPU Driver"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        // Available if there is at least one NVIDIA adapter on this system.
        enumerate_display_adapters()
            .await
            .iter()
            .any(|a| a.vendor == GpuVendor::Nvidia)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let adapters = enumerate_display_adapters().await;
        let nvidia_adapters: Vec<_> = adapters
            .iter()
            .filter(|a| a.vendor == GpuVendor::Nvidia)
            .collect();

        if nvidia_adapters.is_empty() {
            return Ok(Vec::new());
        }

        // Get installed driver version from registry.
        let installed_version = read_installed_driver_version().await;

        // Query winget for available NVIDIA driver versions.
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
            tracing::warn!(
                stderr = %out.stderr,
                "winget search for NVIDIA driver failed"
            );
            return Ok(Vec::new());
        }

        // Parse the winget table output to get the available version.
        let candidates = crate::winget::table::parse(&out.stdout)
            .into_iter()
            .filter(|row| row.id.eq_ignore_ascii_case(WINGET_PACKAGE_ID))
            .map(|row| {
                let name = if nvidia_adapters.len() == 1 {
                    format!("NVIDIA {} Driver", nvidia_adapters[0].name)
                } else {
                    "NVIDIA GPU Driver".to_string()
                };
                UpdateCandidate {
                    id: PackageId::new(BackendKind::NvidiaGpu, WINGET_PACKAGE_ID),
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

    async fn apply_with_progress(
        &self,
        candidate: &UpdateCandidate,
        tx: Option<tokio::sync::mpsc::Sender<ApplyProgress>>,
    ) -> Result<()> {
        if let Some(tx) = &tx {
            let _ = tx
                .send(ApplyProgress {
                    percent: None,
                    message: format!(
                        "Downloading {} {}…",
                        candidate.name,
                        candidate.available.raw()
                    ),
                    phase: ApplyPhase::Downloading,
                })
                .await;
        }

        let result = self.apply(candidate).await;

        if let Some(tx) = &tx {
            if result.is_ok() {
                let _ = tx
                    .send(ApplyProgress {
                        percent: Some(100),
                        message: format!(
                            "Installed {} {}",
                            candidate.name,
                            candidate.available.raw()
                        ),
                        phase: ApplyPhase::Verifying,
                    })
                    .await;
            }
        }

        result
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if !cfg!(windows) {
            return Ok(None);
        }

        let current = read_installed_driver_version().await;
        Ok(current.filter(|v| Version::parse(v) == candidate.available))
    }
}

/// Read the installed NVIDIA driver version from the Windows registry.
///
/// NVIDIA stores the driver version in several places:
/// - `HKLM\SOFTWARE\NVIDIA Corporation\Global\DriverSync\Installed`
/// - `HKLM\SYSTEM\CurrentControlSet\services\nvlddmkm\Global\NVTweak\DisplayVersion`
/// - `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{NVIDIA driver}\DisplayVersion`
#[cfg(windows)]
async fn read_installed_driver_version() -> Option<String> {
    let script = r#"
        $paths = @(
            'HKLM:\SOFTWARE\NVIDIA Corporation\Global\DriverSync',
            'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm\Global\NVTweak'
        )
        foreach ($p in $paths) {
            $val = (Get-ItemProperty -Path $p -Name 'Installed' -ErrorAction SilentlyContinue).Installed
            if ($val) { Write-Output $val; break }
            $val = (Get-ItemProperty -Path $p -Name 'DisplayVersion' -ErrorAction SilentlyContinue).DisplayVersion
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
        let b = NvidiaGpuBackend::new();
        assert_eq!(b.kind(), BackendKind::NvidiaGpu);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = NvidiaGpuBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = NvidiaGpuBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::NvidiaGpu, WINGET_PACKAGE_ID),
            name: "NVIDIA Driver".into(),
            installed: Version::parse("500.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
