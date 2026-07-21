//! Package-manager integrations and host detection.
//!
//! Backends are discovered at runtime rather than chosen at compile time: a
//! single binary ships every integration for its platform and simply reports
//! "not available" for the ones the machine does not have. Adding a package
//! manager means implementing [`Backend`] and adding one line to
//! [`detect_backends`] — nothing else in the codebase changes.

pub mod appimage;
pub mod apt;
pub mod autostart;
pub mod chocolatey;
pub mod diagnostics;
pub mod dnf;
pub mod firmware_backends;
pub mod flatpak;
pub mod fwupd;
// GPU driver updates are delivered through winget, so these backends reference
// `crate::winget`, which only exists on Windows. Every use site in
// `all_backends` is already `cfg(windows)`; the module declaration was not, so
// the crate failed to compile on Linux and macOS.
#[cfg(windows)]
pub mod gpu;
pub mod homebrew;
pub mod lang_backends;
pub mod mac_firmware;
pub mod mac_software_update;
pub mod maintenance;
pub mod nix;
pub mod notifications;
#[cfg(windows)]
pub mod oem;
pub mod offline;
pub mod pacman;
pub mod scheduler;
pub mod scoop;
pub mod security;
pub mod snap;
// Same as `gpu`: installs the guest additions through winget, so it cannot
// compile without `crate::winget`. Its only use site is already cfg(windows).
#[cfg(windows)]
pub mod virtualization_guest;
#[cfg(windows)]
pub mod windows_drivers;
#[cfg(windows)]
pub mod winget;
pub mod zypper;

use odysync_core::backend::Backend;
use odysync_core::config::Config;

/// Every backend compiled into this build, whether usable here or not.
fn all_backends() -> Vec<Box<dyn Backend>> {
    let mut v: Vec<Box<dyn Backend>> = Vec::new();

    #[cfg(windows)]
    {
        v.push(Box::new(winget::WingetBackend::new()));
        v.push(Box::new(winget::WingetBackend::store()));
        v.push(Box::new(windows_drivers::WindowsDriverBackend::new()));
        v.push(Box::new(gpu::nvidia_gpu::NvidiaGpuBackend::new()));
        v.push(Box::new(gpu::amd_gpu::AmdGpuBackend::new()));
        v.push(Box::new(gpu::intel_gpu::IntelGpuBackend::new()));
        v.push(Box::new(gpu::qualcomm_gpu::QualcommGpuBackend::new()));
        v.push(Box::new(
            oem::dell_command_update::DellCommandUpdateBackend::new(),
        ));
        v.push(Box::new(
            oem::hp_image_assistant::HpImageAssistantBackend::new(),
        ));
        v.push(Box::new(
            oem::lenovo_system_update::LenovoSystemUpdateBackend::new(),
        ));
        v.push(Box::new(oem::msi_center::MsiCenterBackend::new()));
        v.push(Box::new(oem::asus_armoury::AsusArmouryBackend::new()));
        v.push(Box::new(
            oem::gigabyte_control_center::GigabyteControlCenterBackend::new(),
        ));
        v.push(Box::new(oem::acer_care_center::AcerCareCenterBackend::new()));
        v.push(Box::new(oem::razer_synapse::RazerSynapseBackend::new()));
        v.push(Box::new(chocolatey::ChocolateyBackend::new()));
        v.push(Box::new(scoop::ScoopBackend::new()));
        v.push(Box::new(
            virtualization_guest::VirtualizationGuestBackend::new(),
        ));
    }

    // Language runtime package managers (cross-platform)
    v.push(Box::new(lang_backends::PipBackend::new()));
    v.push(Box::new(lang_backends::CargoBackend::new()));
    v.push(Box::new(lang_backends::NpmBackend::new()));
    v.push(Box::new(lang_backends::GoBackend::new()));
    v.push(Box::new(lang_backends::DotnetToolBackend::new()));
    v.push(Box::new(lang_backends::VscodeExtensionBackend::new()));
    v.push(Box::new(lang_backends::JetbrainsPluginBackend::new()));

    #[cfg(windows)]
    {
        v.push(Box::new(lang_backends::PowerShellModuleBackend::new()));
        v.push(Box::new(lang_backends::WindowsOptionalFeatureBackend::new()));
        v.push(Box::new(
            lang_backends::NvidiaGeForceExperienceBackend::new(),
        ));
        v.push(Box::new(lang_backends::IntelDsaBackend::new()));
        v.push(Box::new(firmware_backends::DellFirmwareBackend::new()));
        v.push(Box::new(firmware_backends::HpFirmwareBackend::new()));
        v.push(Box::new(firmware_backends::LenovoFirmwareBackend::new()));
    }

    // Homebrew also runs on Linux, so it is not gated to macOS.
    v.push(Box::new(homebrew::HomebrewBackend::new()));
    v.push(Box::new(apt::AptBackend::new()));
    v.push(Box::new(flatpak::FlatpakBackend::new()));
    v.push(Box::new(nix::NixBackend::new()));

    #[cfg(target_os = "linux")]
    {
        v.push(Box::new(dnf::DnfBackend::new()));
        v.push(Box::new(pacman::PacmanBackend::new()));
        v.push(Box::new(fwupd::FwupdBackend::new()));
        v.push(Box::new(snap::SnapBackend::new()));
        v.push(Box::new(zypper::ZypperBackend::new()));
        v.push(Box::new(appimage::AppImageBackend::new()));
    }

    #[cfg(target_os = "macos")]
    {
        v.push(Box::new(mac_firmware::MacFirmwareBackend::new()));
        v.push(Box::new(
            mac_software_update::MacSoftwareUpdateBackend::new(),
        ));
    }

    v
}

