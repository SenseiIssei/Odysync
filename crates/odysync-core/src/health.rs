//! Pre-update health checks.
//!
//! Before applying updates, the runner can consult these checks to determine
//! whether the system is in a safe state for updating.  If any check fails,
//! the update is blocked with a descriptive reason.

use std::path::Path;

/// Result of a single health check.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// Machine-readable check name.
    pub check: &'static str,
    /// Whether the check passed (system is safe for updates).
    pub passed: bool,
    /// Human-readable detail, especially important on failure.
    pub detail: String,
}

impl HealthCheckResult {
    fn pass(check: &'static str) -> Self {
        Self {
            check,
            passed: true,
            detail: String::new(),
        }
    }

    fn fail(check: &'static str, detail: impl Into<String>) -> Self {
        Self {
            check,
            passed: false,
            detail: detail.into(),
        }
    }
}

/// Run all applicable health checks for the current platform.
pub async fn run_health_checks() -> Vec<HealthCheckResult> {
    let mut results = Vec::new();

    results.push(check_disk_space().await);
    results.push(check_battery_or_ac_power().await);

    #[cfg(windows)]
    {
        results.push(check_windows_update_in_progress().await);
    }

    #[cfg(target_os = "linux")]
    {
        results.push(check_no_running_package_manager().await);
    }

    results
}

/// Whether all health checks passed.
pub fn all_passed(results: &[HealthCheckResult]) -> bool {
    results.iter().all(|r| r.passed)
}

/// Collect failure reasons from failed checks.
pub fn failure_reasons(results: &[HealthCheckResult]) -> Vec<String> {
    results
        .iter()
        .filter(|r| !r.passed)
        .map(|r| format!("{}: {}", r.check, r.detail))
        .collect()
}

/// Check that there is at least 2 GB of free disk space on the system drive.
async fn check_disk_space() -> HealthCheckResult {
    let min_bytes: u64 = 2 * 1024 * 1024 * 1024; // 2 GB

    #[cfg(windows)]
    {
        let drive = std::env::var_os("SystemDrive").unwrap_or_else(|| "C:".into());
        // `SystemDrive` is `C:` — a drive prefix with no root, which means
        // "current directory on C:", not "the root of C:". Joining `\` is what
        // turns it into `C:\`. clippy's join_absolute_paths lint does not model
        // the Windows drive-relative case and flags this incorrectly.
        #[allow(clippy::join_absolute_paths)]
        let root = Path::new(&drive).join("\\");
        match get_free_space(&root) {
            Some(free) if free >= min_bytes => HealthCheckResult::pass("disk-space"),
            Some(free) => HealthCheckResult::fail(
                "disk-space",
                format!(
                    "only {:.1} GB free on {} (need at least 2 GB)",
                    free as f64 / (1024.0 * 1024.0 * 1024.0),
                    drive.to_string_lossy()
                ),
            ),
            None => HealthCheckResult::fail("disk-space", "could not determine free disk space"),
        }
    }

    #[cfg(not(windows))]
    {
        let root = Path::new("/");
        match get_free_space(root) {
            Some(free) if free >= min_bytes => HealthCheckResult::pass("disk-space"),
            Some(free) => HealthCheckResult::fail(
                "disk-space",
                format!(
                    "only {:.1} GB free on / (need at least 2 GB)",
                    free as f64 / (1024.0 * 1024.0 * 1024.0)
                ),
            ),
            None => HealthCheckResult::fail("disk-space", "could not determine free disk space"),
        }
    }
}

/// Check that a laptop is on AC power (or that this is a desktop).
async fn check_battery_or_ac_power() -> HealthCheckResult {
    #[cfg(windows)]
    {
        use std::process::Command;
        let ps = "Get-WmiObject -Class BatteryStatus -Namespace root\\wmi | Select-Object -First 1 -ExpandProperty PowerOnline";
        let out = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps])
            .output();

        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    // No battery found — likely a desktop.
                    HealthCheckResult::pass("power")
                } else if s == "True" || s == "$true" {
                    HealthCheckResult::pass("power")
                } else {
                    HealthCheckResult::fail("power", "laptop is not on AC power")
                }
            }
            _ => {
                // Could not query — assume OK to avoid blocking updates on desktops.
                HealthCheckResult::pass("power")
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let bat_path = Path::new("/sys/class/power_supply");
        if !bat_path.exists() {
            return HealthCheckResult::pass("power");
        }

        let mut on_ac = false;
        let mut has_battery = false;
        if let Ok(entries) = std::fs::read_dir(bat_path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let online_path = bat_path.join(&name).join("online");
                if online_path.exists() {
                    has_battery = true;
                    if let Ok(content) = std::fs::read_to_string(&online_path) {
                        if content.trim() == "1" {
                            on_ac = true;
                        }
                    }
                }
            }
        }

        if !has_battery || on_ac {
            HealthCheckResult::pass("power")
        } else {
            HealthCheckResult::fail("power", "laptop is not on AC power")
        }
    }

    #[cfg(target_os = "macos")]
    {
        // pmset -g batt returns battery info; if no battery, it says "AC Power".
        use std::process::Command;
        let out = Command::new("pmset").args(["-g", "batt"]).output();

        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                // The second branch used to test `contains("AC")` separately,
                // which is subsumed by "AC Power" and produced an identical
                // body — a clippy error under `-D warnings`.
                if s.contains("AC") || !s.contains("Battery") {
                    HealthCheckResult::pass("power")
                } else {
                    HealthCheckResult::fail("power", "laptop is not on AC power")
                }
            }
            _ => HealthCheckResult::pass("power"),
        }
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        HealthCheckResult::pass("power")
    }
}

