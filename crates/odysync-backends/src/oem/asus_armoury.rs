//! ASUS Armoury Crate backend (informational).
//!
//! ASUS Armoury Crate's "Live Update" feature does not have a documented CLI,
//! so this backend detects ASUS hardware and logs a recommendation to use
//! Armoury Crate's Live Update feature. Actual driver delivery falls back to
//! the WindowsDrivers backend which queries Windows Update.
//!
//! References:
//!   - https://www.asus.com/software/armoury-crate/

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, UpdateCandidate};

use crate::oem;

pub struct AsusArmouryBackend;

impl AsusArmouryBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AsusArmouryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AsusArmouryBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::AsusArmoury
    }

    fn display_name(&self) -> &str {
        "ASUS Armoury Crate"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        oem::OemManufacturer::detect().await == oem::OemManufacturer::Asus
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let installed = oem::oem_tool_path(oem::OemManufacturer::Asus).is_some();

        if installed {
            tracing::info!(
                "ASUS Armoury Crate is installed; use its Live Update feature for driver updates"
            );
        } else {
            tracing::info!(
                "ASUS machine detected but Armoury Crate is not installed; \
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
            "ASUS Armoury Crate",
            "ASUS Armoury Crate does not support CLI-driven installs; \
             please use the Armoury Crate Live Update feature manually",
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
        let b = AsusArmouryBackend::new();
        assert_eq!(b.kind(), BackendKind::AsusArmoury);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = AsusArmouryBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = AsusArmouryBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::AsusArmoury, "asus-driver"),
            name: "ASUS Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
