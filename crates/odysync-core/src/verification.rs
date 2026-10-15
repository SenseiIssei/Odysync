//! Who verifies the bytes before they are executed.
//!
//! `README.md` used to list "installer digest verification and signature
//! checking" as a feature. The `odysync-verify` crate exists and is correct,
//! but it had no call sites: nothing on the apply path ever called it. The
//! claim was a dependency edge, not a behaviour.
//!
//! The reason is structural rather than an oversight. Odysync almost never
//! holds an installer file. It asks winget, apt, Homebrew or the Windows
//! Update Agent to perform the update, and those tools download, verify and
//! execute the payload inside their own process. There is no file for us to
//! hash. The one exception is the offline cache, which fetches an installer
//! over HTTPS itself — and that path now does call `odysync-verify`.
//!
//! So rather than invent a check we cannot perform, this module answers the
//! question the user actually has: *something* verified this — what was it?
//! A "Verified ✓" badge that means "winget probably did something" is worse
//! than a line saying "verified by winget, not by Odysync", because the first
//! one is indistinguishable from a check we ran ourselves.

use serde::{Deserialize, Serialize};

use crate::model::BackendKind;

/// Where the integrity guarantee for a backend's payloads comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Verification {
    /// The package manager verifies its own downloads inside its own process.
    ///
    /// Odysync cannot independently re-check these: the tool does not expose
    /// the digest it validated against, and the file never exists at a path we
    /// control. The payload *is* verified — just not by us, and the UI should
    /// say whose word it is on.
    Delegated {
        /// Who does the checking, e.g. `"winget"`.
        by: &'static str,
        /// What they check, in one clause, e.g. `"installer digests against
        /// its own package manifests"`.
        what: &'static str,
    },
    /// Odysync downloads the installer itself and checks it before running it.
    ///
    /// Only for backends where Odysync really holds the file: the publisher's
    /// code signature must validate *and* name the publisher the catalog
    /// expects, and a published SHA-256 must match when the source offers one.
    Odysync {
        /// What is checked, in one clause.
        what: &'static str,
    },
    /// Nothing verifies the payload before it runs.
    ///
    /// Not a bug to be fixed by adding a check here — it means the ecosystem
    /// has no verification mechanism to delegate to. Worth surfacing plainly
    /// so the user can weigh it.
    Unverified {
        /// Why there is nothing to delegate to.
        why: &'static str,
    },
}

impl Verification {
    /// A short label for a table cell or badge.
    pub fn label(&self) -> String {
        match self {
            Verification::Delegated { by, .. } => format!("verified by {by}"),
            Verification::Odysync { .. } => "verified by Odysync".to_string(),
            Verification::Unverified { .. } => "not verified".to_string(),
        }
    }

    /// The full sentence, for a tooltip or the `--verbose` report.
    pub fn detail(&self) -> String {
        match self {
            Verification::Delegated { by, what } => format!(
                "{by} verifies {what}. Odysync cannot re-check this independently — \
                 the payload is never written to a path it controls."
            ),
            Verification::Odysync { what } => format!(
                "Odysync downloads this installer itself and checks {what} before it runs. \
                 A mismatch stops the update."
            ),
            Verification::Unverified { why } => format!("Nothing verifies this payload: {why}."),
        }
    }

    /// Whether *someone* checks the payload.
    pub fn is_verified_by_someone(&self) -> bool {
        matches!(
            self,
            Verification::Delegated { .. } | Verification::Odysync { .. }
        )
    }
}

/// Shorthand for the common case.
const fn by(who: &'static str, what: &'static str) -> Verification {
    Verification::Delegated { by: who, what }
}

