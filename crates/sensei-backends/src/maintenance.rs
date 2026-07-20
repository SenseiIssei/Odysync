//! Maintenance action implementations.
//!
//! These are system-level housekeeping tasks — temp cleanup, Recycle Bin,
//! DISM/SFC health checks, and the startup-programs viewer — that are not
//! package updates and do not flow through the update policy.

use std::time::Duration;

use async_trait::async_trait;
use sensei_core::error::{Error, Result};
use sensei_core::maintenance::{Maintenance, MaintenanceKind, MaintenanceResult};
use sensei_core::proc;

const MAINTENANCE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// ── Temp cleanup ────────────────────────────────────────────────────────────

pub struct TempCleanup;

impl Default for TempCleanup {
    fn default() -> Self {
        Self
    }
}

impl TempCleanup {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Maintenance for TempCleanup {
    fn kind(&self) -> MaintenanceKind {
        MaintenanceKind::TempCleanup
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self) -> Result<MaintenanceResult> {
        #[cfg(windows)]
        {
            return temp_cleanup_windows().await;
        }
        #[cfg(not(windows))]
        {
            return temp_cleanup_unix().await;
        }
    }
}

#[cfg(windows)]
async fn temp_cleanup_windows() -> Result<MaintenanceResult> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$count = 0
$roots = @($env:TEMP, $env:TMP, "C:\Windows\Temp") | Sort-Object -Unique
foreach ($root in $roots) {
    if (-not $root -or -not (Test-Path $root)) { continue }
    Get-ChildItem -Path $root -Force | ForEach-Object {
        try {
            if ($_.PSIsContainer) {
                Remove-Item -Path $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
            } else {
                Remove-Item -Path $_.FullName -Force -ErrorAction SilentlyContinue
            }
            $count++
        } catch { }
    }
}
Write-Output "COUNT=$count"
"#;
    let out = proc::powershell(script, MAINTENANCE_TIMEOUT).await?;
    let count = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("COUNT="))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);

    Ok(MaintenanceResult {
        kind: MaintenanceKind::TempCleanup,
        success: true,
        summary: format!("Removed {count} temp items"),
    })
}

#[cfg(not(windows))]
async fn temp_cleanup_unix() -> Result<MaintenanceResult> {
    use std::path::PathBuf;

    let mut count = 0usize;
    let dirs: Vec<PathBuf> = vec![
        std::env::temp_dir(),
        PathBuf::from("/tmp"),
    ];

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let _ = std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path));
                count += 1;
            }
        }
    }

    Ok(MaintenanceResult {
        kind: MaintenanceKind::TempCleanup,
        success: true,
        summary: format!("Removed {count} temp items"),
    })
}

// ── Recycle Bin / Trash ─────────────────────────────────────────────────────

pub struct CleanRecycleBin;

impl Default for CleanRecycleBin {
    fn default() -> Self {
        Self
    }
}

impl CleanRecycleBin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Maintenance for CleanRecycleBin {
    fn kind(&self) -> MaintenanceKind {
        MaintenanceKind::CleanRecycleBin
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self) -> Result<MaintenanceResult> {
        #[cfg(windows)]
        {
            let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
try {
    Clear-RecycleBin -Force -ErrorAction SilentlyContinue
    Write-Output "OK"
} catch {
    Write-Output "SKIP"
}
"#;
            let out = proc::powershell(script, Duration::from_secs(60)).await?;
            let ok = out.stdout.contains("OK");
            return Ok(MaintenanceResult {
                kind: MaintenanceKind::CleanRecycleBin,
                success: ok,
                summary: if ok {
                    "Recycle Bin emptied".into()
                } else {
                    "Could not empty Recycle Bin".into()
                },
            });
        }
        #[cfg(not(windows))]
        {
            // On Unix, attempt to empty ~/.local/share/Trash if it exists.
            let trash = std::env::var("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(|h| std::path::PathBuf::from(h).join(".local/share"))
                        .unwrap_or_default()
                })
                .join("Trash");

            if trash.exists() {
                let files = trash.join("files");
                let info = trash.join("info");
                let mut count = 0;
                if files.exists() {
                    if let Ok(entries) = std::fs::read_dir(&files) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            let _ = std::fs::remove_file(&p)
                                .or_else(|_| std::fs::remove_dir_all(&p));
                            count += 1;
                        }
                    }
                }
                if info.exists() {
                    if let Ok(entries) = std::fs::read_dir(&info) {
                        for entry in entries.flatten() {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
                Ok(MaintenanceResult {
                    kind: MaintenanceKind::CleanRecycleBin,
                    success: true,
                    summary: format!("Emptied trash ({count} items)"),
                })
            } else {
                Ok(MaintenanceResult {
                    kind: MaintenanceKind::CleanRecycleBin,
                    success: true,
                    summary: "No trash directory found".into(),
                })
            }
        }
    }
}

// ── DISM + SFC (Windows only) ───────────────────────────────────────────────

pub struct SystemHealth;

impl Default for SystemHealth {
    fn default() -> Self {
        Self
    }
}

