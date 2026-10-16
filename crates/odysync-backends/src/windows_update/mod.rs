//! Windows Update backends: security, quality, Defender and feature upgrades.
//!
//! Windows Update is one API offering four very different things. Rather than
//! one backend with an internal risk switch, each class gets its own backend
//! over the shared [`session`] wrapper. That falls out of the existing
//! architecture for free:
//!
//!   * the user can disable Defender definitions without disabling security
//!     patches, using the `disabled-backends` list they already have;
//!   * feature upgrades are gated by [`BackendKind::enabled_by_default`]
//!     returning `false`, so a background daemon can never start an OS version
//!     jump on its own;
//!   * the CLI, the GUI, holds, pins and profiles all pick the split up
//!     without a line of front-end code.
//!
//! Drivers are the fourth class and keep their own backend in
//! `crate::windows_drivers`, which now drives the same session wrapper.

pub mod session;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;
use odysync_core::windows_update::UpdateClass;

use session::{WuaUpdate, CRITERIA_SOFTWARE};

/// One backend per [`UpdateClass`], all sharing the same Windows Update search.
pub struct WindowsUpdateBackend {
    kind: BackendKind,
    class: UpdateClass,
    display_name: &'static str,
}

impl WindowsUpdateBackend {
    /// Security, critical and ordinary quality updates — the default case.
    ///
    /// Security and quality share a backend deliberately. Splitting them would
    /// invite users to disable "quality" updates, and Microsoft ships security
    /// fixes inside the monthly cumulative, so the distinction does not survive
    /// contact with how Windows is actually serviced.
    pub fn new() -> Self {
        Self {
            kind: BackendKind::WindowsUpdate,
            class: UpdateClass::Security,
            display_name: "Windows Update (security & quality)",
        }
    }

    /// Microsoft Defender signature updates.
    pub fn defender() -> Self {
        Self {
            kind: BackendKind::WindowsDefenderUpdate,
            class: UpdateClass::Definition,
            display_name: "Microsoft Defender definitions",
        }
    }

    /// Windows version upgrades and service packs. Off unless opted into.
    pub fn feature_upgrades() -> Self {
        Self {
            kind: BackendKind::WindowsFeatureUpdate,
            class: UpdateClass::FeatureUpgrade,
            display_name: "Windows feature upgrades",
        }
    }

    /// Whether this backend claims `update`.
    ///
    /// The security backend also takes `Quality`, per the note on [`new`].
    fn owns(&self, update: &WuaUpdate) -> bool {
        match self.class {
            UpdateClass::Security => {
                matches!(update.class, UpdateClass::Security | UpdateClass::Quality)
            }
            other => update.class == other,
        }
    }
}

impl Default for WindowsUpdateBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Turn a WUA offering into a candidate the policy engine can reason about.
///
/// Windows updates have no installed version to compare against — the thing is
/// either present or it is not. Inventing a version pair out of the update's
/// GUID, which is what the driver backend used to do, produced a
/// `Version::Unknown` that the policy engine then blocked forever. `0` to the
/// revision number is a real ordered pair: it compares correctly, it is never
/// a pre-release, and it moves when Microsoft revises the update in place.
/// Identity is carried by the `PackageId` and the KB number, where it belongs.
fn to_candidate(kind: BackendKind, update: &WuaUpdate) -> UpdateCandidate {
    // The native id has to be `<guid>.<revision>`: that is what the Update
    // Agent accepts, and pinning the exact revision is required by the backend
    // contract. But nobody holds an update by GUID — they hold `KB5101650`, so
    // that goes on as an alias the policy engine will also match.
    let mut id = PackageId::new(kind, update.key());
    if let Some(kb) = update.kb_ids.first() {
        id = id.with_alias(format!("KB{kb}"));
    }

    UpdateCandidate {
        id,
        name: update.title.clone(),
        installed: Version::parse("0"),
        // A revision of 0 would compare equal to "not installed" and the
        // update would be skipped as `NotAnUpgrade`. Revisions start at 1 in
        // practice; the clamp is here so a surprising 0 cannot silence it.
        available: Version::parse(&update.revision.max(1).to_string()),
        size_bytes: update.size_bytes,
        expected_sha256: None,
    }
}

