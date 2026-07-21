//! Razer Synapse backend (informational).
//!
//! Razer Synapse 3 does not have a documented CLI for driver/software updates.
//! This backend detects Razer hardware and logs a recommendation. Software
//! updates for Razer peripherals can also be found via winget.
//!
//! References:
//!   - https://www.razer.com/synapse-3

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, UpdateCandidate};

use crate::oem;

pub struct RazerSynapseBackend;

impl RazerSynapseBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RazerSynapseBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for RazerSynapseBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::RazerSynapse
    }

    fn display_name(&self) -> &str {
        "Razer Synapse"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        oem::OemManufacturer::detect().await == oem::OemManufacturer::Razer
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let installed = oem::oem_tool_path(oem::OemManufacturer::Razer).is_some();

        if installed {
            tracing::info!(
                "Razer Synapse is installed; use its built-in updater for software updates"
            );
        } else {
            tracing::info!(
                "Razer machine detected but Synapse is not installed; \
                 software updates may be available via winget"
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
            "Razer Synapse",
            "Razer Synapse does not support CLI-driven installs; \
             please use the Synapse built-in updater manually",
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
        let b = RazerSynapseBackend::new();
        assert_eq!(b.kind(), BackendKind::RazerSynapse);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = RazerSynapseBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = RazerSynapseBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::RazerSynapse, "razer-driver"),
            name: "Razer Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
