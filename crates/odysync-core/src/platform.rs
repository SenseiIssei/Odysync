//! Small platform probes shared by the CLI and the backends.

/// Whether this process holds administrator (Windows) or root (Unix) rights.
///
/// Used to decide which backends may run, and — just as importantly — to refuse
/// Microsoft Store updates while elevated, which silently corrupts their
/// install state.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid.
    let mut token: HANDLE = Default::default();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok.is_err() {
        return false;
    }

    // SAFETY: token is a valid handle from OpenProcessToken. The buffer is a
    // TOKEN_ELEVATION struct, which is what TokenElevation expects.
    let mut elevation = TOKEN_ELEVATION::default();
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut 0u32,
        )
    };

    // SAFETY: token is a valid handle; CloseHandle on a pseudo-handle is a no-op
    // but we call it for correctness.
    unsafe { _ = windows::Win32::Foundation::CloseHandle(token) };

    ok.is_ok() && elevation.TokenIsElevated != 0
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid is always safe to call and cannot fail.
    unsafe { libc_geteuid() == 0 }
}

#[cfg(not(windows))]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Whether Windows is waiting on a restart to finish servicing itself.
///
/// `RunReport::reboot_required` was plumbed to the CLI renderer and to three
/// GUI components, and never once set to `true` — no code assigned it. Reading
/// the flag off an individual install result would only have covered the
/// backend that did the reading; asking the OS covers every backend, including
/// a winget package whose installer staged a pending file rename.
///
/// These are the three signals Windows itself consults, and the same set
/// `security::posture` already checks:
///
///   * Component Based Servicing left a `RebootPending` key behind
///   * Windows Update left a `RebootRequired` key behind
///   * a pending file rename is queued for the next boot
///
/// Any one of them means "restart to finish". Absent or unreadable is reported
/// as "no", because a spurious restart prompt trains users to ignore them.
#[cfg(windows)]
pub fn reboot_pending() -> bool {
    use windows::core::w;

    hklm_key_exists(w!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"
    )) || hklm_key_exists(w!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired"
    )) || hklm_value_exists(
        w!(r"SYSTEM\CurrentControlSet\Control\Session Manager"),
        w!("PendingFileRenameOperations"),
    )
}

/// Does this key exist under `HKEY_LOCAL_MACHINE`?
#[cfg(windows)]
fn hklm_key_exists(path: windows::core::PCWSTR) -> bool {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
    };

    let mut key = HKEY::default();
    // SAFETY: `path` is a wide string valid for the call, and `key` is a
    // valid out-param that is closed again whenever the open succeeded.
    let opened = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, None, KEY_READ, &mut key) };
    if opened.is_ok() {
        unsafe { _ = RegCloseKey(key) };
        true
    } else {
        false
    }
}

/// Does this value exist under `HKEY_LOCAL_MACHINE\<path>`?
#[cfg(windows)]
fn hklm_value_exists(path: windows::core::PCWSTR, name: windows::core::PCWSTR) -> bool {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };

    let mut key = HKEY::default();
    // SAFETY: both arguments are wide strings valid for the call; `key` is a
    // valid out-param and is closed on every path that opened it.
    let opened = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, None, KEY_READ, &mut key) };
    if opened.is_err() {
        return false;
    }
    let present = unsafe { RegQueryValueExW(key, name, None, None, None, None) }.is_ok();
    unsafe { _ = RegCloseKey(key) };
    present
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::core::w;

    /// `reboot_pending` returning `false` is only meaningful if the registry
    /// reads underneath it can return `true` at all. Both probes are checked
    /// against keys and values that exist on every Windows install, so a
    /// broken call cannot masquerade as "no restart needed".
    #[test]
    fn the_registry_probes_find_things_that_exist() {
        assert!(hklm_key_exists(w!(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion"
        )));
        assert!(hklm_value_exists(
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuild"),
        ));
    }

    #[test]
    fn the_registry_probes_reject_things_that_do_not_exist() {
        assert!(!hklm_key_exists(w!(
            r"SOFTWARE\Odysync\DefinitelyNotAKeyThatExists"
        )));
        assert!(!hklm_value_exists(
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("OdysyncDefinitelyNotAValue"),
        ));
    }

    /// A pending reboot must be reported as a plain boolean, never a panic —
    /// this runs on every developer machine and CI runner, in both states.
    #[test]
    fn reboot_pending_answers_without_panicking() {
        let _ = reboot_pending();
    }

    #[test]
    fn the_power_source_is_readable_and_is_not_unknown_on_a_real_machine() {
        // CI runners and developer machines are both real Windows installs, so
        // an Unknown here means the probe is broken rather than the machine
        // being unusual.
        let source = power_source();
        assert_ne!(
            source,
            PowerSource::Unknown,
            "could not read the power source on a real Windows machine"
        );
    }

    #[test]
    fn only_running_on_battery_defers_background_work() {
        assert!(!PowerSource::Battery.allows_background_work());
        assert!(PowerSource::Mains.allows_background_work());
        // Unknown proceeds: refusing to update a desktop whose probe failed is
        // worse than one scan on a laptop battery.
        assert!(PowerSource::Unknown.allows_background_work());
    }

    #[test]
    fn resident_memory_is_a_plausible_figure() {
        let rss = resident_memory_bytes().expect("could not read this process's working set");
        // A test binary is at least a megabyte and nothing like a terabyte;
        // this catches a struct-size or units mistake rather than asserting a
        // budget.
        assert!(rss > 1024 * 1024, "implausibly small RSS: {rss}");
        assert!(rss < 8 * 1024 * 1024 * 1024, "implausibly large RSS: {rss}");
    }
}