/// How payloads from `kind` are verified, and by whom.
pub fn verification_of(kind: BackendKind) -> Verification {
    use BackendKind::*;

    match kind {
        // ── Windows ─────────────────────────────────────────────────────────
        Winget
        // AMD, Intel and Qualcomm GPU drivers and the virtualisation guest
        // tools are delivered through winget, so they inherit its guarantee
        // exactly.
        | AmdGpu
        | IntelGpu
        | QualcommGpu
        | VirtualizationGuest => by(
            "winget",
            "installer digests against its own package manifests",
        ),
        MsStore => by("the Microsoft Store", "package signatures on every appx"),
        Direct => Verification::Odysync {
            what: "the publisher's code signature against the signer the catalog \
                 pins for that app, plus the SHA-256 the source publishes when it has one",
        },
        NvidiaGpu => Verification::Odysync {
            what: "that the driver package is WHQL-certified and signed by NVIDIA \
                 Corporation",
        },
        WindowsDrivers | WindowsUpdate | WindowsDefenderUpdate | WindowsFeatureUpdate => by(
            "the Windows Update Agent",
            "Microsoft's signatures on every update before installing it",
        ),
        WindowsOptionalFeature => by(
            "DISM",
            "components against the local servicing store, which Windows signs",
        ),
        Chocolatey => by(
            "Chocolatey",
            "checksums when a package declares them — many community packages do not",
        ),
        Scoop => by("Scoop", "hashes declared in the app manifest"),

        // ── Vendor tools ────────────────────────────────────────────────────
        // Each of these hands the whole operation to a signed vendor
        // executable. What it checks internally is the vendor's business; the
        // honest statement is that the vendor's tool is in charge.
        DellCommandUpdate | DellFirmware => by("Dell Command Update", "Dell's own signed payloads"),
        HpImageAssistant | HpFirmware => by("HP Image Assistant", "HP's own signed payloads"),
        LenovoSystemUpdate | LenovoFirmware => {
            by("Lenovo System Update", "Lenovo's own signed payloads")
        }
        MsiCenter => by("MSI Center", "MSI's own payloads"),
        AsusArmoury => by("ASUS Armoury Crate", "ASUS's own payloads"),
        GigabyteControlCenter => by("Gigabyte Control Center", "Gigabyte's own payloads"),
        AcerCareCenter => by("Acer Care Center", "Acer's own payloads"),
        RazerSynapse => by("Razer Synapse", "Razer's own payloads"),
        NvidiaGeForceExperience => by("GeForce Experience", "NVIDIA's signed driver packages"),
        IntelDsa => by(
            "Intel Driver & Support Assistant",
            "Intel's signed driver packages",
        ),

        // ── Unix package managers ───────────────────────────────────────────
        Apt => by("apt", "repository metadata signed with the archive's GPG key"),
        Dnf => by("dnf", "package GPG signatures against the enabled repo keys"),
        Zypper => by("zypper", "package GPG signatures against the enabled repo keys"),
        Pacman => by("pacman", "package signatures against the pacman keyring"),
        Homebrew => by("Homebrew", "the SHA-256 declared in each formula"),
        Flatpak => by("Flatpak", "OSTree commit signatures on every remote"),
        Snap => by("snapd", "the assertions Canonical signs for each snap"),
        Nix => by(
            "Nix",
            "content-addressed store paths, so a changed byte is a different path"
        ),
        Fwupd => by(
            "fwupd",
            "LVFS signatures on firmware before flashing it",
        ),
        MacSoftwareUpdate | MacFirmware => by("Apple's softwareupdate", "Apple's own signatures"),
        AppImage => Verification::Unverified {
            why: "AppImage has no standard signing or checksum mechanism, and upstream \
                  projects publish digests inconsistently",
        },

        // ── Language ecosystems ─────────────────────────────────────────────
        Pip => by("pip", "the hashes PyPI serves alongside each wheel"),
        Npm => by("npm", "the integrity field in the registry's packument"),
        Cargo => by("cargo", "the checksum crates.io records for each crate"),
        Go => by(
            "the Go toolchain",
            "module hashes against the public checksum database",
        ),
        DotnetTool => by("NuGet", "package signatures from the configured feeds"),
        VscodeExtension => by("VS Code", "Marketplace signatures on each extension"),
        JetbrainsPlugin => by("the JetBrains IDE", "Marketplace signatures on each plugin"),
        PowerShellModule => by(
            "PowerShell Gallery",
            "the catalog hash it publishes — note that gallery modules are frequently unsigned",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_kind_has_a_verification_story() {
        for kind in BackendKind::ALL {
            let v = verification_of(*kind);
            assert!(!v.label().is_empty(), "{kind} has an empty label");
            assert!(v.detail().len() > 20, "{kind} has a stub detail");
        }
    }

    #[test]
    fn the_label_names_who_did_the_checking() {
        let v = verification_of(BackendKind::Winget);
        assert_eq!(v.label(), "verified by winget");
        assert!(v.is_verified_by_someone());
    }

    /// The whole point of the module: Odysync only claims a check it runs
    /// itself, which is exactly the backends that download their own payload.
    #[test]
    fn only_backends_that_download_their_own_payload_claim_odysync_verified_it() {
        for kind in BackendKind::ALL {
            let v = verification_of(*kind);
            let claims_odysync = v.label().to_ascii_lowercase().contains("odysync")
                || v.detail()
                    .to_ascii_lowercase()
                    .contains("odysync downloads");
            let downloads_itself = matches!(kind, BackendKind::Direct | BackendKind::NvidiaGpu);
            assert_eq!(
                claims_odysync, downloads_itself,
                "{kind}: claims Odysync verified it = {claims_odysync}"
            );
        }
    }

    #[test]
    fn appimage_is_reported_as_unverified_with_a_reason() {
        let v = verification_of(BackendKind::AppImage);
        assert!(!v.is_verified_by_someone());
        assert_eq!(v.label(), "not verified");
        assert!(v.detail().contains("no standard signing"));
    }

    /// These backends shell out to winget, so claiming a separate guarantee
    /// for them would be inventing one.
    #[test]
    fn winget_delivered_backends_inherit_wingets_guarantee() {
        let winget = verification_of(BackendKind::Winget);
        for kind in [
            BackendKind::AmdGpu,
            BackendKind::IntelGpu,
            BackendKind::QualcommGpu,
            BackendKind::VirtualizationGuest,
        ] {
            assert_eq!(verification_of(kind), winget, "{kind} diverged from winget");
        }
    }
}
