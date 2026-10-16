//! Disk and battery health, in plain words.
//!
//! The two parts of a computer that wear out on their own and take data or
//! mobility with them when they go. Both are read from what the hardware
//! itself reports — SMART and NVMe health logs for disks, design versus
//! full-charge capacity for batteries — through each platform's own tools:
//!
//! - Windows: `Get-PhysicalDisk` and its reliability counters; the battery
//!   classes in `root\wmi`.
//! - macOS: `diskutil` (SMART status) and the `AppleSmartBattery` registry
//!   entry.
//! - Linux: `smartctl` from smartmontools when it is installed and allowed to
//!   read the device, `/sys/class/power_supply` for the battery.
//!
//! Nothing here changes anything, and nothing is guessed: a value the platform
//! does not report is `None` and the state is [`HealthState::Unknown`] rather
//! than an optimistic "good".

use serde::Serialize;

/// How worried to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Good,
    /// Still working, but worth acting on soon (back up, plan a replacement).
    Warning,
    /// The hardware reports a failure or is worn out.
    Bad,
    /// The platform did not tell us.
    Unknown,
}

impl HealthState {
    /// The worse of two states. Unknown never hides a known problem.
    fn worst(self, other: HealthState) -> HealthState {
        use HealthState::*;
        let rank = |s: HealthState| match s {
            Good => 0,
            Unknown => 1,
            Warning => 2,
            Bad => 3,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiskHealth {
    pub name: String,
    /// "NVMe SSD", "SSD", "Hard disk" or "Disk".
    pub kind: String,
    pub size_bytes: u64,
    pub state: HealthState,
    /// One sentence a non-technical reader can act on.
    pub summary: String,
    pub temperature_c: Option<u32>,
    /// Share of the rated write endurance used up, 0–100+.
    pub wear_percent: Option<u32>,
    pub power_on_hours: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatteryHealth {
    pub state: HealthState,
    pub summary: String,
    /// Full-charge capacity as a share of the design capacity.
    pub capacity_percent: Option<u32>,
    pub design_capacity: Option<u64>,
    pub full_capacity: Option<u64>,
    /// "mWh" or "mAh" — whichever the platform reports.
    pub capacity_unit: &'static str,
    pub cycle_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthReport {
    pub disks: Vec<DiskHealth>,
    /// `None` on machines without a battery.
    pub battery: Option<BatteryHealth>,
    /// What could not be read, and how to get it.
    pub notes: Vec<String>,
}

impl HealthReport {
    /// The single state the overview shows.
    pub fn overall(&self) -> HealthState {
        if self.disks.is_empty() && self.battery.is_none() {
            return HealthState::Unknown;
        }
        let disks = self
            .disks
            .iter()
            .fold(HealthState::Good, |acc, d| acc.worst(d.state));
        match &self.battery {
            Some(b) => disks.worst(b.state),
            None => disks,
        }
    }
}

// ── Shared judgement ─────────────────────────────────────────────────────────

/// SSDs throttle around 70 °C and are rated to about that; a disk that sits
/// there is worth mentioning, not alarming about.
const DISK_HOT_C: u32 = 70;

/// Judge a disk from what is known about it. `reported_failure` is the
/// device's own verdict (SMART failed, Windows "Unhealthy"); `reported_warning`
/// a softer one (Windows "Warning", reallocated sectors).
fn judge_disk(
    reported_failure: bool,
    reported_warning: bool,
    reported_ok: bool,
    wear_percent: Option<u32>,
    temperature_c: Option<u32>,
) -> (HealthState, String) {
    if reported_failure {
        return (
            HealthState::Bad,
            "The disk reports that it is failing. Back up your files now and replace it.".into(),
        );
    }
    if let Some(w) = wear_percent
        && w >= 100
    {
        return (
            HealthState::Bad,
            "The disk has used up its rated write endurance. Back up and plan to replace it."
                .into(),
        );
    }
    if reported_warning {
        return (
            HealthState::Warning,
            "The disk reports early signs of trouble. Keep a current backup.".into(),
        );
    }
    if let Some(w) = wear_percent
        && w >= 80
    {
        return (
            HealthState::Warning,
            format!("{w}% of the rated write endurance is used. Fine for now; keep backups."),
        );
    }
    if let Some(t) = temperature_c
        && t >= DISK_HOT_C
    {
        return (
            HealthState::Warning,
            format!("Healthy, but running hot at {t} °C. Check the airflow around it."),
        );
    }
    if reported_ok || wear_percent.is_some() {
        return (HealthState::Good, "Healthy.".into());
    }
    (
        HealthState::Unknown,
        "This disk does not report its health.".into(),
    )
}

fn judge_battery(capacity_percent: Option<u32>) -> (HealthState, String) {
    match capacity_percent {
        Some(p) if p >= 80 => (
            HealthState::Good,
            format!("Holds {p}% of its original charge."),
        ),
        Some(p) if p >= 60 => (
            HealthState::Warning,
            format!("Holds {p}% of its original charge — expect noticeably shorter runtimes."),
        ),
        Some(p) => (
            HealthState::Bad,
            format!("Holds only {p}% of its original charge. A replacement would help."),
        ),
        None => (
            HealthState::Unknown,
            "The battery does not report its capacity.".into(),
        ),
    }
}

fn percent(full: u64, design: u64) -> Option<u32> {
    if design == 0 || full == 0 {
        return None;
    }
    // Capped: a new battery can measure slightly above its design value.
    Some(((full as f64 / design as f64) * 100.0).round().min(100.0) as u32)
}

fn disk_kind(ssd: Option<bool>, nvme: bool) -> String {
    match (nvme, ssd) {
        (true, _) => "NVMe SSD".into(),
        (false, Some(true)) => "SSD".into(),
        (false, Some(false)) => "Hard disk".into(),
        (false, None) => "Disk".into(),
    }
}

fn battery_from(
    design: Option<u64>,
    full: Option<u64>,
    cycles: Option<u32>,
    unit: &'static str,
) -> BatteryHealth {
    let capacity_percent = match (full, design) {
        (Some(f), Some(d)) => percent(f, d),
        _ => None,
    };
    let (state, summary) = judge_battery(capacity_percent);
    BatteryHealth {
        state,
        summary,
        capacity_percent,
        design_capacity: design,
        full_capacity: full,
        capacity_unit: unit,
        cycle_count: cycles,
    }
}

// ── Windows ──────────────────────────────────────────────────────────────────

#[cfg(windows)]
const WINDOWS_DISKS_PS: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$out = foreach ($d in Get-PhysicalDisk) {
  $r = $d | Get-StorageReliabilityCounter
  [pscustomobject]@{
    Name         = [string]$d.FriendlyName
    MediaType    = [string]$d.MediaType
    BusType      = [string]$d.BusType
    Size         = [uint64]$d.Size
    HealthStatus = [string]$d.HealthStatus
    Temperature  = $r.Temperature
    Wear         = $r.Wear
    PowerOnHours = $r.PowerOnHours
  }
}
ConvertTo-Json -InputObject @($out) -Compress
"#;

#[cfg(windows)]
const WINDOWS_BATTERY_PS: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$s = @(Get-CimInstance -Namespace root\wmi -ClassName BatteryStaticData)[0]
$f = @(Get-CimInstance -Namespace root\wmi -ClassName BatteryFullChargedCapacity)[0]
$c = @(Get-CimInstance -Namespace root\wmi -ClassName BatteryCycleCount)[0]
$present = @(Get-CimInstance -ClassName Win32_Battery).Count -gt 0
ConvertTo-Json -Compress -InputObject ([pscustomobject]@{
  Present  = $present
  Design   = $s.DesignedCapacity
  Full     = $f.FullChargedCapacity
  Cycles   = $c.CycleCount
})
"#;

/// One row of [`WINDOWS_DISKS_PS`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(not(windows), allow(dead_code))]
struct WindowsDisk {
    name: Option<String>,
    media_type: Option<String>,
    bus_type: Option<String>,
    size: Option<u64>,
    health_status: Option<String>,
    temperature: Option<u32>,
    wear: Option<u32>,
    power_on_hours: Option<u64>,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_windows_disks(json: &str) -> Vec<DiskHealth> {
    let rows: Vec<WindowsDisk> = serde_json::from_str(json.trim()).unwrap_or_default();
    rows.into_iter()
        .map(|r| {
            let status = r.health_status.as_deref().unwrap_or("");
            let nvme = r.bus_type.as_deref() == Some("NVMe");
            let ssd = match r.media_type.as_deref() {
                Some("SSD") | Some("SCM") => Some(true),
                Some("HDD") => Some(false),
                _ => None,
            };
            // Windows reports 0 for "not measured" on many controllers.
            let temperature = r.temperature.filter(|t| *t > 0);
            let (state, summary) = judge_disk(
                status == "Unhealthy",
                status == "Warning",
                status == "Healthy",
                r.wear,
                temperature,
            );
            DiskHealth {
                name: r.name.unwrap_or_else(|| "Disk".into()),
                kind: disk_kind(ssd, nvme),
                size_bytes: r.size.unwrap_or(0),
                state,
                summary,
                temperature_c: temperature,
                wear_percent: r.wear,
                power_on_hours: r.power_on_hours,
            }
        })
        .collect()
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(not(windows), allow(dead_code))]
struct WindowsBattery {
    present: Option<bool>,
    design: Option<u64>,
    full: Option<u64>,
    cycles: Option<u32>,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_windows_battery(json: &str) -> Option<BatteryHealth> {
    let b: WindowsBattery = serde_json::from_str(json.trim()).ok()?;
    if b.present != Some(true) {
        return None;
    }
    // A cycle count of 0 is what firmware reports when it does not count.
    Some(battery_from(
        b.design,
        b.full,
        b.cycles.filter(|c| *c > 0),
        "mWh",
    ))
}

#[cfg(windows)]
pub async fn report() -> HealthReport {
    use odysync_core::proc;
    use std::time::Duration;

    let timeout = Duration::from_secs(60);
    let (disks, battery) = tokio::join!(
        proc::powershell(WINDOWS_DISKS_PS, timeout),
        proc::powershell(WINDOWS_BATTERY_PS, timeout),
    );

    let mut notes = Vec::new();
    let disks = match disks {
        Ok(o) => parse_windows_disks(&o.stdout),
        Err(e) => {
            notes.push(format!("Could not read the disks: {e}"));
            Vec::new()
        }
    };
    if !odysync_core::platform::is_elevated() && disks.iter().any(|d| d.wear_percent.is_none()) {
        notes.push(
            "Wear and temperature need administrator rights on Windows; \
             run Odysync as administrator to see them."
                .into(),
        );
    }
    let battery = battery.ok().and_then(|o| parse_windows_battery(&o.stdout));

    HealthReport {
        disks,
        battery,
        notes,
    }
}

// ── macOS ────────────────────────────────────────────────────────────────────

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_diskutil_whole_disks(plist_xml: &[u8]) -> Vec<String> {
    let Ok(value) = plist::Value::from_reader_xml(plist_xml) else {
        return Vec::new();
    };
    value
        .as_dictionary()
        .and_then(|d| d.get("WholeDisks"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_string().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_diskutil_info(plist_xml: &[u8]) -> Option<DiskHealth> {
    let value = plist::Value::from_reader_xml(plist_xml).ok()?;
    let d = value.as_dictionary()?;
    let text = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string);
    let flag = |k: &str| d.get(k).and_then(|v| v.as_boolean());
    let number = |k: &str| d.get(k).and_then(|v| v.as_unsigned_integer());

    // Disk images and other virtual devices have nothing to report.
    if text("VirtualOrPhysical").as_deref() == Some("Virtual") {
        return None;
    }

    let smart = text("SMARTStatus").unwrap_or_default();
    let nvme = text("BusProtocol").as_deref() == Some("PCI-Express")
        || text("BusProtocol").as_deref() == Some("Apple Fabric");
    let (state, summary) = judge_disk(smart == "Failing", false, smart == "Verified", None, None);
    Some(DiskHealth {
        name: text("MediaName")
            .or_else(|| text("DeviceIdentifier"))
            .unwrap_or_else(|| "Disk".into()),
        kind: disk_kind(flag("SolidState"), nvme),
        size_bytes: number("TotalSize").or_else(|| number("Size")).unwrap_or(0),
        state,
        summary,
        temperature_c: None,
        wear_percent: None,
        power_on_hours: None,
    })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_apple_smart_battery(plist_xml: &[u8]) -> Option<BatteryHealth> {
    let value = plist::Value::from_reader_xml(plist_xml).ok()?;
    // `ioreg -a` prints an array of matching entries; a desktop Mac has none.
    let entry = value.as_array()?.first()?.as_dictionary()?;
    let number = |k: &str| entry.get(k).and_then(|v| v.as_unsigned_integer());

    let design = number("DesignCapacity");
    // Apple Silicon reports MaxCapacity as a percentage and the real figure
    // as AppleRawMaxCapacity; Intel Macs report mAh in MaxCapacity.
    let full = number("AppleRawMaxCapacity").or_else(|| number("MaxCapacity").filter(|v| *v > 100));
    let cycles = number("CycleCount").map(|c| c as u32);
    Some(battery_from(design, full, cycles, "mAh"))
}

#[cfg(target_os = "macos")]
pub async fn report() -> HealthReport {
    use odysync_core::proc;
    use std::time::Duration;

    let timeout = Duration::from_secs(30);
    let mut disks = Vec::new();
    let mut notes = Vec::new();

    match proc::run("diskutil", &["list", "-plist", "physical"], timeout).await {
        Ok(o) => {
            for id in parse_diskutil_whole_disks(o.stdout.as_bytes()) {
                if let Ok(info) = proc::run("diskutil", &["info", "-plist", &id], timeout).await
                    && let Some(d) = parse_diskutil_info(info.stdout.as_bytes())
                {
                    disks.push(d);
                }
            }
        }
        Err(e) => notes.push(format!("Could not read the disks: {e}")),
    }

    let battery = match proc::run("ioreg", &["-rn", "AppleSmartBattery", "-a"], timeout).await {
        Ok(o) if !o.stdout.trim().is_empty() => parse_apple_smart_battery(o.stdout.as_bytes()),
        _ => None,
    };

    HealthReport {
        disks,
        battery,
        notes,
    }
}

// ── Linux ────────────────────────────────────────────────────────────────────

/// Block devices that are real disks: not loop, RAM, zram, device-mapper,
/// software RAID or optical drives.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_physical_block_device(name: &str) -> bool {
    const SKIP: &[&str] = &["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"];
    !SKIP.iter().any(|p| name.starts_with(p))
}

/// What `smartctl --json` says about one device.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq)]
struct SmartReading {
    state: HealthState,
    summary: String,
    temperature_c: Option<u32>,
    wear_percent: Option<u32>,
    power_on_hours: Option<u64>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_smartctl(json: &str) -> Option<SmartReading> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let passed = v.pointer("/smart_status/passed").and_then(|p| p.as_bool());
    // smartctl ran but could not open or read the device.
    passed?;

    let temperature = v
        .pointer("/temperature/current")
        .and_then(|t| t.as_u64())
        .map(|t| t as u32);
    let hours = v.pointer("/power_on_time/hours").and_then(|h| h.as_u64());
    let wear = v
        .pointer("/nvme_smart_health_information_log/percentage_used")
        .and_then(|w| w.as_u64())
        .map(|w| w as u32);

    // ATA attribute 5: sectors the drive has had to remap. A handful is not
    // an emergency, any is a reason to keep a backup.
    let reallocated = v
        .pointer("/ata_smart_attributes/table")
        .and_then(|t| t.as_array())
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.get("id").and_then(|i| i.as_u64()) == Some(5))
                .and_then(|r| r.pointer("/raw/value"))
                .and_then(|n| n.as_u64())
        })
        .unwrap_or(0);

    let (state, summary) = judge_disk(
        passed == Some(false),
        reallocated > 0,
        passed == Some(true),
        wear,
        temperature,
    );
    Some(SmartReading {
        state,
        summary,
        temperature_c: temperature,
        wear_percent: wear,
        power_on_hours: hours,
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn read_linux_battery(dir: &std::path::Path) -> Option<BatteryHealth> {
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    };
    // Values are in µWh (energy_*) or µAh (charge_*); report whole mWh / mAh.
    let (design, full, unit) = match (read("energy_full_design"), read("energy_full")) {
        (Some(d), Some(f)) => (Some(d / 1000), Some(f / 1000), "mWh"),
        _ => (
            read("charge_full_design").map(|v| v / 1000),
            read("charge_full").map(|v| v / 1000),
            "mAh",
        ),
    };
    if design.is_none() && full.is_none() {
        return None;
    }
    let cycles = read("cycle_count").map(|c| c as u32).filter(|c| *c > 0);
    Some(battery_from(design, full, cycles, unit))
}

#[cfg(target_os = "linux")]
pub async fn report() -> HealthReport {
    use odysync_core::proc;
    use std::path::Path;
    use std::time::Duration;

    let mut disks = Vec::new();
    let mut notes = Vec::new();
    let mut unreadable = false;

    let entries = std::fs::read_dir("/sys/block")
        .into_iter()
        .flatten()
        .flatten();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_physical_block_device(&name) {
            continue;
        }
        let base = entry.path();
        let read = |rel: &str| {
            std::fs::read_to_string(base.join(rel))
                .ok()
                .map(|s| s.trim().to_string())
        };
        let sectors: u64 = read("size").and_then(|s| s.parse().ok()).unwrap_or(0);
        if sectors == 0 {
            continue;
        }
        let nvme = name.starts_with("nvme");
        let ssd = read("queue/rotational").map(|r| r == "0");
        let model = read("device/model").filter(|m| !m.is_empty());

        let smart = proc::run(
            "smartctl",
            &["--json=c", "-H", "-A", &format!("/dev/{name}")],
            Duration::from_secs(30),
        )
        .await
        .ok()
        .and_then(|o| parse_smartctl(&o.stdout));

        let SmartReading {
            state,
            summary,
            temperature_c,
            wear_percent,
            power_on_hours,
        } = smart.unwrap_or_else(|| {
            unreadable = true;
            SmartReading {
                state: HealthState::Unknown,
                summary: "SMART data could not be read for this disk.".to_string(),
                temperature_c: None,
                wear_percent: None,
                power_on_hours: None,
            }
        });

        disks.push(DiskHealth {
            name: model.unwrap_or_else(|| name.clone()),
            kind: disk_kind(ssd, nvme),
            size_bytes: sectors * 512,
            state,
            summary,
            temperature_c,
            wear_percent,
            power_on_hours,
        });
    }
    if unreadable {
        notes.push(
            "Disk health comes from smartctl. Install smartmontools and run Odysync \
             with root rights to read it."
                .into(),
        );
    }

    let battery = std::fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery")
                // Mice and headsets report as batteries too, with scope Device.
                && std::fs::read_to_string(p.join("scope")).map_or(true, |s| s.trim() != "Device")
        })
        .find_map(|p| read_linux_battery(Path::new(&p)));

    HealthReport {
        disks,
        battery,
        notes,
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn report() -> HealthReport {
    HealthReport {
        disks: Vec::new(),
        battery: None,
        notes: vec!["Hardware health is not available on this platform.".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worst_state_never_lets_unknown_hide_a_problem() {
        assert_eq!(
            HealthState::Good.worst(HealthState::Unknown),
            HealthState::Unknown
        );
        assert_eq!(
            HealthState::Bad.worst(HealthState::Unknown),
            HealthState::Bad
        );
        assert_eq!(
            HealthState::Unknown.worst(HealthState::Warning),
            HealthState::Warning
        );
    }

    #[test]
    fn windows_disks_are_judged_from_status_wear_and_temperature() {
        let json = r#"[
          {"Name":"Samsung SSD 990 PRO 2TB","MediaType":"SSD","BusType":"NVMe","Size":2000398934016,
           "HealthStatus":"Healthy","Temperature":41,"Wear":3,"PowerOnHours":4211},
          {"Name":"WDC WD40EFRX","MediaType":"HDD","BusType":"SATA","Size":4000787030016,
           "HealthStatus":"Warning","Temperature":null,"Wear":null,"PowerOnHours":null},
          {"Name":"USB Stick","MediaType":"Unspecified","BusType":"USB","Size":32000000000,
           "HealthStatus":"Healthy","Temperature":0,"Wear":null,"PowerOnHours":null}
        ]"#;
        let disks = parse_windows_disks(json);
        assert_eq!(disks.len(), 3);

        assert_eq!(disks[0].kind, "NVMe SSD");
        assert_eq!(disks[0].state, HealthState::Good);
        assert_eq!(disks[0].wear_percent, Some(3));
        assert_eq!(disks[0].temperature_c, Some(41));

        assert_eq!(disks[1].kind, "Hard disk");
        assert_eq!(disks[1].state, HealthState::Warning);

        // Temperature 0 means "not measured", not "freezing".
        assert_eq!(disks[2].temperature_c, None);
        assert_eq!(disks[2].kind, "Disk");
        assert_eq!(disks[2].state, HealthState::Good);
    }

    #[test]
    fn a_worn_out_or_failing_disk_is_bad() {
        let json = r#"[{"Name":"Old","MediaType":"SSD","BusType":"SATA","Size":1,
                        "HealthStatus":"Healthy","Wear":100}]"#;
        assert_eq!(parse_windows_disks(json)[0].state, HealthState::Bad);
        let json = r#"[{"Name":"Dying","MediaType":"HDD","BusType":"SATA","Size":1,
                        "HealthStatus":"Unhealthy"}]"#;
        assert_eq!(parse_windows_disks(json)[0].state, HealthState::Bad);
    }

    #[test]
    fn a_hot_but_otherwise_healthy_disk_is_a_warning() {
        let (state, summary) = judge_disk(false, false, true, Some(10), Some(74));
        assert_eq!(state, HealthState::Warning);
        assert!(summary.contains("74"));
    }

    #[test]
    fn a_disk_that_reports_nothing_is_unknown_not_good() {
        assert_eq!(
            judge_disk(false, false, false, None, None).0,
            HealthState::Unknown
        );
    }

    #[test]
    fn garbage_json_yields_no_disks() {
        assert!(parse_windows_disks("").is_empty());
        assert!(parse_windows_disks("not json").is_empty());
    }

    #[test]
    fn windows_battery_capacity_is_a_share_of_the_design_value() {
        let b =
            parse_windows_battery(r#"{"Present":true,"Design":57000,"Full":45600,"Cycles":312}"#)
                .unwrap();
        assert_eq!(b.capacity_percent, Some(80));
        assert_eq!(b.state, HealthState::Good);
        assert_eq!(b.cycle_count, Some(312));

        let worn =
            parse_windows_battery(r#"{"Present":true,"Design":50000,"Full":25000,"Cycles":0}"#)
                .unwrap();
        assert_eq!(worn.state, HealthState::Bad);
        assert_eq!(worn.cycle_count, None, "0 cycles means not counted");
    }

    #[test]
    fn a_desktop_has_no_battery() {
        assert!(parse_windows_battery(r#"{"Present":false,"Design":null,"Full":null}"#).is_none());
    }

    #[test]
    fn battery_without_capacity_figures_is_unknown() {
        let b =
            parse_windows_battery(r#"{"Present":true,"Design":null,"Full":null,"Cycles":null}"#)
                .unwrap();
        assert_eq!(b.state, HealthState::Unknown);
    }

    #[test]
    fn capacity_above_design_is_capped_at_100() {
        assert_eq!(percent(5200, 5000), Some(100));
        assert_eq!(percent(0, 5000), None);
        assert_eq!(percent(100, 0), None);
    }

    const DISKUTIL_LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>WholeDisks</key><array><string>disk0</string><string>disk4</string></array>
</dict></plist>"#;

    const DISKUTIL_INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>BusProtocol</key><string>Apple Fabric</string>
  <key>DeviceIdentifier</key><string>disk0</string>
  <key>MediaName</key><string>APPLE SSD AP1024Z</string>
  <key>SMARTStatus</key><string>Verified</string>
  <key>SolidState</key><true/>
  <key>TotalSize</key><integer>1000555581440</integer>
  <key>VirtualOrPhysical</key><string>Physical</string>
</dict></plist>"#;

    #[test]
    fn diskutil_whole_disks_and_info_are_parsed() {
        assert_eq!(
            parse_diskutil_whole_disks(DISKUTIL_LIST.as_bytes()),
            vec!["disk0".to_string(), "disk4".to_string()]
        );
        let d = parse_diskutil_info(DISKUTIL_INFO.as_bytes()).unwrap();
        assert_eq!(d.name, "APPLE SSD AP1024Z");
        assert_eq!(d.kind, "NVMe SSD");
        assert_eq!(d.state, HealthState::Good);
        assert_eq!(d.size_bytes, 1_000_555_581_440);

        let failing = DISKUTIL_INFO.replace("Verified", "Failing");
        assert_eq!(
            parse_diskutil_info(failing.as_bytes()).unwrap().state,
            HealthState::Bad
        );
        let virt = DISKUTIL_INFO.replace(">Physical<", ">Virtual<");
        assert!(parse_diskutil_info(virt.as_bytes()).is_none());
    }

    #[test]
    fn apple_silicon_battery_uses_the_raw_capacity() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><array><dict>
  <key>AppleRawMaxCapacity</key><integer>4382</integer>
  <key>CycleCount</key><integer>187</integer>
  <key>DesignCapacity</key><integer>4563</integer>
  <key>MaxCapacity</key><integer>100</integer>
</dict></array></plist>"#;
        let b = parse_apple_smart_battery(xml.as_bytes()).unwrap();
        assert_eq!(b.capacity_percent, Some(96));
        assert_eq!(b.cycle_count, Some(187));
        assert_eq!(b.capacity_unit, "mAh");
        assert!(parse_apple_smart_battery(b"<plist version=\"1.0\"><array/></plist>").is_none());
    }

    #[test]
    fn smartctl_nvme_and_ata_reports_are_read() {
        let nvme = r#"{"smart_status":{"passed":true},"temperature":{"current":38},
          "power_on_time":{"hours":1200},
          "nvme_smart_health_information_log":{"percentage_used":2}}"#;
        let r = parse_smartctl(nvme).unwrap();
        assert_eq!(
            (r.state, r.temperature_c, r.wear_percent, r.power_on_hours),
            (HealthState::Good, Some(38), Some(2), Some(1200))
        );

        let ata = r#"{"smart_status":{"passed":true},
          "ata_smart_attributes":{"table":[{"id":5,"raw":{"value":8}}]}}"#;
        assert_eq!(parse_smartctl(ata).unwrap().state, HealthState::Warning);

        let failed = r#"{"smart_status":{"passed":false}}"#;
        assert_eq!(parse_smartctl(failed).unwrap().state, HealthState::Bad);

        // Permission denied: smartctl prints JSON without a smart_status.
        assert!(parse_smartctl(r#"{"smartctl":{"exit_status":2}}"#).is_none());
    }

    #[test]
    fn only_real_block_devices_are_considered() {
        assert!(is_physical_block_device("nvme0n1"));
        assert!(is_physical_block_device("sda"));
        for skip in ["loop0", "zram0", "dm-1", "md127", "sr0", "ram0"] {
            assert!(!is_physical_block_device(skip), "{skip}");
        }
    }

    #[test]
    fn linux_battery_reads_energy_or_charge_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("energy_full_design"), "57020000\n").unwrap();
        std::fs::write(dir.path().join("energy_full"), "49030000\n").unwrap();
        std::fs::write(dir.path().join("cycle_count"), "421\n").unwrap();
        let b = read_linux_battery(dir.path()).unwrap();
        assert_eq!(b.capacity_unit, "mWh");
        assert_eq!(b.capacity_percent, Some(86));
        assert_eq!(b.cycle_count, Some(421));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("charge_full_design"), "5000000").unwrap();
        std::fs::write(dir.path().join("charge_full"), "3500000").unwrap();
        let b = read_linux_battery(dir.path()).unwrap();
        assert_eq!(b.capacity_unit, "mAh");
        assert_eq!(b.state, HealthState::Warning);

        assert!(read_linux_battery(tempfile::tempdir().unwrap().path()).is_none());
    }

    #[test]
    fn overall_is_the_worst_part() {
        let disk = |state| DiskHealth {
            name: "d".into(),
            kind: "SSD".into(),
            size_bytes: 1,
            state,
            summary: String::new(),
            temperature_c: None,
            wear_percent: None,
            power_on_hours: None,
        };
        let report = HealthReport {
            disks: vec![disk(HealthState::Good), disk(HealthState::Warning)],
            battery: Some(battery_from(Some(100), Some(50), None, "mWh")),
            notes: vec![],
        };
        assert_eq!(report.overall(), HealthState::Bad);
        let empty = HealthReport {
            disks: vec![],
            battery: None,
            notes: vec![],
        };
        assert_eq!(empty.overall(), HealthState::Unknown);
    }
}
