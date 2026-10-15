//! What this machine is: processor, memory, graphics, disks, model, OS.
//!
//! Read-only, and read from the platform's own sources — CIM on Windows,
//! `/proc`, `/sys` and `os-release` on Linux, `sysctl` and `system_profiler`
//! on macOS. Every parser is pure and tested against captured real output,
//! because "Unknown" in a system report is a bug the user has no way to see
//! past.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemReport {
    /// e.g. "Windows 11 Pro 25H2 (build 26200)", "Ubuntu 24.04.1 LTS".
    pub os: String,
    /// Manufacturer and model, e.g. "Micro-Star International MS-7E12".
    pub machine: Option<String>,
    pub cpu: String,
    pub cpu_threads: u32,
    pub memory_bytes: u64,
    pub gpus: Vec<Gpu>,
    pub disks: Vec<Disk>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// Drive letter or mount point.
    pub mount: String,
    pub label: Option<String>,
    pub filesystem: String,
    pub size_bytes: u64,
    pub free_bytes: u64,
}

// ── Pure parsers ────────────────────────────────────────────────────────────

/// `PRETTY_NAME` from `/etc/os-release`.
pub fn parse_os_release(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.strip_prefix("PRETTY_NAME=")?;
        let v = v.trim().trim_matches('"').trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// `MemTotal` from `/proc/meminfo`, in bytes.
pub fn parse_meminfo(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix("MemTotal:")?;
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kb * 1024)
    })
}

/// The first `model name` from `/proc/cpuinfo` (x86), or `Model` (ARM boards).
pub fn parse_cpuinfo(text: &str) -> Option<String> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == name)
                .then(|| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
    };
    field("model name").or_else(|| field("Model"))
}

/// Graphics devices from `lspci -mm`, which quotes every field.
///
/// Lines look like:
/// `01:00.0 "VGA compatible controller" "NVIDIA Corporation" "AD103 [GeForce RTX 4080]" -ra1 "Micro-Star" "Device 5112"`
pub fn parse_lspci_mm(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('"').collect();
            // [slot, class, " ", vendor, " ", device, ...]
            if fields.len() < 6 {
                return None;
            }
            let class = fields[1];
            let is_display = class.starts_with("VGA")
                || class.starts_with("3D controller")
                || class.starts_with("Display controller");
            is_display.then(|| Gpu {
                name: fields[5].trim().to_string(),
                vendor: fields[3].trim().to_string(),
                driver_version: None,
            })
        })
        .collect()
}

/// Local, real file systems from `df -P -k -T` (Linux) output.
///
/// Columns: source, type, 1024-blocks, used, available, capacity, mount.
/// Pseudo file systems are dropped — a tmpfs is not a disk.
pub fn parse_df_linux(text: &str) -> Vec<Disk> {
    const PSEUDO: &[&str] = &[
        "tmpfs", "devtmpfs", "squashfs", "overlay", "proc", "sysfs", "efivarfs", "ramfs",
    ];
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 7 || PSEUDO.contains(&cols[1]) {
                return None;
            }
            let size: u64 = cols[2].parse().ok()?;
            let free: u64 = cols[4].parse().ok()?;
            Some(Disk {
                mount: cols[6..].join(" "),
                label: Some(cols[0].to_string()),
                filesystem: cols[1].to_string(),
                size_bytes: size * 1024,
                free_bytes: free * 1024,
            })
        })
        .filter(|d| d.size_bytes > 0)
        .collect()
}

/// Volumes from `df -P -k -l` on macOS (no type column there).
///
/// Only `/` and `/Volumes/*` are disks a user thinks of; the APFS system
/// sub-volumes (`/System/Volumes/...`) are implementation detail.
pub fn parse_df_macos(text: &str) -> Vec<Disk> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 6 {
                return None;
            }
            let mount = cols[5..].join(" ");
            if mount != "/" && !mount.starts_with("/Volumes/") {
                return None;
            }
            Some(Disk {
                mount,
                label: None,
                filesystem: "apfs".into(),
                size_bytes: cols[1].parse::<u64>().ok()? * 1024,
                free_bytes: cols[3].parse::<u64>().ok()? * 1024,
            })
        })
        .collect()
}

