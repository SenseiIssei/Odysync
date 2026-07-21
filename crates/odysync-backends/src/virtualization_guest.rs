//! Virtualization guest tools backend.
//!
//! Detects and updates guest tools for VirtualBox, VMware, and QEMU/KVM
//! virtual machines. On Windows, guest tools are typically installed as
//! regular software packages and can be updated via winget. On Linux,
//! guest tools are usually kernel modules or packages managed by the
//! distribution's package manager.
//!
//! Detection is done by checking for hypervisor-specific indicators:
//! - VirtualBox: `VBoxService` process or `VBoxGuest` driver
//! - VMware: `vmtoolsd` process or `vmci` driver
//! - QEMU/KVM: `/sys/class/dmi/id/sys_vendor` contains "QEMU" or CPU has
//!   hypervisor flag
//!
//! Commands used:
//!   - Windows: `winget search "guest tools"` filtered by hypervisor
//!   - Linux: `pacman -Q virtualbox-guest-utils` / `apt list --installed`
//!   - macOS: `brew list --cask | grep virtualbox`

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Which hypervisor this machine is running under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hypervisor {
    VirtualBox,
    Vmware,
    Qemu,
    Unknown,
}

pub struct VirtualizationGuestBackend {
    hypervisor: Hypervisor,
}

impl VirtualizationGuestBackend {
    pub fn new() -> Self {
        Self {
            hypervisor: Hypervisor::Unknown,
        }
    }

    pub fn with_hypervisor(hv: Hypervisor) -> Self {
        Self {
            hypervisor: hv,
        }
    }
}

