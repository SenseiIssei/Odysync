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
    /// Windows: security and quality updates via the Windows Update Agent.
    WindowsUpdate,
    /// Windows: Microsoft Defender signature updates.
    WindowsDefenderUpdate,
    /// Windows: version upgrades and service packs. Opt-in only.
    WindowsFeatureUpdate,
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
    /// Cross-platform: Python pip packages.
    Pip,
    /// Cross-platform: Rust cargo install crates.
    Cargo,
    /// Cross-platform: Node.js npm global packages.
    Npm,
    /// Cross-platform: Go modules.
    Go,
    /// Cross-platform: .NET dotnet global tools.
    DotnetTool,
    /// Cross-platform: VS Code extensions.
    VscodeExtension,
    /// Cross-platform: PowerShell modules.
    PowerShellModule,
    /// Windows: NVIDIA GeForce Experience driver updates.
    NvidiaGeForceExperience,
    /// Windows: Intel Driver & Support Assistant.
    IntelDsa,
    /// Cross-platform: JetBrains IDE plugins.
    JetbrainsPlugin,
    /// Windows: Windows Optional Features.
    WindowsOptionalFeature,
    /// Windows: Dell firmware updates via dcu-cli.
    DellFirmware,
    /// Windows: HP firmware updates via HPIA.
    HpFirmware,
    /// Windows: Lenovo firmware updates via SUHelper.
    LenovoFirmware,
    /// Installers fetched straight from the publisher — GitHub releases or a
    /// vendor's own update feed — for apps found in the OS's list of installed
    /// programs. Needs no package manager at all, which is what lets Odysync
    /// work on a machine where winget is missing or disabled.
    Direct,
}

