//! Shared infrastructure for GPU driver backends.
//!
//! All three GPU vendors (NVIDIA, AMD, Intel) need to enumerate display
//! adapters and parse `pnputil` output.  This module centralises that logic so
//! each vendor backend only has to implement scan/apply for its own update
//! mechanism.

pub mod nvidia_gpu;
pub mod amd_gpu;
pub mod intel_gpu;
pub mod qualcomm_gpu;

use std::time::Duration;

use odysync_core::proc;

/// Which GPU vendor a display adapter belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Qualcomm,
    Unknown,
}

impl GpuVendor {
    /// PCI vendor ID for this vendor.
    pub fn pci_id(self) -> Option<u32> {
        match self {
            GpuVendor::Nvidia => Some(0x10DE),
            GpuVendor::Amd => Some(0x1002),
            GpuVendor::Intel => Some(0x8086),
            GpuVendor::Qualcomm => Some(0x5143),
            GpuVendor::Unknown => None,
        }
    }

    /// Map a PCI vendor ID to a known vendor, or `Unknown`.
    pub fn from_pci_id(vid: u32) -> Self {
        match vid {
            0x10DE => GpuVendor::Nvidia,
            0x1002 => GpuVendor::Amd,
            0x8086 => GpuVendor::Intel,
            0x5143 => GpuVendor::Qualcomm,
            _ => GpuVendor::Unknown,
        }
    }
}

/// A display adapter found on the system.
#[derive(Debug, Clone)]
pub struct GpuAdapter {
    pub vendor: GpuVendor,
    /// PCI device ID (hex string, e.g. "2489").
    pub device_id: String,
    /// Installed driver version string.
    pub driver_version: String,
    /// Human-readable device name.
    pub name: String,
}

/// Enumerate display adapters by shelling out to `pnputil /enum-devices /class Display`.
///
/// On non-Windows platforms this returns an empty vec.
pub async fn enumerate_display_adapters() -> Vec<GpuAdapter> {
    if !cfg!(windows) {
        return Vec::new();
    }

    let out = match proc::run(
        "pnputil",
        &["/enum-devices", "/class", "Display"],
        Duration::from_secs(15),
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "pnputil /enum-devices failed");
            return Vec::new();
        }
    };

    parse_pnputil_display(&out.stdout)
}

/// Parse `pnputil /enum-devices /class Display` output.
///
/// The output looks like:
/// ```text
/// Instance ID:    PCI\VEN_10DE&DEV_2489&SUBSYS_...
/// Device Description:  NVIDIA GeForce RTX 3060
/// Driver Name:    oemNN.inf
/// ...
/// ```
///
/// We extract the vendor/device ID from the Instance ID line and the device
/// name from the Device Description line.  The driver version is not directly
/// available from pnputil; each vendor backend reads it from the registry.
pub fn parse_pnputil_display(output: &str) -> Vec<GpuAdapter> {
    let mut adapters = Vec::new();
    let mut current_vendor = GpuVendor::Unknown;
    let mut current_device_id = String::new();
    let mut current_name = String::new();

    for line in output.lines() {
        let line = line.trim();

        if line.starts_with("Instance ID:") {
            // Push the previous adapter if we have one.
            if current_vendor != GpuVendor::Unknown && !current_name.is_empty() {
                adapters.push(GpuAdapter {
                    vendor: current_vendor,
                    device_id: current_device_id.clone(),
                    driver_version: String::new(),
                    name: current_name.clone(),
                });
            }
            current_vendor = GpuVendor::Unknown;
            current_device_id.clear();
            current_name.clear();

            // Parse VEN_XXXX&DEV_XXXX from the instance ID.
            if let Some(vid) = parse_hex_field(line, "VEN_") {
                current_vendor = GpuVendor::from_pci_id(vid);
            }
            if let Some(did) = parse_hex_field(line, "DEV_") {
                current_device_id = format!("{:04X}", did);
            }
        } else if line.starts_with("Device Description:") {
            current_name = line
                .trim_start_matches("Device Description:")
                .trim()
                .to_string();
        }
    }

    // Push the last adapter.
    if current_vendor != GpuVendor::Unknown && !current_name.is_empty() {
        adapters.push(GpuAdapter {
            vendor: current_vendor,
            device_id: current_device_id,
            driver_version: String::new(),
            name: current_name,
        });
    }

    adapters
}