impl SystemHealth {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Maintenance for SystemHealth {
    fn kind(&self) -> MaintenanceKind {
        MaintenanceKind::SystemHealth
    }

    async fn is_available(&self) -> bool {
        cfg!(windows)
    }

    async fn run(&self) -> Result<MaintenanceResult> {
        #[cfg(windows)]
        {
            let dism_scan = proc::run(
                "DISM",
                &["/Online", "/Cleanup-Image", "/ScanHealth"],
                MAINTENANCE_TIMEOUT,
            )
            .await;
            let dism_restore = proc::run(
                "DISM",
                &["/Online", "/Cleanup-Image", "/RestoreHealth"],
                MAINTENANCE_TIMEOUT,
            )
            .await;
            let sfc = proc::run(
                "sfc",
                &["/scannow"],
                MAINTENANCE_TIMEOUT,
            )
            .await;

            let dism_scan_ok = matches!(&dism_scan, Ok(o) if o.success());
            let dism_restore_ok = matches!(&dism_restore, Ok(o) if o.success());
            let sfc_ok = matches!(&sfc, Ok(o) if o.success());

            let summary = format!(
                "DISM ScanHealth: {}, DISM RestoreHealth: {}, SFC: {}",
                if dism_scan_ok { "ok" } else { "failed" },
                if dism_restore_ok { "ok" } else { "failed" },
                if sfc_ok { "ok" } else { "failed" },
            );

            return Ok(MaintenanceResult {
                kind: MaintenanceKind::SystemHealth,
                success: dism_scan_ok && dism_restore_ok && sfc_ok,
                summary,
            });
        }
        #[cfg(not(windows))]
        {
            return Ok(MaintenanceResult {
                kind: MaintenanceKind::SystemHealth,
                success: false,
                summary: "DISM/SFC is only available on Windows".into(),
            });
        }
    }
}

// ── Startup programs viewer ─────────────────────────────────────────────────

pub struct StartupPrograms;

impl Default for StartupPrograms {
    fn default() -> Self {
        Self
    }
}

impl StartupPrograms {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Maintenance for StartupPrograms {
    fn kind(&self) -> MaintenanceKind {
        MaintenanceKind::StartupPrograms
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self) -> Result<MaintenanceResult> {
        #[cfg(windows)]
        {
            let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
Get-CimInstance Win32_StartupCommand |
  Select-Object Name, Command, Location |
  Sort-Object Name |
  Format-Table -AutoSize |
  Out-String -Width 4096
"#;
            let out = proc::powershell(script, Duration::from_secs(30)).await?;
            let text = out.stdout.trim().to_string();
            return Ok(MaintenanceResult {
                kind: MaintenanceKind::StartupPrograms,
                success: !text.is_empty(),
                summary: text,
            });
        }
        #[cfg(not(windows))]
        {
            // On Linux, list systemd user units that are enabled.
            let out = proc::run(
                "systemctl",
                &["--user", "list-unit-files", "--state=enabled"],
                Duration::from_secs(15),
            )
            .await;
            let text = match out {
                Ok(o) => o.stdout.trim().to_string(),
                Err(_) => "Could not list startup programs".into(),
            };
            return Ok(MaintenanceResult {
                kind: MaintenanceKind::StartupPrograms,
                success: !text.is_empty(),
                summary: text,
            });
        }
    }
}

/// All maintenance actions available on this platform.
pub fn all_maintenance() -> Vec<Box<dyn Maintenance>> {
    vec![
        Box::new(TempCleanup::new()),
        Box::new(CleanRecycleBin::new()),
        Box::new(SystemHealth::new()),
        Box::new(StartupPrograms::new()),
    ]
}

/// Run a single maintenance action by kind, returning its result.
pub async fn run_maintenance(kind: MaintenanceKind) -> Result<MaintenanceResult> {
    let actions = all_maintenance();
    let action = actions
        .into_iter()
        .find(|a| a.kind() == kind)
        .ok_or_else(|| Error::parse("maintenance", format!("{kind} is not implemented")))?;

    if !action.is_available().await {
        return Ok(MaintenanceResult {
            kind,
            success: false,
            summary: format!("{kind} is not available on this platform"),
        });
    }

    action.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_maintenance_covers_every_kind() {
        let actions = all_maintenance();
        let kinds: Vec<MaintenanceKind> = actions.iter().map(|a| a.kind()).collect();
        assert!(kinds.contains(&MaintenanceKind::TempCleanup));
        assert!(kinds.contains(&MaintenanceKind::CleanRecycleBin));
        assert!(kinds.contains(&MaintenanceKind::SystemHealth));
        assert!(kinds.contains(&MaintenanceKind::StartupPrograms));
    }

    #[test]
    fn maintenance_kinds_have_distinct_ids() {
        let kinds = [
            MaintenanceKind::TempCleanup,
            MaintenanceKind::CleanRecycleBin,
            MaintenanceKind::SystemHealth,
            MaintenanceKind::StartupPrograms,
        ];
        let ids: Vec<&str> = kinds.iter().map(|k| k.id()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }
}
