//! Shared infrastructure for OEM fleet tool backends.
//!
//! OEMs like Dell, HP, Lenovo, and MSI ship their own update utilities.
//! This module provides shared utilities for detecting which OEM made the
//! current machine and where their update tool is installed.

pub mod acer_care_center;
pub mod asus_armoury;
pub mod dell_command_update;
pub mod gigabyte_control_center;
pub mod hp_image_assistant;
pub mod lenovo_system_update;
pub mod msi_center;
pub mod razer_synapse;

use std::path::PathBuf;

/// Which OEM manufacturer this machine belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OemManufacturer {
    Dell,
    Hp,
    Lenovo,
    Msi,
    Asus,
    Asrock,
    Gigabyte,
    Acer,
    Toshiba,
    Samsung,
    Razer,
    Unknown,
}

impl OemManufacturer {
    /// Detect the OEM by querying WMI for `Win32_ComputerSystem.Manufacturer`.
    ///
    /// On non-Windows platforms this returns `Unknown`.
    /// Detect the machine's manufacturer, computing it at most once per run.
    ///
    /// All eight OEM backends probe this during availability detection. Each
    /// call spawned a fresh `powershell.exe` running the same CIM query — and
    /// on this hardware a cold `powershell.exe` alone is ~1 s, so eight of them
    /// firing together at startup was pure waste and process-contention. The
    /// manufacturer cannot change while the app runs, so it is memoized.
    pub async fn detect() -> OemManufacturer {
        static CACHE: tokio::sync::OnceCell<OemManufacturer> = tokio::sync::OnceCell::const_new();
        *CACHE.get_or_init(Self::detect_uncached).await
    }

    async fn detect_uncached() -> OemManufacturer {
        if !cfg!(windows) {
            return OemManufacturer::Unknown;
        }

        let out = match odysync_core::proc::powershell(
            "(Get-CimInstance -ClassName Win32_ComputerSystem).Manufacturer",
            std::time::Duration::from_secs(10),
        )
        .await
        {
            Ok(o) if o.success() => o.stdout.trim().to_string(),
            _ => return OemManufacturer::Unknown,
        };

        Self::from_manufacturer_string(&out)
    }

    /// Map a manufacturer string (e.g. "Dell Inc.") to an OEM.
    pub fn from_manufacturer_string(s: &str) -> OemManufacturer {
        let lower = s.to_lowercase();
        if lower.contains("dell") {
            OemManufacturer::Dell
        } else if lower.contains("hp") || lower.contains("hewlett") {
            OemManufacturer::Hp
        } else if lower.contains("lenovo") {
            OemManufacturer::Lenovo
        } else if lower.contains("micro-star") || lower.contains("msi") {
            OemManufacturer::Msi
        } else if lower.contains("asus") || lower.contains("asustek") {
            OemManufacturer::Asus
        } else if lower.contains("asrock") {
            OemManufacturer::Asrock
        } else if lower.contains("gigabyte") {
            OemManufacturer::Gigabyte
        } else if lower.contains("acer") {
            OemManufacturer::Acer
        } else if lower.contains("toshiba") {
            OemManufacturer::Toshiba
        } else if lower.contains("samsung") {
            OemManufacturer::Samsung
        } else if lower.contains("razer") {
            OemManufacturer::Razer
        } else {
            OemManufacturer::Unknown
        }
    }
}

/// Check if a file exists at the given path.
pub fn tool_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Common install paths for OEM update tools.
pub fn oem_tool_path(oem: OemManufacturer) -> Option<PathBuf> {
    let paths: &[&str] = match oem {
        OemManufacturer::Dell => &[
            r"C:\Program Files\Dell\CommandUpdate\dcu-cli.exe",
            r"C:\Program Files (x86)\Dell\CommandUpdate\dcu-cli.exe",
        ],
        OemManufacturer::Hp => &[
            r"C:\Program Files\HP\HPIA\HPImageAssistant.exe",
            r"C:\Program Files (x86)\HP\HPIA\HPImageAssistant.exe",
        ],
        OemManufacturer::Lenovo => &[
            r"C:\Program Files (x86)\Lenovo\System Update\tvsu.exe",
            r"C:\Program Files (x86)\Lenovo\Commercial Vantage\SUHelper.exe",
        ],
        OemManufacturer::Msi => &[r"C:\Program Files\MSI\MSI Center\MSI Center.exe"],
        OemManufacturer::Asus => &[
            r"C:\Program Files\ASUS\ArmouryDevice\ArmouryCrateService.exe",
            r"C:\Program Files (x86)\ASUS\ArmouryDevice\ArmouryCrateService.exe",
        ],
        OemManufacturer::Gigabyte => &[
            r"C:\Program Files\Gigabyte\ControlCenter\GigabyteControlCenter.exe",
            r"C:\Program Files (x86)\Gigabyte\ControlCenter\GigabyteControlCenter.exe",
        ],
        OemManufacturer::Acer => &[
            r"C:\Program Files\Acer\CareCenter\CareCenter.exe",
            r"C:\Program Files (x86)\Acer\CareCenter\CareCenter.exe",
        ],
        OemManufacturer::Razer => &[
            r"C:\Program Files\Razer\Synapse\Synapse.exe",
            r"C:\Program Files (x86)\Razer\Synapse\Synapse.exe",
        ],
        _ => return None,
    };

    paths.iter().find(|p| tool_exists(p)).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dell_from_manufacturer_string() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Dell Inc."),
            OemManufacturer::Dell
        );
    }

    #[test]
    fn detects_hp_from_hewlett_packard() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Hewlett-Packard"),
            OemManufacturer::Hp
        );
    }

    #[test]
    fn detects_lenovo() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("LENOVO"),
            OemManufacturer::Lenovo
        );
    }

    #[test]
    fn detects_msi_from_micro_star() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Micro-Star International Co., Ltd."),
            OemManufacturer::Msi
        );
    }

    #[test]
    fn detects_asus_from_asustek() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("ASUSTeK COMPUTER INC."),
            OemManufacturer::Asus
        );
    }

    #[test]
    fn detects_gigabyte() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Gigabyte Technology Co., Ltd."),
            OemManufacturer::Gigabyte
        );
    }

    #[test]
    fn detects_acer() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Acer Incorporated"),
            OemManufacturer::Acer
        );
    }

    #[test]
    fn detects_razer() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Razer Blade"),
            OemManufacturer::Razer
        );
    }

    #[test]
    fn unknown_for_unrecognized() {
        assert_eq!(
            OemManufacturer::from_manufacturer_string("Custom Builder"),
            OemManufacturer::Unknown
        );
    }
}