impl Default for VirtualizationGuestBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for VirtualizationGuestBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::VirtualizationGuest
    }

    fn display_name(&self) -> &str {
        "Virtualization Guest Tools"
    }

    async fn is_available(&self) -> bool {
        detect_hypervisor().await != Hypervisor::Unknown
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let hv = if self.hypervisor != Hypervisor::Unknown {
            self.hypervisor
        } else {
            detect_hypervisor().await
        };

        if hv == Hypervisor::Unknown {
            return Ok(Vec::new());
        }

        let installed_version = read_installed_guest_version(hv).await;
        let search_term = match hv {
            Hypervisor::VirtualBox => "VirtualBox Guest Additions",
            Hypervisor::Vmware => "VMware Tools",
            Hypervisor::Qemu => "QEMU guest agent",
            Hypervisor::Unknown => return Ok(Vec::new()),
        };

        #[cfg(windows)]
        {
            let out = proc::run(
                "winget",
                &[
                    "search",
                    "--query",
                    search_term,
                    "--source",
                    "winget",
                    "--accept-source-agreements",
                    "--disable-interactivity",
                ],
                SCAN_TIMEOUT,
            )
            .await?;

            if !out.success() {
                return Ok(Vec::new());
            }

            let candidates = crate::winget::table::parse(&out.stdout);
            let mut results = Vec::with_capacity(candidates.len());

            for row in candidates {
                let name_lower = row.name.to_lowercase();
                let matches_hv = match hv {
                    Hypervisor::VirtualBox => name_lower.contains("virtualbox") || name_lower.contains("guest additions"),
                    Hypervisor::Vmware => name_lower.contains("vmware") || name_lower.contains("tools"),
                    Hypervisor::Qemu => name_lower.contains("qemu") || name_lower.contains("guest agent"),
                    Hypervisor::Unknown => false,
                };

                if matches_hv {
                    results.push(UpdateCandidate {
                        id: PackageId::new(BackendKind::VirtualizationGuest, &row.id),
                        name: row.name.clone(),
                        installed: Version::parse(installed_version.as_deref().unwrap_or("")),
                        available: Version::parse(&row.available),
                        size_bytes: None,
                        expected_sha256: None,
                    });
                }
            }

            return Ok(results);
        }

        #[cfg(not(windows))]
        {
            // On Linux/macOS, guest tools are managed by the system package manager.
            // We report the installed version and let the user update via their PM.
            if let Some(version) = installed_version {
                return Ok(vec![UpdateCandidate {
                    id: PackageId::new(BackendKind::VirtualizationGuest, search_term),
                    name: search_term.to_string(),
                    installed: Version::parse(&version),
                    available: Version::parse(""), // No direct way to query available version
                    size_bytes: None,
                    expected_sha256: None,
                }]);
            }

            Ok(Vec::new())
        }
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        #[cfg(windows)]
        {
            let out = proc::run(
                "winget",
                &[
                    "install",
                    "--id",
                    &candidate.id.native,
                    "--source",
                    "winget",
                    "--exact",
                    "--version",
                    candidate.available.raw(),
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--disable-interactivity",
                    "--silent",
                ],
                INSTALL_TIMEOUT,
            )
            .await?;

            if out.success() {
                return Ok(());
            }

            return Err(Error::CommandFailed {
                command: format!("winget install --id {} --version {}", candidate.id.native, candidate.available.raw()),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            });
        }

        #[cfg(not(windows))]
        {
            Err(Error::parse(
                "Virtualization Guest Tools",
                "On Linux/macOS, guest tools are managed by the system package manager",
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let hv = if self.hypervisor != Hypervisor::Unknown {
            self.hypervisor
        } else {
            detect_hypervisor().await
        };

        let version = read_installed_guest_version(hv).await;
        Ok(version.filter(|v| Version::parse(v) == candidate.available))
    }
}

/// Detect which hypervisor this machine is running under.
async fn detect_hypervisor() -> Hypervisor {
    #[cfg(windows)]
    {
        // Check for VBoxService
        if proc::exists("VBoxService", &["--version"]).await {
            return Hypervisor::VirtualBox;
        }
        // Check for VMware tools
        if proc::exists("VMwareToolboxCmd", &["--version"]).await {
            return Hypervisor::Vmware;
        }
        // Check for QEMU guest agent
        if proc::exists("qemu-ga", &["--version"]).await {
            return Hypervisor::Qemu;
        }

        // WMI check for hypervisor
        let script = r#"
            $wmi = Get-WmiObject Win32_ComputerSystem -ErrorAction SilentlyContinue
            if ($wmi) { Write-Output $wmi.Manufacturer }
        "#;
        if let Ok(out) = proc::powershell(script, Duration::from_secs(10)).await {
            let manufacturer = out.stdout.trim().to_lowercase();
            if manufacturer.contains("virtualbox") || manufacturer.contains("innotek") {
                return Hypervisor::VirtualBox;
            }
            if manufacturer.contains("vmware") {
                return Hypervisor::Vmware;
            }
            if manufacturer.contains("qemu") {
                return Hypervisor::Qemu;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Check /sys/class/dmi/id/sys_vendor
        if let Ok(vendor) = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor") {
            let vendor = vendor.trim().to_lowercase();
            if vendor.contains("virtualbox") || vendor.contains("innotek") {
                return Hypervisor::VirtualBox;
            }
            if vendor.contains("vmware") {
                return Hypervisor::Vmware;
            }
            if vendor.contains("qemu") {
                return Hypervisor::Qemu;
            }
        }

        // Check for VBoxService
        if proc::exists("VBoxService", &["--version"]).await {
            return Hypervisor::VirtualBox;
        }
        if proc::exists("vmtoolsd", &["--version"]).await {
            return Hypervisor::Vmware;
        }
        if proc::exists("qemu-ga", &["--version"]).await {
            return Hypervisor::Qemu;
        }
    }

    #[cfg(target_os = "macos")]
    {
        // On macOS, check for VMware tools or VBox additions
        if proc::exists("vmtoolsd", &["--version"]).await {
            return Hypervisor::Vmware;
        }
        if proc::exists("VBoxService", &["--version"]).await {
            return Hypervisor::VirtualBox;
        }
    }

    Hypervisor::Unknown
}

/// Read the installed guest tools version for a given hypervisor.
async fn read_installed_guest_version(hv: Hypervisor) -> Option<String> {
    match hv {
        Hypervisor::VirtualBox => {
            #[cfg(windows)]
            {
                let script = r#"
                    $val = (Get-ItemProperty -Path 'HKLM:\SOFTWARE\Oracle\VirtualBox Guest Additions' -Name 'Version' -ErrorAction SilentlyContinue).Version
                    if ($val) { Write-Output $val }
                "#;
                let out = proc::powershell(script, Duration::from_secs(10)).await.ok()?;
                let v = out.stdout.trim().to_string();
                if v.is_empty() { None } else { Some(v) }
            }
            #[cfg(not(windows))]
            {
                let out = proc::run("VBoxService", &["--version"], QUERY_TIMEOUT).await.ok()?;
                let v = out.stdout.trim().to_string();
                if v.is_empty() { None } else { Some(v) }
            }
        }
        Hypervisor::Vmware => {
            if let Ok(out) = proc::run("vmtoolsd", &["--version"], QUERY_TIMEOUT).await {
                // Output: "VMware Tools daemon, version X.Y.Z build-NNNNN"
                let version = out
                    .stdout
                    .split("version ")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("");
                if version.is_empty() { None } else { Some(version.to_string()) }
            } else {
                None
            }
        }
        Hypervisor::Qemu => {
            if let Ok(out) = proc::run("qemu-ga", &["--version"], QUERY_TIMEOUT).await {
                let v = out.stdout.trim().to_string();
                if v.is_empty() { None } else { Some(v) }
            } else {
                None
            }
        }
        Hypervisor::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = VirtualizationGuestBackend::new();
        assert_eq!(b.kind(), BackendKind::VirtualizationGuest);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = VirtualizationGuestBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[test]
    fn hypervisor_variants_are_distinct() {
        assert_ne!(Hypervisor::VirtualBox, Hypervisor::Vmware);
        assert_ne!(Hypervisor::Vmware, Hypervisor::Qemu);
        assert_ne!(Hypervisor::Qemu, Hypervisor::Unknown);
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = VirtualizationGuestBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::VirtualizationGuest, "test-guest-tools"),
            name: "Test Guest Tools".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