/// Non-Windows platforms have no equivalent OS-wide signal.
///
/// A Linux kernel update leaves `/var/run/reboot-required` on Debian and
/// nothing at all elsewhere, and macOS does not expose one. Reporting "no"
/// is honest; guessing per-distribution would be worse than saying nothing.
#[cfg(not(windows))]
pub fn reboot_pending() -> bool {
    false
}

/// Where the machine is currently getting its power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerSource {
    /// Plugged in, or a desktop with no battery at all.
    Mains,
    /// Running on battery.
    Battery,
    /// Could not be determined. Treated as mains, because refusing to update a
    /// desktop that failed to answer would be worse than a laptop doing one
    /// scan on battery.
    Unknown,
}

impl PowerSource {
    /// Whether background work should go ahead.
    pub fn allows_background_work(&self) -> bool {
        !matches!(self, PowerSource::Battery)
    }
}

/// Read the current power source.
///
/// A background scanner that wakes every hour and spawns thirty package
/// managers is a laptop-battery problem, and `ROADMAP.md` §4 has had this open
/// since the daemon shipped.
#[cfg(windows)]
pub fn power_source() -> PowerSource {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a valid, correctly sized out-param.
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return PowerSource::Unknown;
    }

    // ACLineStatus: 0 offline, 1 online, 255 unknown.
    // BatteryFlag 128 means "no system battery" — a desktop, which is always
    // on mains however the line status reads.
    if status.BatteryFlag == 128 {
        return PowerSource::Mains;
    }
    match status.ACLineStatus {
        0 => PowerSource::Battery,
        1 => PowerSource::Mains,
        _ => PowerSource::Unknown,
    }
}

/// Whether the machine has a system battery — in practice, whether it is a
/// laptop. NVIDIA, for one, ships separate notebook driver packages.
#[cfg(windows)]
pub fn has_system_battery() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a valid, correctly sized out-param.
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return false;
    }
    // 128 is "no system battery"; 255 is "unknown status", which a desktop
    // without a battery also reports on some firmware.
    !matches!(status.BatteryFlag, 128 | 255)
}

#[cfg(not(windows))]
pub fn has_system_battery() -> bool {
    false
}

/// Non-Windows platforms are not probed.
///
/// Linux exposes this through `/sys/class/power_supply` and macOS through
/// IOKit, but neither is implemented yet, and guessing would be worse than
/// reporting that we did not look.
#[cfg(not(windows))]
pub fn power_source() -> PowerSource {
    PowerSource::Unknown
}

/// Resident set size of this process, in bytes.
///
/// `ROADMAP.md` §4 sets a 15 MB idle target and says "measure, don't assume".
/// This is the measurement.
#[cfg(windows)]
pub fn resident_memory_bytes() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };

    // SAFETY: the pseudo-handle from GetCurrentProcess is always valid, and
    // `counters` is correctly sized via its `cb` field.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok.is_ok() {
        Some(counters.WorkingSetSize as u64)
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn resident_memory_bytes() -> Option<u64> {
    None
}

/// A short label for the current OS, used in reports and the UI.
pub fn os_label() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}