#[cfg(windows)]
async fn check_windows_update_in_progress() -> HealthCheckResult {
    // Check if wuauclt.exe (Windows Update service) is actively using the CPU.
    // This is a heuristic — there's no clean API to check if WU is mid-update.
    use std::process::Command;
    let out = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Process -Name wuauclt -ErrorAction SilentlyContinue | Measure-Object | Select-Object -ExpandProperty Count",
        ])
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let count: i32 = String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
            if count > 0 {
                HealthCheckResult::fail(
                    "windows-update-busy",
                    "Windows Update is currently running; wait for it to finish",
                )
            } else {
                HealthCheckResult::pass("windows-update-busy")
            }
        }
        _ => HealthCheckResult::pass("windows-update-busy"),
    }
}

#[cfg(target_os = "linux")]
async fn check_no_running_package_manager() -> HealthCheckResult {
    // Check for lock files that indicate a package manager is running.
    let lock_files = [
        "/var/lib/dpkg/lock-frontend",
        "/var/lib/apt/lists/lock",
        "/var/cache/dnf/metadata_lock",
        "/var/lib/rpm/.rpm.lock",
        "/var/lib/pacman/db.lck",
    ];

    for &lock in &lock_files {
        if Path::new(lock).exists() {
            // Check if the lock is held (flock-style check is complex; just
            // check if the process has been recently active by seeing if
            // the lock file is non-empty or very recently modified).
            if let Ok(metadata) = std::fs::metadata(lock) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(elapsed) = modified.elapsed() {
                        // If the lock was modified in the last 60 seconds,
                        // assume a package manager is actively running.
                        if elapsed.as_secs() < 60 {
                            return HealthCheckResult::fail(
                                "package-manager-busy",
                                format!("package manager lock {} is recently active", lock),
                            );
                        }
                    }
                }
            }
        }
    }

    HealthCheckResult::pass("package-manager-busy")
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn get_free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut buf = unsafe { std::mem::zeroed::<libc::statvfs>() };
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut buf) };
        if ret == 0 {
            Some(buf.f_bavail as u64 * buf.f_frsize as u64)
        } else {
            None
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                free_bytes_available: *mut u64,
                total_bytes: *mut u64,
                total_free_bytes: *mut u64,
            ) -> i32;
        }

        let mut free: u64 = 0;
        let mut total: u64 = 0;
        let mut total_free: u64 = 0;
        let ret =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
        if ret != 0 {
            Some(free)
        } else {
            None
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn get_free_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disk_space_check_runs() {
        let result = check_disk_space().await;
        assert_eq!(result.check, "disk-space");
    }

    #[tokio::test]
    async fn power_check_runs() {
        let result = check_battery_or_ac_power().await;
        assert_eq!(result.check, "power");
    }

    #[tokio::test]
    async fn all_health_checks_run() {
        let results = run_health_checks().await;
        assert!(!results.is_empty());
    }

    #[test]
    fn all_passed_returns_true_for_all_passing() {
        let results = vec![HealthCheckResult::pass("a"), HealthCheckResult::pass("b")];
        assert!(all_passed(&results));
    }

    #[test]
    fn all_passed_returns_false_for_any_failure() {
        let results = vec![
            HealthCheckResult::pass("a"),
            HealthCheckResult::fail("b", "something"),
        ];
        assert!(!all_passed(&results));
    }

    #[test]
    fn failure_reasons_collects_only_failures() {
        let results = vec![
            HealthCheckResult::pass("a"),
            HealthCheckResult::fail("b", "broken"),
            HealthCheckResult::fail("c", "also broken"),
        ];
        let reasons = failure_reasons(&results);
        assert_eq!(reasons.len(), 2);
        assert!(reasons[0].contains("b: broken"));
        assert!(reasons[1].contains("c: also broken"));
    }
}