impl BackendKind {
    /// Every backend kind that exists, on any platform.
    ///
    /// This is the list [`BackendKind::from_id`] searches, and it exists so
    /// that front-ends do not each keep their own copy. The Tauri layer used
    /// to carry a hand-written `&str -> BackendKind` match; a kind added to
    /// this enum was simply absent from it, and holding or unholding such a
    /// package failed with "unknown backend" until somebody noticed.
    ///
    /// Kept in step with [`BackendKind::id`] by
    /// `every_kind_is_listed_in_all`, which fails if the two ever diverge.
    pub const ALL: &'static [BackendKind] = &[
        BackendKind::Winget,
        BackendKind::MsStore,
        BackendKind::WindowsDrivers,
        BackendKind::WindowsUpdate,
        BackendKind::WindowsDefenderUpdate,
        BackendKind::WindowsFeatureUpdate,
        BackendKind::Homebrew,
        BackendKind::MacSoftwareUpdate,
        BackendKind::Apt,
        BackendKind::Dnf,
        BackendKind::Pacman,
        BackendKind::Flatpak,
        BackendKind::NvidiaGpu,
        BackendKind::AmdGpu,
        BackendKind::IntelGpu,
        BackendKind::DellCommandUpdate,
        BackendKind::HpImageAssistant,
        BackendKind::LenovoSystemUpdate,
        BackendKind::MsiCenter,
        BackendKind::Fwupd,
        BackendKind::MacFirmware,
        BackendKind::Snap,
        BackendKind::Zypper,
        BackendKind::Chocolatey,
        BackendKind::Scoop,
        BackendKind::Nix,
        BackendKind::AppImage,
        BackendKind::AsusArmoury,
        BackendKind::GigabyteControlCenter,
        BackendKind::AcerCareCenter,
        BackendKind::RazerSynapse,
        BackendKind::QualcommGpu,
        BackendKind::VirtualizationGuest,
        BackendKind::Pip,
        BackendKind::Cargo,
        BackendKind::Npm,
        BackendKind::Go,
        BackendKind::DotnetTool,
        BackendKind::VscodeExtension,
        BackendKind::PowerShellModule,
        BackendKind::NvidiaGeForceExperience,
        BackendKind::IntelDsa,
        BackendKind::JetbrainsPlugin,
        BackendKind::WindowsOptionalFeature,
        BackendKind::DellFirmware,
        BackendKind::HpFirmware,
        BackendKind::LenovoFirmware,
        BackendKind::Direct,
    ];

    /// Parse the [`id`](BackendKind::id) form back into a kind.
    ///
    /// Case-insensitive, because config files are written by hand.
    pub fn from_id(id: &str) -> Option<BackendKind> {
        BackendKind::ALL
            .iter()
            .copied()
            .find(|k| k.id().eq_ignore_ascii_case(id.trim()))
    }

    /// Stable machine-readable name, used in config files and the CLI.
    pub fn id(&self) -> &'static str {
        match self {
            BackendKind::Winget => "winget",
            BackendKind::MsStore => "msstore",
            BackendKind::WindowsDrivers => "windows-drivers",
            BackendKind::WindowsUpdate => "windows-update",
            BackendKind::WindowsDefenderUpdate => "windows-defender-update",
            BackendKind::WindowsFeatureUpdate => "windows-feature-update",
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
            BackendKind::Pip => "pip",
            BackendKind::Cargo => "cargo",
            BackendKind::Npm => "npm",
            BackendKind::Go => "go",
            BackendKind::DotnetTool => "dotnet-tool",
            BackendKind::VscodeExtension => "vscode-extension",
            BackendKind::PowerShellModule => "powershell-module",
            BackendKind::NvidiaGeForceExperience => "nvidia-geforce-experience",
            BackendKind::IntelDsa => "intel-dsa",
            BackendKind::JetbrainsPlugin => "jetbrains-plugin",
            BackendKind::WindowsOptionalFeature => "windows-optional-feature",
            BackendKind::DellFirmware => "dell-firmware",
            BackendKind::HpFirmware => "hp-firmware",
            BackendKind::LenovoFirmware => "lenovo-firmware",
            BackendKind::Direct => "direct",
        }
    }

    /// Whether this backend needs elevated privileges to apply updates.
    pub fn requires_elevation(&self) -> bool {
        match self {
            BackendKind::WindowsDrivers
            | BackendKind::WindowsUpdate
            | BackendKind::WindowsDefenderUpdate
            | BackendKind::WindowsFeatureUpdate
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
            | BackendKind::VirtualizationGuest
            | BackendKind::NvidiaGeForceExperience
            | BackendKind::IntelDsa
            | BackendKind::WindowsOptionalFeature
            | BackendKind::DellFirmware
            | BackendKind::HpFirmware
            | BackendKind::LenovoFirmware => true,
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
            BackendKind::Pip => false,
            BackendKind::Cargo => false,
            BackendKind::Npm => false,
            BackendKind::Go => false,
            BackendKind::DotnetTool => false,
            BackendKind::VscodeExtension => false,
            BackendKind::PowerShellModule => false,
            BackendKind::JetbrainsPlugin => false,
            // Decided per package: a per-user install needs no rights, a
            // machine-wide one does. See [`UpdateCandidate::requires_elevation`].
            BackendKind::Direct => false,
        }
    }

    /// Whether running this backend elevated actively breaks it.
    pub fn forbids_elevation(&self) -> bool {
        matches!(
            self,
            BackendKind::MsStore | BackendKind::Homebrew | BackendKind::Scoop
        )
    }

    /// Whether this backend participates in a run the user has not configured.
    ///
    /// Almost everything does — the point of the tool is that it works out of
    /// the box. The exception is a backend whose updates change the machine in
    /// a way no background process should decide on: a Windows version upgrade
    /// replaces the operating system, takes an hour, and cannot be undone
    /// without the previous build still being on disk.
    ///
    /// This is deliberately *not* expressed as an entry in the default
    /// `disabled-backends` list. That list is serialised into every user's
    /// config file, so a default added later would never reach anyone who had
    /// already run the tool once. A method on the kind applies to everybody,
    /// including existing installs, and the user opts in by name through
    /// `enabled-backends`.
    ///
    /// winget and the Microsoft Store (which Odysync drives through winget)
    /// are also opt-in. Odysync has its own source for installed programs —
    /// [`BackendKind::Direct`] — and depending on winget by default meant a
    /// machine without it, or with a broken App Installer, silently got no
    /// software updates at all. Users who prefer winget enable it by name.
    pub fn enabled_by_default(&self) -> bool {
        !matches!(self, BackendKind::WindowsFeatureUpdate) && !self.delivered_through_winget()
    }

    /// Drivers, firmware and operating-system upgrades: the updates that can
    /// change how the hardware behaves, flicker the display, drop the network
    /// for a moment or need a restart.
    ///
    /// Odysync's unattended care — `odysync care`, a schedule, the one-click
    /// fix in the app — leaves these for a person to start, with a backup and
    /// the machine in front of them. `odysync apply` and the Drivers page
    /// install them as before.
    pub fn needs_attended_install(&self) -> bool {
        matches!(
            self,
            BackendKind::WindowsDrivers
                | BackendKind::WindowsFeatureUpdate
                | BackendKind::NvidiaGpu
                | BackendKind::NvidiaGeForceExperience
                | BackendKind::AmdGpu
                | BackendKind::IntelGpu
                | BackendKind::IntelDsa
                | BackendKind::QualcommGpu
                | BackendKind::DellCommandUpdate
                | BackendKind::DellFirmware
                | BackendKind::HpImageAssistant
                | BackendKind::HpFirmware
                | BackendKind::LenovoSystemUpdate
                | BackendKind::LenovoFirmware
                | BackendKind::MsiCenter
                | BackendKind::Fwupd
                | BackendKind::MacFirmware
        )
    }

    /// Whether this backend shells out to winget to do its work.
    ///
    /// These are opt-in together with winget itself: enabling them without
    /// winget would make every scan fail, and enabling them by default would
    /// bring back the dependency on winget that [`BackendKind::Direct`] and
    /// the vendor-direct NVIDIA backend exist to remove. AMD, Intel and
    /// Qualcomm publish no machine-readable driver feed, so for those GPUs the
    /// default path is the Windows Update driver backend.
    pub fn delivered_through_winget(&self) -> bool {
        matches!(
            self,
            BackendKind::Winget
                | BackendKind::MsStore
                | BackendKind::AmdGpu
                | BackendKind::IntelGpu
                | BackendKind::QualcommGpu
                | BackendKind::VirtualizationGuest
        )
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A globally unique handle for a package: backend + that backend's own id.
///
/// Equality and hashing deliberately ignore [`alias`](PackageId::alias) — see
/// the manual impls below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageId {
    pub backend: BackendKind,
    /// The backend's native identifier, e.g. `Mozilla.Firefox` or `firefox`.
    pub native: String,
    /// A second name the user is more likely to know this package by.
    ///
    /// Exists for Windows Update, whose native id has to be
    /// `<update-guid>.<revision>` — that is what the Update Agent accepts, and
    /// pinning the exact revision is required by the backend contract. Nobody
    /// holds an update by GUID, though; they hold `KB5101650`. The alias lets
    /// [`crate::policy::Policy`] match either.
    ///
    /// Not part of the package's identity, only an extra way to name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Identity is `(backend, native)` and nothing else.
///
/// Deriving `PartialEq` would have folded `alias` into equality, so the same
/// package would compare unequal to itself depending on whether the alias
/// happened to be filled in — and `PackageId` is used as a `HashMap` key and
/// to correlate a scan result with its apply outcome. An alias is a label, not
/// a distinguishing feature.
impl PartialEq for PackageId {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend && self.native == other.native
    }
}