/// One compiled-in backend paired with the result of probing this machine.
pub struct ProbedBackend {
    pub backend: Box<dyn Backend>,
    /// Whether the underlying tool is present and usable here.
    pub available: bool,
    /// Whether the user has disabled it in config.
    pub enabled: bool,
}

impl ProbedBackend {
    /// Usable for a scan or apply: present on the machine and not disabled.
    pub fn usable(&self) -> bool {
        self.available && self.enabled
    }
}

/// Probe every compiled-in backend, reporting availability for all of them.
///
/// Availability probes run concurrently — each shells out to a package manager
/// and they are independent, so doing them in sequence would make startup as
/// slow as the sum of every probe.
///
/// Unlike [`detect_backends`], this keeps the backends that are *not* present,
/// so a UI can show "winget: not installed" rather than silently omitting it.
/// It is also the only place availability is probed: callers should probe once
/// and cache, since a full sweep is roughly one process spawn per backend.
pub async fn probe_backends(config: &Config) -> Vec<ProbedBackend> {
    let candidates = all_backends();
    let results = futures::future::join_all(candidates.iter().map(|b| b.is_available())).await;

    candidates
        .into_iter()
        .zip(results)
        .map(|(backend, available)| {
            let enabled = config.backend_enabled(backend.kind());
            if !available {
                tracing::debug!(backend = %backend.kind(), "not available on this host");
            }
            ProbedBackend {
                backend,
                available,
                enabled,
            }
        })
        .collect()
}

/// Backends that are present on this machine and enabled in `config`.
pub async fn detect_backends(config: &Config) -> Vec<Box<dyn Backend>> {
    probe_backends(config)
        .await
        .into_iter()
        .filter(ProbedBackend::usable)
        .map(|p| p.backend)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::model::BackendKind;

    #[test]
    fn the_build_includes_the_expected_backends_for_this_platform() {
        let kinds: Vec<BackendKind> = all_backends().iter().map(|b| b.kind()).collect();

        assert!(kinds.contains(&BackendKind::Homebrew));
        assert!(kinds.contains(&BackendKind::Apt));
        assert!(kinds.contains(&BackendKind::Flatpak));

        #[cfg(windows)]
        {
            assert!(kinds.contains(&BackendKind::Winget));
            assert!(kinds.contains(&BackendKind::MsStore));
            assert!(kinds.contains(&BackendKind::WindowsDrivers));
            assert!(kinds.contains(&BackendKind::NvidiaGpu));
            assert!(kinds.contains(&BackendKind::AmdGpu));
            assert!(kinds.contains(&BackendKind::IntelGpu));
            assert!(kinds.contains(&BackendKind::QualcommGpu));
            assert!(kinds.contains(&BackendKind::DellCommandUpdate));
            assert!(kinds.contains(&BackendKind::HpImageAssistant));
            assert!(kinds.contains(&BackendKind::LenovoSystemUpdate));
            assert!(kinds.contains(&BackendKind::MsiCenter));
            assert!(kinds.contains(&BackendKind::AsusArmoury));
            assert!(kinds.contains(&BackendKind::GigabyteControlCenter));
            assert!(kinds.contains(&BackendKind::AcerCareCenter));
            assert!(kinds.contains(&BackendKind::RazerSynapse));
            assert!(kinds.contains(&BackendKind::Chocolatey));
            assert!(kinds.contains(&BackendKind::Scoop));
            assert!(kinds.contains(&BackendKind::VirtualizationGuest));
            assert!(kinds.contains(&BackendKind::Pip));
            assert!(kinds.contains(&BackendKind::Cargo));
            assert!(kinds.contains(&BackendKind::Npm));
            assert!(kinds.contains(&BackendKind::Go));
            assert!(kinds.contains(&BackendKind::DotnetTool));
            assert!(kinds.contains(&BackendKind::VscodeExtension));
            assert!(kinds.contains(&BackendKind::PowerShellModule));
            assert!(kinds.contains(&BackendKind::NvidiaGeForceExperience));
            assert!(kinds.contains(&BackendKind::IntelDsa));
            assert!(kinds.contains(&BackendKind::JetbrainsPlugin));
            assert!(kinds.contains(&BackendKind::WindowsOptionalFeature));
            assert!(kinds.contains(&BackendKind::DellFirmware));
            assert!(kinds.contains(&BackendKind::HpFirmware));
            assert!(kinds.contains(&BackendKind::LenovoFirmware));
        }

        #[cfg(target_os = "linux")]
        {
            assert!(kinds.contains(&BackendKind::Dnf));
            assert!(kinds.contains(&BackendKind::Pacman));
            assert!(kinds.contains(&BackendKind::Fwupd));
            assert!(kinds.contains(&BackendKind::Snap));
            assert!(kinds.contains(&BackendKind::Zypper));
            assert!(kinds.contains(&BackendKind::AppImage));
        }

        #[cfg(target_os = "macos")]
        {
            assert!(kinds.contains(&BackendKind::MacFirmware));
            assert!(kinds.contains(&BackendKind::MacSoftwareUpdate));
        }
    }

    #[test]
    fn every_backend_reports_a_non_empty_display_name() {
        for b in all_backends() {
            assert!(!b.display_name().is_empty(), "{} has no name", b.kind());
        }
    }

    #[tokio::test]
    async fn disabled_backends_are_excluded_from_detection() {
        let cfg = Config {
            disabled_backends: all_backends()
                .iter()
                .map(|b| b.kind().id().to_string())
                .collect(),
            ..Config::default()
        };

        assert!(detect_backends(&cfg).await.is_empty());
    }
}