/// GPUs from `system_profiler SPDisplaysDataType -json`.
pub fn parse_system_profiler_displays(json: &str) -> Vec<Gpu> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .get("SPDisplaysDataType")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let name = g
                .get("sppci_model")
                .or_else(|| g.get("_name"))
                .and_then(|v| v.as_str())?
                .to_string();
            let vendor = g
                .get("spdisplays_vendor")
                .and_then(|v| v.as_str())
                .map(|v| v.trim_start_matches("sppci_vendor_").to_string())
                .unwrap_or_else(|| "Apple".into());
            Some(Gpu {
                name,
                vendor,
                driver_version: None,
            })
        })
        .collect()
}

// ── Platform readers ────────────────────────────────────────────────────────

fn threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

/// Collect the report for this machine. Never fails: anything that cannot be
/// read is left empty rather than failing the whole report.
pub async fn report() -> SystemReport {
    imp::report().await
}

#[cfg(windows)]
mod imp {
    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Raw {
        cpu: Option<String>,
        /// A string: TotalPhysicalMemory exceeds what PowerShell's JSON
        /// writer represents exactly as a number.
        memory_bytes: Option<String>,
        os_caption: Option<String>,
        os_display: Option<String>,
        os_build: Option<String>,
        manufacturer: Option<String>,
        model: Option<String>,
        #[serde(default)]
        gpu: Vec<RawGpu>,
        #[serde(default)]
        disks: Vec<RawDisk>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct RawGpu {
        name: Option<String>,
        driver_version: Option<String>,
        adapter_compatibility: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct RawDisk {
        // CIM spells it `DeviceID`, which PascalCase would turn into `DeviceId`.
        #[serde(rename = "DeviceID")]
        device_id: Option<String>,
        volume_name: Option<String>,
        file_system: Option<String>,
        size: Option<serde_json::Value>,
        free_space: Option<serde_json::Value>,
    }

    fn bytes(v: Option<serde_json::Value>) -> u64 {
        match v {
            Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0) as u64,
            Some(serde_json::Value::String(s)) => s.trim().parse().unwrap_or(0),
            _ => 0,
        }
    }

    pub async fn report() -> SystemReport {
        // One PowerShell process for everything: separate spawns took several
        // seconds and made the page the slowest in the app.
        let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
[PSCustomObject]@{
    Cpu          = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
    MemoryBytes  = [string]$cs.TotalPhysicalMemory
    OsCaption    = $os.Caption
    OsDisplay    = $cv.DisplayVersion
    OsBuild      = $os.BuildNumber
    Manufacturer = $cs.Manufacturer
    Model        = $cs.Model
    Gpu          = @(Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, AdapterCompatibility)
    Disks        = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | Select-Object DeviceID, VolumeName, FileSystem, Size, FreeSpace)
} | ConvertTo-Json -Depth 4 -Compress
"#;
        let raw: Option<Raw> =
            odysync_core::proc::powershell(script, std::time::Duration::from_secs(30))
                .await
                .ok()
                .and_then(|o| serde_json::from_str(o.stdout.trim()).ok());
        let Some(raw) = raw else {
            return SystemReport {
                os: odysync_core::platform::os_label().into(),
                cpu: "Unknown".into(),
                cpu_threads: threads(),
                ..SystemReport::default()
            };
        };

        let os = match (raw.os_caption, raw.os_display, raw.os_build) {
            (Some(c), Some(d), Some(b)) => format!("{} {d} (build {b})", c.trim()),
            (Some(c), None, Some(b)) => format!("{} (build {b})", c.trim()),
            (Some(c), _, _) => c.trim().to_string(),
            _ => "Windows".into(),
        };
        let machine = match (raw.manufacturer, raw.model) {
            (Some(m), Some(n)) if !m.trim().is_empty() => {
                Some(format!("{} {}", m.trim(), n.trim()))
            }
            _ => None,
        };

        SystemReport {
            os,
            machine,
            cpu: raw
                .cpu
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "Unknown".into()),
            cpu_threads: threads(),
            memory_bytes: raw
                .memory_bytes
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
            gpus: raw
                .gpu
                .into_iter()
                .filter_map(|g| {
                    Some(Gpu {
                        name: g.name.filter(|n| !n.trim().is_empty())?,
                        vendor: g.adapter_compatibility.unwrap_or_default(),
                        driver_version: g.driver_version,
                    })
                })
                .collect(),
            disks: raw
                .disks
                .into_iter()
                .map(|d| Disk {
                    mount: d.device_id.unwrap_or_default(),
                    label: d.volume_name.filter(|v| !v.trim().is_empty()),
                    filesystem: d.file_system.unwrap_or_default(),
                    size_bytes: bytes(d.size),
                    free_bytes: bytes(d.free_space),
                })
                .collect(),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use std::time::Duration;

    fn read(path: &str) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    pub async fn report() -> SystemReport {
        let vendor = read("/sys/class/dmi/id/sys_vendor");
        let product = read("/sys/class/dmi/id/product_name");
        let machine = (!vendor.trim().is_empty()).then(|| {
            format!("{} {}", vendor.trim(), product.trim())
                .trim()
                .to_string()
        });

        let mut gpus =
            match odysync_core::proc::run("lspci", &["-mm"], Duration::from_secs(10)).await {
                Ok(out) if out.success() => parse_lspci_mm(&out.stdout),
                _ => Vec::new(),
            };
        // The proprietary NVIDIA driver publishes its version here.
        let nvidia = read("/sys/module/nvidia/version");
        for g in gpus.iter_mut() {
            if g.vendor.to_ascii_lowercase().contains("nvidia") && !nvidia.trim().is_empty() {
                g.driver_version = Some(nvidia.trim().to_string());
            }
        }

        let disks =
            match odysync_core::proc::run("df", &["-P", "-k", "-T"], Duration::from_secs(10)).await
            {
                Ok(out) => parse_df_linux(&out.stdout),
                Err(_) => Vec::new(),
            };

        SystemReport {
            os: parse_os_release(&read("/etc/os-release")).unwrap_or_else(|| "Linux".into()),
            machine,
            cpu: parse_cpuinfo(&read("/proc/cpuinfo")).unwrap_or_else(|| "Unknown".into()),
            cpu_threads: threads(),
            memory_bytes: parse_meminfo(&read("/proc/meminfo")).unwrap_or(0),
            gpus,
            disks,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::time::Duration;

    async fn out(program: &str, args: &[&str]) -> String {
        odysync_core::proc::run(program, args, Duration::from_secs(20))
            .await
            .map(|o| o.stdout.trim().to_string())
            .unwrap_or_default()
    }

    pub async fn report() -> SystemReport {
        let name = out("sw_vers", &["-productName"]).await;
        let version = out("sw_vers", &["-productVersion"]).await;
        let model = out("sysctl", &["-n", "hw.model"]).await;
        SystemReport {
            os: format!(
                "{} {}",
                if name.is_empty() { "macOS" } else { &name },
                version
            )
            .trim()
            .to_string(),
            machine: (!model.is_empty()).then(|| format!("Apple {model}")),
            cpu: Some(out("sysctl", &["-n", "machdep.cpu.brand_string"]).await)
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "Unknown".into()),
            cpu_threads: threads(),
            memory_bytes: out("sysctl", &["-n", "hw.memsize"])
                .await
                .parse()
                .unwrap_or(0),
            gpus: parse_system_profiler_displays(
                &out("system_profiler", &["SPDisplaysDataType", "-json"]).await,
            ),
            disks: parse_df_macos(&out("df", &["-P", "-k", "-l"]).await),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_pretty_name() {
        let text = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(
            parse_os_release(text).as_deref(),
            Some("Ubuntu 24.04.1 LTS")
        );
        assert_eq!(parse_os_release("ID=x\n"), None);
    }

    #[test]
    fn meminfo_total_in_bytes() {
        let text = "MemTotal:       65771340 kB\nMemFree:         1234 kB\n";
        assert_eq!(parse_meminfo(text), Some(65_771_340 * 1024));
    }

    #[test]
    fn cpu_model_from_x86_and_arm_cpuinfo() {
        let x86 = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 9 9950X3D 16-Core Processor\n";
        assert_eq!(
            parse_cpuinfo(x86).as_deref(),
            Some("AMD Ryzen 9 9950X3D 16-Core Processor")
        );
        let pi =
            "processor\t: 0\nBogoMIPS\t: 108.00\n\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n";
        assert_eq!(
            parse_cpuinfo(pi).as_deref(),
            Some("Raspberry Pi 5 Model B Rev 1.0")
        );
    }

    #[test]
    fn lspci_display_devices_only() {
        let text = concat!(
            "00:00.0 \"Host bridge\" \"Advanced Micro Devices, Inc. [AMD]\" \"Root Complex\" \"\" \"\"\n",
            "01:00.0 \"VGA compatible controller\" \"NVIDIA Corporation\" \"AD103 [GeForce RTX 4080]\" -ra1 \"Micro-Star International Co., Ltd. [MSI]\" \"Device 5112\"\n",
            "02:00.0 \"3D controller\" \"NVIDIA Corporation\" \"GA107M [GeForce RTX 3050 Mobile]\" -ra1 \"\" \"\"\n",
        );
        let gpus = parse_lspci_mm(text);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "AD103 [GeForce RTX 4080]");
        assert_eq!(gpus[0].vendor, "NVIDIA Corporation");
    }

    #[test]
    fn df_on_linux_skips_pseudo_file_systems() {
        let text = "Filesystem     Type  1024-blocks      Used Available Capacity Mounted on\n\
                    tmpfs          tmpfs     6577136      2056   6575080       1% /run\n\
                    /dev/nvme0n1p2 ext4    982862268 412345678 520516590      45% /\n\
                    /dev/sda1      ntfs3  3907016700 100 3907016600       1% /mnt/Data Drive\n";
        let disks = parse_df_linux(text);
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].mount, "/");
        assert_eq!(disks[0].filesystem, "ext4");
        assert_eq!(disks[0].free_bytes, 520_516_590 * 1024);
        assert_eq!(disks[1].mount, "/mnt/Data Drive");
    }

    #[test]
    fn df_on_macos_keeps_root_and_external_volumes() {
        let text = "Filesystem     1024-blocks      Used Available Capacity  Mounted on\n\
                    /dev/disk3s1s1   971350180  10476592 512345678     3%    /\n\
                    /dev/disk3s6     971350180   2097172 512345678     1%    /System/Volumes/VM\n\
                    /dev/disk5s1    1953514552 100000000 1853514552    6%    /Volumes/Backup Disk\n";
        let disks = parse_df_macos(text);
        assert_eq!(
            disks.iter().map(|d| d.mount.as_str()).collect::<Vec<_>>(),
            vec!["/", "/Volumes/Backup Disk"]
        );
    }

    #[test]
    fn system_profiler_gpus() {
        let json = r#"{"SPDisplaysDataType":[{"_name":"Apple M3 Pro","sppci_model":"Apple M3 Pro","spdisplays_vendor":"sppci_vendor_Apple","sppci_cores":"18"}]}"#;
        let gpus = parse_system_profiler_displays(json);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Apple M3 Pro");
        assert_eq!(gpus[0].vendor, "Apple");
        assert!(parse_system_profiler_displays("not json").is_empty());
    }
}