impl Eq for PackageId {}

impl std::hash::Hash for PackageId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.backend.hash(state);
        self.native.hash(state);
    }
}

impl PackageId {
    pub fn new(backend: BackendKind, native: impl Into<String>) -> Self {
        Self {
            backend,
            native: native.into(),
            alias: None,
        }
    }

    /// Add a friendlier name the user can also hold or exclude this package by.
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        let alias = alias.into();
        // An empty alias would make `Policy::matches` compare against "", and
        // a hold pattern that trims to nothing would then match everything.
        self.alias = if alias.trim().is_empty() {
            None
        } else {
            Some(alias)
        };
        self
    }

    /// Validate the native package ID against shell injection and path traversal.
    ///
    /// Returns `Ok(())` if the ID is safe, or an `Error` describing the violation.
    /// This should be called before passing the ID to any shell command.
    pub fn validate(&self) -> crate::error::Result<()> {
        let id = &self.native;

        if id.is_empty() {
            return Err(crate::error::Error::invalid_package_id(
                id,
                "package ID is empty",
            ));
        }

        if id.len() > 256 {
            return Err(crate::error::Error::invalid_package_id(
                id,
                "package ID exceeds 256 characters",
            ));
        }

        // Shell metacharacters that could enable command injection.
        const FORBIDDEN: &[char] = &[
            ';', '&', '|', '`', '$', '(', ')', '{', '}', '<', '>', '\n', '\r', '\0',
        ];

        if let Some(c) = id.chars().find(|c| FORBIDDEN.contains(c)) {
            return Err(crate::error::Error::invalid_package_id(
                id,
                format!("package ID contains forbidden character: {:?}", c),
            ));
        }

        // Reject path traversal patterns.
        if id.contains("..") {
            return Err(crate::error::Error::invalid_package_id(
                id,
                "package ID contains path traversal sequence '..'",
            ));
        }

        // Reject absolute paths (could be used to execute arbitrary binaries).
        if id.starts_with('/') || (cfg!(windows) && id.len() >= 2 && id.as_bytes()[1] == b':') {
            return Err(crate::error::Error::invalid_package_id(
                id,
                "package ID must not be an absolute path",
            ));
        }

        Ok(())
    }

    /// Like `validate` but returns a bool for convenience in tests.
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
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