/// Extract a 4-digit hex value from a field like `VEN_10DE` or `DEV_2489`.
fn parse_hex_field(line: &str, prefix: &str) -> Option<u32> {
    let pos = line.find(prefix)?;
    let rest = &line[pos + prefix.len()..];
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u32::from_str_radix(&hex, 16).ok()
}

/// Read an installed driver version from the Windows registry.
///
/// GPU drivers store their version under vendor-specific registry keys:
/// - NVIDIA: `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm\Global\NVTweak`
/// - AMD: `HKLM\SYSTEM\CurrentControlSet\Services\amdkmdag\Global`
/// - Intel: `HKLM\SYSTEM\CurrentControlSet\Services\igfx\Global`
/// - Qualcomm: `HKLM\SYSTEM\CurrentControlSet\Services\qcomdisp\Global`
///
/// The `registry_suffix` parameter selects the vendor-specific subkey path
/// (e.g. `Global\NVTweak` for NVIDIA, `Global` for AMD/Intel).
#[cfg(windows)]
pub async fn read_driver_version_from_registry(
    service_name: &str,
    registry_suffix: &str,
    value_name: &str,
) -> Option<String> {
    let script = format!(
        r#"
        $val = (Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Services\{}\{}' -Name '{}' -ErrorAction SilentlyContinue).{}
        if ($val) {{ Write-Output $val }}
        "#,
        service_name, registry_suffix, value_name, value_name
    );

    let out = proc::powershell(&script, Duration::from_secs(10)).await.ok()?;
    let version = out.stdout.trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pnputil_display_output_with_nvidia_adapter() {
        let output = "\
Instance ID:    PCI\\VEN_10DE&DEV_2489&SUBSYS_39701462&REV_A1\\4&30B728B&0&0008
Device Description:  NVIDIA GeForce RTX 3060
Driver Name:    oem42.inf
";
        let adapters = parse_pnputil_display(output);
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].vendor, GpuVendor::Nvidia);
        assert_eq!(adapters[0].device_id, "2489");
        assert!(adapters[0].name.contains("RTX 3060"));
    }

    #[test]
    fn parses_multiple_adapters() {
        let output = "\
Instance ID:    PCI\\VEN_10DE&DEV_2489&SUBSYS_...\\4&...
Device Description:  NVIDIA GeForce RTX 3060

Instance ID:    PCI\\VEN_1002&DEV_73BF&SUBSYS_...\\4&...
Device Description:  AMD Radeon RX 7900 XT
";
        let adapters = parse_pnputil_display(output);
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].vendor, GpuVendor::Nvidia);
        assert_eq!(adapters[1].vendor, GpuVendor::Amd);
        assert_eq!(adapters[1].device_id, "73BF");
    }

    #[test]
    fn parses_intel_adapter() {
        let output = "\
Instance ID:    PCI\\VEN_8086&DEV_9A49&SUBSYS_...\\4&...
Device Description:  Intel(R) Iris(R) Xe Graphics
";
        let adapters = parse_pnputil_display(output);
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].vendor, GpuVendor::Intel);
        assert_eq!(adapters[0].device_id, "9A49");
    }

    #[test]
    fn ignores_non_display_entries() {
        let output = "Instance ID:    USB\\VID_046D&PID_C52B\\5&...\nDevice Description:  Logitech USB Receiver\n";
        let adapters = parse_pnputil_display(output);
        // USB device — vendor 0x046D is Logitech, not a known GPU vendor.
        assert_eq!(adapters.len(), 0);
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_pnputil_display("").is_empty());
    }

    #[test]
    fn vendor_from_pci_id_round_trips() {
        assert_eq!(GpuVendor::from_pci_id(0x10DE), GpuVendor::Nvidia);
        assert_eq!(GpuVendor::from_pci_id(0x1002), GpuVendor::Amd);
        assert_eq!(GpuVendor::from_pci_id(0x8086), GpuVendor::Intel);
        assert_eq!(GpuVendor::from_pci_id(0x5143), GpuVendor::Qualcomm);
        assert_eq!(GpuVendor::from_pci_id(0x1234), GpuVendor::Unknown);
    }
}
