//! Windows driver updates via the Windows Update Agent COM API.
//!
//! v1 used the PSWindowsUpdate PowerShell module, which is a third-party
//! module from PSGallery installed at runtime as Administrator — a supply-chain
//! hole and a slow one. The COM API is built into Windows, needs no install,
//! and is considerably faster.
//!
//! The COM plumbing itself lives in [`crate::windows_update::session`], shared
//! with the Windows Update backends. It used to be duplicated here, and the
//! duplicate was missing the download step, the EULA acceptance and the
//! reboot flag — see that module's header for what each omission cost.

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;

use crate::windows_update::session::{self, WuaUpdate, CRITERIA_DRIVER};

pub struct WindowsDriverBackend;

impl WindowsDriverBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsDriverBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Turn a driver offering into a candidate the policy engine can order.
///
/// The previous version of this function set
/// `available = Version::parse("{update_guid}.{revision}")`. `Version::parse`
/// splits at the first `-` following a digit, so the whole string collapsed to
/// the GUID's first block — `22cb1a63` — which contains no numeric segment and
/// therefore parsed as `Version::Unknown`. The policy engine blocks unknown
/// versions, and `apply` below refuses them outright, so **no driver could ever
/// be installed**: the scan listed them and every apply was rejected.
///
/// A driver has no installed version to compare against; it is either the one
/// Windows wants or it is not. `0` to the revision number is a real ordered
/// pair, and the identity stays where it belongs, in the `PackageId`.
fn to_candidate(update: &WuaUpdate) -> UpdateCandidate {
    UpdateCandidate {
        id: PackageId::new(BackendKind::WindowsDrivers, update.key()),
        name: update.title.clone(),
        installed: Version::parse("0"),
        available: Version::parse(&update.revision.max(1).to_string()),
        size_bytes: update.size_bytes,
        expected_sha256: None,
    }
}

#[async_trait]
impl Backend for WindowsDriverBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::WindowsDrivers
    }

    fn display_name(&self) -> &str {
        "Windows Update (Drivers)"
    }

    async fn is_available(&self) -> bool {
        // This used to also require elevation, on the stated grounds that the
        // WUA driver search fails with access-denied otherwise. That is not
        // true: run unelevated against a real machine, the same search returns
        // its results normally — it is the *install* that needs admin rights.
        //
        // Requiring elevation here hid the backend entirely, so an ordinary
        // user saw an empty Hardware page rather than "3 driver updates
        // pending". Listing them and letting the policy engine block the apply
        // with `RequiresElevation` puts the reason in front of the user, which
        // is what the inline skip-reason design is for.
        cfg!(windows)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let updates = session::search(CRITERIA_DRIVER).await?;
        Ok(updates.iter().map(to_candidate).collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target revision".into(),
            });
        }

        session::install(CRITERIA_DRIVER, candidate.id.native.clone()).await?;
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        // Convergence means Windows stopped offering the driver. Still listed
        // is still pending; gone is installed.
        if !cfg!(windows) {
            return Ok(None);
        }

        let remaining = session::search(CRITERIA_DRIVER).await?;
        if remaining.iter().any(|u| u.key() == candidate.id.native) {
            return Ok(None);
        }
        Ok(Some(candidate.available.raw().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::windows_update::UpdateClass;

    fn driver(revision: i32) -> WuaUpdate {
        WuaUpdate {
            id: "22cb1a63-4de1-4b7f-9c1e-000000000001".into(),
            revision,
            title: "Intel Corporation - Display - 31.0.101.5333".into(),
            category_ids: vec![odysync_core::windows_update::CATEGORY_DRIVER.into()],
            kb_ids: Vec::new(),
            is_downloaded: false,
            eula_accepted: false,
            size_bytes: None,
            class: UpdateClass::Driver,
        }
    }

    #[test]
    fn driver_backend_reports_correct_kind() {
        let b = WindowsDriverBackend::new();
        assert_eq!(b.kind(), BackendKind::WindowsDrivers);
    }

    #[test]
    fn driver_backend_has_display_name() {
        let b = WindowsDriverBackend::new();
        assert!(!b.display_name().is_empty());
    }

    /// The regression that made every driver uninstallable. A GUID-derived
    /// version parses to `Unknown`, which the policy engine blocks and `apply`
    /// refuses, so the driver could be listed forever and never installed.
    #[test]
    fn a_driver_candidate_is_orderable_rather_than_unknown() {
        let c = to_candidate(&driver(7));

        assert!(c.installed.is_known(), "installed version is unorderable");
        assert!(c.available.is_known(), "available version is unorderable");
        assert!(
            c.installed.is_upgrade_to(&c.available),
            "a pending driver must read as an upgrade"
        );
        assert!(!c.available.is_prerelease(), "stable-only would block it");
    }

    /// Pinning down the exact input that used to break: the old scheme fed the
    /// GUID itself into the version parser.
    #[test]
    fn the_old_guid_based_version_scheme_was_unparseable() {
        let old = Version::parse("22cb1a63-4de1-4b7f-9c1e-000000000001.7");
        assert!(
            !old.is_known(),
            "if this ever parses, the regression above is no longer being tested"
        );
    }

    #[test]
    fn the_candidate_id_keeps_the_revision_and_passes_validation() {
        let a = to_candidate(&driver(7));
        let b = to_candidate(&driver(8));
        assert_ne!(a.id, b.id);
        assert!(a.id.native.ends_with(".7"));
        a.id.validate().expect("id must pass injection validation");
    }
}
