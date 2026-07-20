//! Core domain types shared by every backend and front-end.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::version::Version;

/// Which package manager / update mechanism a package came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    /// Windows: winget / Windows Package Manager.
    Winget,
    /// Windows: Microsoft Store packages (must run unelevated).
    MsStore,
    /// Windows: driver and firmware updates via Windows Update.
    WindowsDrivers,
    /// macOS: Homebrew formulae and casks.
    Homebrew,
    /// macOS: `softwareupdate` system updates.
    MacSoftwareUpdate,
    /// Linux: Debian/Ubuntu apt.
    Apt,
    /// Linux: Fedora/RHEL dnf.
    Dnf,
    /// Linux: Arch pacman.
    Pacman,
    /// Linux: Flatpak.
    Flatpak,
    /// Windows: NVIDIA GPU driver updates.
    NvidiaGpu,
    /// Windows: AMD GPU driver updates.
    AmdGpu,
    /// Windows: Intel GPU/Arc driver updates.
    IntelGpu,
    /// Windows: Dell Command Update (dcu-cli.exe).
    DellCommandUpdate,
    /// Windows: HP Image Assistant.
    HpImageAssistant,
    /// Windows: Lenovo System Update / SUHelper.
    LenovoSystemUpdate,
    /// Windows: MSI Center (informational + Windows Update fallback).
    MsiCenter,
    /// Linux: fwupd / LVFS firmware updates.
    Fwupd,
    /// macOS: firmware and system updates via softwareupdate.
    MacFirmware,
    /// Linux: Snap packages.
    Snap,
    /// Linux: SUSE/openSUSE zypper.
    Zypper,
    /// Windows: Chocolatey package manager.
    Chocolatey,
    /// Windows: Scoop package manager (user-scoped).
    Scoop,
    /// Linux + macOS: Nix package manager.
    Nix,
    /// Linux: AppImage updates.
    AppImage,
    /// Windows: ASUS Armoury Crate (informational).
    AsusArmoury,
    /// Windows: Gigabyte Control Center (informational).
    GigabyteControlCenter,
    /// Windows: Acer Care Center (informational).
    AcerCareCenter,
    /// Windows: Razer Synapse (informational).
    RazerSynapse,
    /// Windows: Qualcomm Adreno GPU driver updates.
    QualcommGpu,
    /// Cross-platform: Virtualization guest tools (VBox/VMware/QEMU).
    VirtualizationGuest,
}

impl BackendKind {
    /// Stable machine-readable name, used in config files and the CLI.
    pub fn id(&self) -> &'static str {
        match self {
            BackendKind::Winget => "winget",
            BackendKind::MsStore => "msstore",
            BackendKind::WindowsDrivers => "windows-drivers",
            BackendKind::Homebrew => "homebrew",
            BackendKind::MacSoftwareUpdate => "softwareupdate",
            BackendKind::Apt => "apt",
            BackendKind::Dnf => "dnf",
            BackendKind::Pacman => "pacman",
            BackendKind::Flatpak => "flatpak",
            BackendKind::NvidiaGpu => "nvidia-gpu",
            BackendKind::AmdGpu => "amd-gpu",
            BackendKind::IntelGpu => "intel-gpu",
            BackendKind::DellCommandUpdate => "dell-command-update",
            BackendKind::HpImageAssistant => "hp-image-assistant",
            BackendKind::LenovoSystemUpdate => "lenovo-system-update",
            BackendKind::MsiCenter => "msi-center",
            BackendKind::Fwupd => "fwupd",
            BackendKind::MacFirmware => "mac-firmware",
            BackendKind::Snap => "snap",
            BackendKind::Zypper => "zypper",
            BackendKind::Chocolatey => "chocolatey",
            BackendKind::Scoop => "scoop",
            BackendKind::Nix => "nix",
            BackendKind::AppImage => "appimage",
            BackendKind::AsusArmoury => "asus-armoury",
            BackendKind::GigabyteControlCenter => "gigabyte-control-center",
            BackendKind::AcerCareCenter => "acer-care-center",
            BackendKind::RazerSynapse => "razer-synapse",
            BackendKind::QualcommGpu => "qualcomm-gpu",
            BackendKind::VirtualizationGuest => "virtualization-guest",
        }
    }

    /// Whether this backend needs elevated privileges to apply updates.
    pub fn requires_elevation(&self) -> bool {
        match self {
            BackendKind::WindowsDrivers
            | BackendKind::Apt
            | BackendKind::Dnf
            | BackendKind::Pacman
            | BackendKind::MacSoftwareUpdate
            | BackendKind::NvidiaGpu
            | BackendKind::AmdGpu
            | BackendKind::IntelGpu
            | BackendKind::DellCommandUpdate
            | BackendKind::HpImageAssistant
            | BackendKind::LenovoSystemUpdate
            | BackendKind::MsiCenter
            | BackendKind::Fwupd
            | BackendKind::MacFirmware
            | BackendKind::Snap
            | BackendKind::Zypper
            | BackendKind::Chocolatey
            | BackendKind::AsusArmoury
            | BackendKind::GigabyteControlCenter
            | BackendKind::AcerCareCenter
            | BackendKind::RazerSynapse
            | BackendKind::QualcommGpu
            | BackendKind::VirtualizationGuest => true,
            // winget machine-scope installs may prompt for UAC per package;
            // that is handled per-package, not as a blanket requirement.
            BackendKind::Winget => false,
            // Store apps break when run elevated.
            BackendKind::MsStore => false,
            BackendKind::Homebrew => false,
            BackendKind::Flatpak => false,
            BackendKind::Scoop => false,
            BackendKind::Nix => false,
            BackendKind::AppImage => false,
        }
    }

    /// Whether running this backend elevated actively breaks it.
    pub fn forbids_elevation(&self) -> bool {
        matches!(
            self,
            BackendKind::MsStore | BackendKind::Homebrew | BackendKind::Scoop
        )
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A globally unique handle for a package: backend + that backend's own id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub backend: BackendKind,
    /// The backend's native identifier, e.g. `Mozilla.Firefox` or `firefox`.
    pub native: String,
}