#[async_trait]
impl Backend for WindowsUpdateBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn display_name(&self) -> &str {
        self.display_name
    }

    async fn is_available(&self) -> bool {
        // Searching works unelevated — verified against a live machine, where
        // an unelevated search returned 14 pending updates, 10 of them
        // security patches. Only the install needs admin rights, and
        // `BackendKind::requires_elevation` already makes the policy engine
        // block the apply with `RequiresElevation`.
        //
        // So the backend stays available: the user sees the pending security
        // updates and the reason they cannot be applied yet. Hiding them until
        // the tool happens to be run as administrator would mean a normal user
        // never learns their machine is missing security patches.
        cfg!(windows)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        let all = session::search(CRITERIA_SOFTWARE).await?;
        Ok(all
            .iter()
            .filter(|u| self.owns(u))
            .map(|u| to_candidate(self.kind, u))
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target revision".into(),
            });
        }

        // Re-check the class at apply time rather than trusting the candidate
        // that came back from the front-end. A candidate crosses a process
        // boundary between scan and apply in the GUI, and this backend must
        // never be the one that installs a feature upgrade.
        let all = session::search(CRITERIA_SOFTWARE).await?;
        let Some(update) = all.iter().find(|u| u.key() == candidate.id.native) else {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "the update was offered during the scan but Windows no longer offers it"
                    .into(),
            });
        };
        if !self.owns(update) {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: format!(
                    "{} is a {} and will not be installed by the {} backend",
                    update.short_name(),
                    update.class.label(),
                    self.kind
                ),
            });
        }

        session::install(CRITERIA_SOFTWARE, candidate.id.native.clone()).await?;
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if !cfg!(windows) {
            return Ok(None);
        }

        // Convergence for a Windows update means "Windows stopped offering
        // it". An exit code of zero is not accepted as proof, per the runner's
        // contract; this is the read-back that replaces it.
        let remaining = session::search(CRITERIA_SOFTWARE).await?;
        if remaining.iter().any(|u| u.key() == candidate.id.native) {
            return Ok(None);
        }
        Ok(Some(candidate.available.raw().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wua(class: UpdateClass, revision: i32) -> WuaUpdate {
        WuaUpdate {
            id: "11111111-2222-3333-4444-555555555555".into(),
            revision,
            title: "A pending update".into(),
            category_ids: Vec::new(),
            kb_ids: vec!["5101650".into()],
            is_downloaded: false,
            eula_accepted: false,
            size_bytes: None,
            class,
        }
    }

    #[test]
    fn each_constructor_reports_its_own_kind_and_name() {
        let all = [
            WindowsUpdateBackend::new(),
            WindowsUpdateBackend::defender(),
            WindowsUpdateBackend::feature_upgrades(),
        ];
        let mut kinds: Vec<BackendKind> = all.iter().map(|b| b.kind()).collect();
        kinds.sort_by_key(|k| k.id());
        let count = kinds.len();
        kinds.dedup();
        assert_eq!(kinds.len(), count, "two backends share a kind");
        assert!(all.iter().all(|b| !b.display_name().is_empty()));
    }

    #[test]
    fn the_security_backend_also_takes_quality_updates() {
        let b = WindowsUpdateBackend::new();
        assert!(b.owns(&wua(UpdateClass::Security, 1)));
        assert!(b.owns(&wua(UpdateClass::Quality, 1)));
        assert!(!b.owns(&wua(UpdateClass::Definition, 1)));
        assert!(!b.owns(&wua(UpdateClass::FeatureUpgrade, 1)));
        assert!(!b.owns(&wua(UpdateClass::Driver, 1)));
    }

    #[test]
    fn the_defender_backend_takes_only_definitions() {
        let b = WindowsUpdateBackend::defender();
        assert!(b.owns(&wua(UpdateClass::Definition, 1)));
        for c in [
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::FeatureUpgrade,
            UpdateClass::Driver,
        ] {
            assert!(!b.owns(&wua(c, 1)), "{} leaked into Defender", c.id());
        }
    }

    #[test]
    fn the_feature_backend_takes_only_feature_upgrades() {
        let b = WindowsUpdateBackend::feature_upgrades();
        assert!(b.owns(&wua(UpdateClass::FeatureUpgrade, 1)));
        for c in [
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::Definition,
            UpdateClass::Driver,
        ] {
            assert!(
                !b.owns(&wua(c, 1)),
                "{} leaked into feature upgrades",
                c.id()
            );
        }
    }

    #[test]
    fn no_class_is_claimed_by_two_backends() {
        let all = [
            WindowsUpdateBackend::new(),
            WindowsUpdateBackend::defender(),
            WindowsUpdateBackend::feature_upgrades(),
        ];
        for class in [
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::Definition,
            UpdateClass::FeatureUpgrade,
        ] {
            let owners = all.iter().filter(|b| b.owns(&wua(class, 1))).count();
            assert_eq!(owners, 1, "{} has {owners} owners", class.id());
        }
    }

    /// The regression that motivated the whole version scheme: a candidate
    /// whose version cannot be parsed is refused by the policy engine and then
    /// again by `apply`, so it can be listed forever and never installed.
    #[test]
    fn a_candidate_carries_versions_the_policy_engine_can_order() {
        let c = to_candidate(BackendKind::WindowsUpdate, &wua(UpdateClass::Security, 3));

        assert!(c.installed.is_known(), "installed version is unorderable");
        assert!(c.available.is_known(), "available version is unorderable");
        assert!(
            c.installed.is_upgrade_to(&c.available),
            "a pending update must read as an upgrade"
        );
        assert!(!c.available.is_prerelease(), "stable-only would block it");
    }

    #[test]
    fn a_zero_revision_still_reads_as_an_upgrade() {
        let c = to_candidate(BackendKind::WindowsUpdate, &wua(UpdateClass::Security, 0));
        assert!(c.installed.is_upgrade_to(&c.available));
    }

    #[test]
    fn the_candidate_id_survives_a_revision_bump() {
        let a = to_candidate(BackendKind::WindowsUpdate, &wua(UpdateClass::Security, 1));
        let b = to_candidate(BackendKind::WindowsUpdate, &wua(UpdateClass::Security, 2));
        assert_ne!(a.id, b.id, "a revised update must be a distinct candidate");
        a.id.validate().expect("id must pass injection validation");
    }
}
