//! Acer Care Center backend (informational).
//!
//! Acer Care Center does not have a documented CLI for driver updates. This
//! backend detects Acer hardware and logs a recommendation. Actual driver
//! delivery falls back to the WindowsDrivers backend.
//!
//! References:
//!   - https://www.acer.com/ac/en/US/content/software-acer-care-center

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, UpdateCandidate};

use crate::oem;

pub struct AcerCareCenterBackend;

impl AcerCareCenterBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AcerCareCenterBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AcerCareCenterBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::AcerCareCenter
    }

    fn display_name(&self) -> &str {
        "Acer Care Center"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        oem::OemManufacturer::detect().await == oem::OemManufacturer::Acer
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let installed = oem::oem_tool_path(oem::OemManufacturer::Acer).is_some();

        if installed {
            tracing::info!(
                "Acer Care Center is installed; use its update feature for driver updates"
            );
        } else {
            tracing::info!(
                "Acer machine detected but Care Center is not installed; \
                 driver updates will be handled by Windows Update"
            );
        }

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
            "Acer Care Center",
            "Acer Care Center does not support CLI-driven installs; \
             please use the Care Center update feature manually",
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
        let b = AcerCareCenterBackend::new();
        assert_eq!(b.kind(), BackendKind::AcerCareCenter);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = AcerCareCenterBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = AcerCareCenterBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::AcerCareCenter, "acer-driver"),
            name: "Acer Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
