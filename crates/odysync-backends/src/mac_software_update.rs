//! macOS software updates: macOS itself and Apple's other system software.
//!
//! Distinct from [`crate::mac_firmware`], which takes the firmware out of the
//! same list and holds the code both share. A new major version of macOS is
//! not offered here: it is an upgrade of the operating system, which the user
//! starts from System Settings.
//!
//! Commands used:
//!   - `softwareupdate --list`: list available updates
//!   - `softwareupdate --install <label>`: install a specific update

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, UpdateCandidate};

use crate::mac_firmware::{Part, apply, installed_version, scan};

pub struct MacSoftwareUpdateBackend;

impl MacSoftwareUpdateBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacSoftwareUpdateBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for MacSoftwareUpdateBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::MacSoftwareUpdate
    }

    fn display_name(&self) -> &str {
        "macOS Software Updates"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "macos")
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        scan(Part::System).await
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        apply(Part::System, candidate).await
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        installed_version(Part::System, candidate).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    #[test]
    fn backend_kind_is_correct() {
        let b = MacSoftwareUpdateBackend::new();
        assert_eq!(b.kind(), BackendKind::MacSoftwareUpdate);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(
            &MacSoftwareUpdateBackend::new(),
            "macOS Tahoe 26.6-25G72",
            None,
        )
        .await;
    }
}
