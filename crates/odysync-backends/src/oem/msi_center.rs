//! MSI Center backend (informational + Windows Update fallback).
//!
//! MSI Center's "Live Update" feature does not have a documented CLI, so this
//! backend detects MSI hardware and falls back to the Windows Update Agent for
//! driver delivery.  If MSI Center is installed, it logs a recommendation to
//! use its Live Update feature.
//!
//! References:
//!   - MSI Center User Guide: https://download-2.msi.com/archive/mnu_exe/mb/MSICENTER.pdf
//!   - MSI SDK: https://msi-sdk.software.informer.com/

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, UpdateCandidate};

use crate::oem;

pub struct MsiCenterBackend;

impl MsiCenterBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MsiCenterBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for MsiCenterBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::MsiCenter
    }

    fn display_name(&self) -> &str {
        "MSI Center"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        oem::OemManufacturer::detect().await == oem::OemManufacturer::Msi
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        // MSI Center has no CLI for scanning.  We return a single informational
        // candidate that tells the user to use MSI Center's Live Update feature.
        let msi_center_installed = oem::oem_tool_path(oem::OemManufacturer::Msi).is_some();

        if msi_center_installed {
            tracing::info!(
                "MSI Center is installed; use its Live Update feature for driver updates"
            );
        }

        // Return empty — actual driver delivery is handled by the
        // WindowsDrivers backend which queries Windows Update.
        Ok(Vec::new())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        Err(Error::parse(
            "MSI Center",
            "MSI Center does not support CLI-driven installs; \
             please use the MSI Center Live Update feature manually",
        ))
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::model::PackageId;
    use odysync_core::version::Version;

    #[test]
    fn backend_kind_is_correct() {
        let b = MsiCenterBackend::new();
        assert_eq!(b.kind(), BackendKind::MsiCenter);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = MsiCenterBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = MsiCenterBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::MsiCenter, "msi-driver"),
            name: "MSI Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