/// A package a backend reports as currently installed, regardless of whether
/// an update is available for it.
/// Native-id prefix for a [`BackendKind::Direct`] package installed for every
/// user, whose installer therefore needs administrator rights.
pub const DIRECT_MACHINE_SCOPE: &str = "machine:";
/// Native-id prefix for a [`BackendKind::Direct`] package installed for the
/// current user only.
pub const DIRECT_USER_SCOPE: &str = "user:";

impl UpdateCandidate {
    /// Whether applying this particular update needs administrator rights.
    ///
    /// For almost every backend this is a property of the backend. The
    /// exception is [`BackendKind::Direct`], where the same app can be
    /// installed per-user (no rights needed) or machine-wide (rights needed),
    /// and the scope is carried in the native id so that it survives the trip
    /// through the GUI and back unchanged.
    pub fn requires_elevation(&self) -> bool {
        match self.id.backend {
            BackendKind::Direct => self.id.native.starts_with(DIRECT_MACHINE_SCOPE),
            other => other.requires_elevation(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: PackageId,
    pub name: String,
    /// Version string exactly as the package manager reported it.
    pub version: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drivers_firmware_and_os_upgrades_wait_for_a_person() {
        for kind in [
            BackendKind::WindowsDrivers,
            BackendKind::NvidiaGpu,
            BackendKind::Fwupd,
            BackendKind::MacFirmware,
            BackendKind::WindowsFeatureUpdate,
        ] {
            assert!(kind.needs_attended_install(), "{kind:?}");
        }
        for kind in [
            BackendKind::Direct,
            BackendKind::WindowsUpdate,
            BackendKind::WindowsDefenderUpdate,
            BackendKind::Homebrew,
            BackendKind::Apt,
            BackendKind::Pip,
        ] {
            assert!(!kind.needs_attended_install(), "{kind:?}");
        }
    }

    /// Fails to compile when a variant is added to [`BackendKind`].
    ///
    /// `ALL` cannot be derived without a macro or a dependency, so the next
    /// best thing is to make forgetting it impossible to miss: this match is
    /// exhaustive, so a new variant breaks the build here, three lines from
    /// the assertion that `ALL` must contain it.
    fn assert_variant_is_known(kind: BackendKind) {
        match kind {
            BackendKind::Winget
            | BackendKind::MsStore
            | BackendKind::WindowsDrivers
            | BackendKind::WindowsUpdate
            | BackendKind::WindowsDefenderUpdate
            | BackendKind::WindowsFeatureUpdate
            | BackendKind::Homebrew
            | BackendKind::MacSoftwareUpdate
            | BackendKind::Apt
            | BackendKind::Dnf
            | BackendKind::Pacman
            | BackendKind::Flatpak
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
            | BackendKind::Scoop
            | BackendKind::Nix
            | BackendKind::AppImage
            | BackendKind::AsusArmoury
            | BackendKind::GigabyteControlCenter
            | BackendKind::AcerCareCenter
            | BackendKind::RazerSynapse
            | BackendKind::QualcommGpu
            | BackendKind::VirtualizationGuest
            | BackendKind::Pip
            | BackendKind::Cargo
            | BackendKind::Npm
            | BackendKind::Go
            | BackendKind::DotnetTool
            | BackendKind::VscodeExtension
            | BackendKind::PowerShellModule
            | BackendKind::NvidiaGeForceExperience
            | BackendKind::IntelDsa
            | BackendKind::JetbrainsPlugin
            | BackendKind::WindowsOptionalFeature
            | BackendKind::DellFirmware
            | BackendKind::HpFirmware
            | BackendKind::LenovoFirmware
            | BackendKind::Direct => {}
        }
        assert!(
            BackendKind::ALL.contains(&kind),
            "{kind} is missing from BackendKind::ALL"
        );
    }

    #[test]
    fn an_alias_does_not_change_a_packages_identity() {
        // `PackageId` is a HashMap key and correlates a scan result with its
        // apply outcome. Deriving equality would have made the same package
        // unequal to itself once an alias was attached.
        let bare = PackageId::new(BackendKind::WindowsUpdate, "guid.3");
        let aliased = PackageId::new(BackendKind::WindowsUpdate, "guid.3").with_alias("KB5101650");

        assert_eq!(bare, aliased);

        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let hash_of = |id: &PackageId| {
            let mut h = DefaultHasher::new();
            id.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_of(&bare), hash_of(&aliased));

        let mut map = std::collections::HashMap::new();
        map.insert(bare.clone(), "scanned");
        assert_eq!(map.get(&aliased), Some(&"scanned"));
    }

    #[test]
    fn a_blank_alias_is_dropped_rather_than_stored() {
        // An empty alias would be compared against a trimmed hold pattern,
        // and an empty pattern would then match everything.
        assert_eq!(
            PackageId::new(BackendKind::Winget, "x")
                .with_alias("   ")
                .alias,
            None
        );
        assert_eq!(
            PackageId::new(BackendKind::Winget, "x")
                .with_alias("KB1")
                .alias
                .as_deref(),
            Some("KB1")
        );
    }

    #[test]
    fn an_absent_alias_is_omitted_from_the_serialised_form() {
        // Config and report files are read by humans; a null on every package
        // is noise, and older files without the field must still load.
        let json = serde_json::to_string(&PackageId::new(BackendKind::Winget, "x")).unwrap();
        assert!(!json.contains("alias"), "{json}");

        let back: PackageId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alias, None);
    }

    #[test]
    fn every_kind_is_listed_in_all() {
        for kind in BackendKind::ALL {
            assert_variant_is_known(*kind);
        }
    }

    #[test]
    fn every_id_is_unique() {
        let mut ids: Vec<&str> = BackendKind::ALL.iter().map(|k| k.id()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "two backend kinds share an id");
    }

    #[test]
    fn ids_round_trip_through_from_id() {
        for kind in BackendKind::ALL {
            assert_eq!(BackendKind::from_id(kind.id()), Some(*kind));
        }
        // Config files are hand-edited, so parsing tolerates case and padding.
        assert_eq!(
            BackendKind::from_id("  Windows-Update "),
            Some(BackendKind::WindowsUpdate)
        );
        assert_eq!(BackendKind::from_id("not-a-backend"), None);
    }

    #[test]
    fn valid_package_ids_pass_validation() {
        assert!(PackageId::new(BackendKind::Winget, "Mozilla.Firefox").is_valid());
        assert!(PackageId::new(BackendKind::Apt, "firefox").is_valid());
        assert!(PackageId::new(BackendKind::Flatpak, "org.mozilla.firefox").is_valid());
        assert!(PackageId::new(BackendKind::Snap, "firefox").is_valid());
        assert!(PackageId::new(BackendKind::Chocolatey, "7zip").is_valid());
    }

    #[test]
    fn empty_id_fails_validation() {
        assert!(!PackageId::new(BackendKind::Apt, "").is_valid());
    }

    #[test]
    fn shell_metacharacters_fail_validation() {
        assert!(!PackageId::new(BackendKind::Apt, "test; rm -rf /").is_valid());
        assert!(!PackageId::new(BackendKind::Apt, "test && echo pwned").is_valid());
        assert!(!PackageId::new(BackendKind::Apt, "test | cat").is_valid());
        assert!(!PackageId::new(BackendKind::Apt, "$(whoami)").is_valid());
        assert!(!PackageId::new(BackendKind::Apt, "`whoami`").is_valid());
    }

    #[test]
    fn path_traversal_fails_validation() {
        assert!(!PackageId::new(BackendKind::Apt, "../etc/passwd").is_valid());
        assert!(!PackageId::new(BackendKind::Apt, "foo/../../bar").is_valid());
    }

    #[test]
    fn absolute_path_fails_validation() {
        assert!(!PackageId::new(BackendKind::Apt, "/usr/bin/firefox").is_valid());
    }

    #[cfg(windows)]
    #[test]
    fn windows_absolute_path_fails_validation() {
        assert!(!PackageId::new(BackendKind::Winget, "C:\\Windows\\System32").is_valid());
    }

    #[test]
    fn oversized_id_fails_validation() {
        let long_id = "a".repeat(257);
        assert!(!PackageId::new(BackendKind::Apt, &long_id).is_valid());
    }

    #[test]
    fn validate_returns_descriptive_error() {
        let id = PackageId::new(BackendKind::Apt, "test; rm -rf /");
        let err = id.validate().unwrap_err();
        assert!(matches!(err, crate::error::Error::InvalidPackageId { .. }));
        assert!(err.to_string().contains("forbidden character"));
    }
}
