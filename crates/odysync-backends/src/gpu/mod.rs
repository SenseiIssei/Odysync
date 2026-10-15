//! Shared infrastructure for GPU driver backends.
//!
//! All three GPU vendors (NVIDIA, AMD, Intel) need to enumerate display
//! adapters and parse `pnputil` output.  This module centralises that logic so
//! each vendor backend only has to implement scan/apply for its own update
//! mechanism.

pub mod amd_gpu;
pub mod intel_gpu;
pub mod nvidia_gpu;
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

/// The machine's display adapters, enumerated at most once per run.
///
/// All four GPU backends call this during availability detection, and again
/// during scan; the list does not change while the app runs, so it is
/// memoized to a single query.
pub async fn enumerate_display_adapters() -> Vec<GpuAdapter> {
    static CACHE: tokio::sync::OnceCell<Vec<GpuAdapter>> = tokio::sync::OnceCell::const_new();
    CACHE
        .get_or_init(enumerate_display_adapters_uncached)
        .await
        .clone()
}

/// Ask WMI for every video controller, as JSON.
///
/// This used to parse `pnputil /enum-devices /class Display`, matching on the
/// labels `Instance ID:` and `Device Description:`. pnputil translates those
/// labels — a German Windows prints `Instanz-ID:` and `Gerätebeschreibung:` —
/// so on every non-English machine no adapter was ever found and all four GPU
/// backends silently reported nothing. `Win32_VideoController` returns
/// property *names* that are never translated, and also carries the driver
/// version the backends need.
#[cfg(windows)]
async fn enumerate_display_adapters_uncached() -> Vec<GpuAdapter> {
    let script = "Get-CimInstance Win32_VideoController | \
                  Select-Object Name, PNPDeviceID, DriverVersion | ConvertTo-Json -Compress";
    match proc::powershell(script, Duration::from_secs(30)).await {
        Ok(out) => parse_video_controllers(&out.stdout),
        Err(e) => {
            tracing::warn!(error = %e, "could not enumerate display adapters");
            Vec::new()
        }
    }
}

#[cfg(not(windows))]
async fn enumerate_display_adapters_uncached() -> Vec<GpuAdapter> {
    Vec::new()
}

/// Parse `Win32_VideoController` rows emitted by `ConvertTo-Json`.
///
/// PowerShell emits a bare object rather than a one-element array when there
/// is a single adapter, so both shapes are accepted. Adapters that are not on
/// the PCI bus (a remote-display or virtual adapter) are skipped, since no
/// vendor feed knows them.
pub fn parse_video_controllers(json: &str) -> Vec<GpuAdapter> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "Name")]
        name: Option<String>,
        #[serde(rename = "PNPDeviceID")]
        pnp_device_id: Option<String>,
        #[serde(rename = "DriverVersion")]
        driver_version: Option<String>,
    }

    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }
    let rows: Vec<Row> = if json.starts_with('[') {
        serde_json::from_str(json).unwrap_or_default()
    } else {
        serde_json::from_str::<Row>(json)
            .map(|r| vec![r])
            .unwrap_or_default()
    };

    rows.into_iter()
        .filter_map(|row| {
            let pnp = row.pnp_device_id?;
            if !pnp.to_ascii_uppercase().starts_with("PCI\\") {
                return None;
            }
            let vendor = GpuVendor::from_pci_id(parse_hex_field(&pnp, "VEN_")?);
            if vendor == GpuVendor::Unknown {
                return None;
            }
            let device_id = format!("{:04X}", parse_hex_field(&pnp, "DEV_")?);
            Some(GpuAdapter {
                vendor,
                device_id,
                driver_version: row.driver_version.unwrap_or_default().trim().to_string(),
                name: row.name.unwrap_or_default().trim().to_string(),
            })
        })
        .collect()
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

    let out = proc::powershell(&script, Duration::from_secs(10))
        .await
        .ok()?;
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

    /// Captured from a German Windows 11 machine with two adapters. The old
    /// pnputil parser returned nothing for this machine.
    const REAL: &str = r#"[{"Name":"NVIDIA GeForce RTX 4080","PNPDeviceID":"PCI\\VEN_10DE&DEV_2704&SUBSYS_51121462&REV_A1\\4&8FC2AB9&0&0009","DriverVersion":"32.0.16.1692"},{"Name":"AMD Radeon(TM) Graphics","PNPDeviceID":"PCI\\VEN_1002&DEV_13C0&SUBSYS_7E261462&REV_CB\\4&16ADCFD2&0&0041","DriverVersion":"32.0.21042.62"}]"#;

    #[test]
    fn video_controllers_are_parsed_whatever_the_os_language() {
        let adapters = parse_video_controllers(REAL);
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].vendor, GpuVendor::Nvidia);
        assert_eq!(adapters[0].device_id, "2704");
        assert_eq!(adapters[0].driver_version, "32.0.16.1692");
        assert_eq!(adapters[0].name, "NVIDIA GeForce RTX 4080");
        assert_eq!(adapters[1].vendor, GpuVendor::Amd);
        assert_eq!(adapters[1].device_id, "13C0");
    }

    #[test]
    fn a_single_adapter_emitted_as_a_bare_object_is_accepted() {
        let one = r#"{"Name":"Intel(R) Iris(R) Xe Graphics","PNPDeviceID":"PCI\\VEN_8086&DEV_9A49&SUBSYS_0","DriverVersion":"31.0.101.5186"}"#;
        let adapters = parse_video_controllers(one);
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].vendor, GpuVendor::Intel);
        assert_eq!(adapters[0].device_id, "9A49");
    }

    #[test]
    fn virtual_and_unknown_adapters_are_skipped() {
        let json = r#"[{"Name":"Microsoft Remote Display Adapter","PNPDeviceID":"SWD\\REMOTEDISPLAYENUM\\RDPIDD","DriverVersion":"10.0"},{"Name":"Mystery","PNPDeviceID":"PCI\\VEN_1234&DEV_0001","DriverVersion":"1.0"}]"#;
        assert!(parse_video_controllers(json).is_empty());
        assert!(parse_video_controllers("").is_empty());
        assert!(parse_video_controllers("not json").is_empty());
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