impl PackageId {
    pub fn new(backend: BackendKind, native: impl Into<String>) -> Self {
        Self {
            backend,
            native: native.into(),
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.backend, self.native)
    }
}

/// An installed package with an update available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCandidate {
    pub id: PackageId,
    /// Human-friendly name for display.
    pub name: String,
    pub installed: Version,
    pub available: Version,
    /// Approximate download size in bytes, when the backend reports one.
    pub size_bytes: Option<u64>,
    /// Expected SHA-256 of the installer, when the backend can supply it.
    pub expected_sha256: Option<String>,
}

/// Why the policy engine refused to act on a candidate.
///
/// Every variant here corresponds to a class of real-world breakage; they are
/// surfaced to the user rather than silently swallowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SkipReason {
    /// The installed version could not be parsed, so "newer" is unknowable.
    UnknownInstalledVersion,
    /// The offered version could not be parsed.
    UnknownAvailableVersion,
    /// The offered version is older than or equal to what is installed.
    NotAnUpgrade,
    /// The offered version is a beta/rc and stable-only is in force.
    PrereleaseBlocked { version: String },
    /// The user pinned this package to a version or held it entirely.
    Held { note: Option<String> },
    /// The package is on the user's exclusion list.
    Excluded,
    /// A Store app was selected while running elevated.
    RequiresUnelevated,
    /// The backend needs privileges the current process does not have.
    RequiresElevation,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::UnknownInstalledVersion => {
                f.write_str("installed version could not be determined")
            }
            SkipReason::UnknownAvailableVersion => {
                f.write_str("offered version could not be determined")
            }
            SkipReason::NotAnUpgrade => f.write_str("offered version is not newer"),
            SkipReason::PrereleaseBlocked { version } => {
                write!(f, "{version} is a pre-release and stable-only is enabled")
            }
            SkipReason::Held { note: Some(n) } => write!(f, "held by policy: {n}"),
            SkipReason::Held { note: None } => f.write_str("held by policy"),
            SkipReason::Excluded => f.write_str("excluded by configuration"),
            SkipReason::RequiresUnelevated => {
                f.write_str("Microsoft Store apps cannot be updated from an elevated process")
            }
            SkipReason::RequiresElevation => f.write_str("requires administrator privileges"),
        }
    }
}

/// The outcome of applying a single update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ApplyOutcome {
    /// Installed and the new version was confirmed on disk afterwards.
    Updated { from: String, to: String },
    /// The backend reported success but the version did not change; treated as
    /// a failure to converge, not a success.
    DidNotConverge { expected: String, actual: String },
    /// Verification failed before anything was installed.
    VerificationFailed { detail: String },
    /// The backend returned a non-zero exit code.
    Failed { detail: String },
    /// Skipped by policy; nothing was run.
    Skipped { reason: SkipReason },
}

impl ApplyOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ApplyOutcome::Updated { .. })
    }
}

/// A single update, paired with the result of running it through policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedUpdate {
    pub candidate: UpdateCandidate,
    /// `None` when the update is allowed to proceed.
    pub blocked_by: Option<SkipReason>,
}

impl PlannedUpdate {
    pub fn is_actionable(&self) -> bool {
        self.blocked_by.is_none()
    }
}
