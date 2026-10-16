//! Commands for the maintenance tools: cleanup, large files, runtime
//! components, device problems, driver backups and disk optimisation.
//!
//! Each one is a thin shell over `odysync-backends`, which is where the safety
//! rules live and are tested; nothing here decides what is safe to delete or
//! install.

use odysync_core::cleanup::{CleanupCategory, Reversibility};
use serde::Serialize;

/// One cleanup category as the GUI needs it: the measurement plus the rules
/// that decide how it may be presented.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupItemDto {
    pub id: String,
    pub label: String,
    pub consequence: String,
    pub bytes: u64,
    pub file_count: usize,
    pub note: Option<String>,
    pub error: Option<String>,
    /// `regenerated`, `discardable` or `loses-capability`.
    pub reversibility: Reversibility,
    pub selected_by_default: bool,
    pub report_only: bool,
    /// The GUI must ask separately before removing this one.
    pub needs_confirmation: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupResultDto {
    pub id: String,
    pub label: String,
    pub removed_bytes: u64,
    pub removed_files: usize,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub async fn cleanup_scan() -> Result<Vec<CleanupItemDto>, String> {
    Ok(odysync_backends::cleanup::scan()
        .await
        .into_iter()
        .map(|f| CleanupItemDto {
            id: f.category.id().to_string(),
            label: f.category.label().to_string(),
            consequence: f.category.consequence().to_string(),
            bytes: f.bytes,
            file_count: f.file_count,
            note: f.note,
            error: f.error,
            reversibility: f.category.reversibility(),
            selected_by_default: f.category.selected_by_default(),
            report_only: f.category.is_report_only(),
            needs_confirmation: f.category.reversibility().needs_explicit_confirmation(),
        })
        .collect())
}

#[tauri::command]
pub async fn cleanup_run(categories: Vec<String>) -> Result<Vec<CleanupResultDto>, String> {
    // An unknown id is an error, not a silent no-op: a typo must not read as
    // "cleaned successfully".
    let mut selected = Vec::new();
    for id in &categories {
        let category = CleanupCategory::from_id(id)
            .ok_or_else(|| format!("unknown cleanup category: {id}"))?;
        selected.push(category);
    }
    Ok(odysync_backends::cleanup::clean(selected)
        .await
        .into_iter()
        .map(|(c, o)| CleanupResultDto {
            id: c.id().to_string(),
            label: c.label().to_string(),
            removed_bytes: o.removed_bytes,
            removed_files: o.removed_files,
            skipped: o.skipped,
        })
        .collect())
}

#[tauri::command]
pub async fn large_files(
    limit: usize,
    min_mb: u64,
) -> Result<odysync_backends::cleanup::LargeFileReport, String> {
    Ok(odysync_backends::cleanup::largest_files(
        limit.clamp(1, 500),
        min_mb.saturating_mul(1024 * 1024),
        std::time::Duration::from_secs(120),
    )
    .await)
}

/// Show a file in the platform's file manager.
///
/// Only an existing file is accepted, and it is passed as a single argument
/// with no shell in between, so a crafted name cannot become a command.
#[tauri::command]
pub async fn reveal_path(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.is_absolute() || !p.exists() {
        return Err("that file no longer exists".into());
    }
    #[cfg(windows)]
    let result = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", p.display()))
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(&p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(&p))
        .spawn();
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn components_status()
-> Result<Vec<odysync_backends::components::ComponentStatus>, String> {
    Ok(odysync_backends::components::status().await)
}

#[tauri::command]
pub async fn component_install(id: String) -> Result<String, String> {
    odysync_backends::components::install(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn device_health() -> Result<odysync_backends::device_health::HealthReport, String> {
    Ok(odysync_backends::device_health::report().await)
}

#[tauri::command]
pub async fn device_problems() -> Result<Vec<odysync_backends::devices::DeviceProblem>, String> {
    Ok(odysync_backends::devices::problems().await)
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverBackupDto {
    pub id: String,
    pub created_at: String,
    pub reason: String,
    pub package_count: usize,
    pub size_bytes: u64,
    pub complete: bool,
}

#[tauri::command]
pub async fn driver_backups() -> Result<Vec<DriverBackupDto>, String> {
    #[cfg(windows)]
    {
        let list = tokio::task::spawn_blocking(odysync_backends::driver_backup::list_backups)
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(list
            .into_iter()
            .map(|m| DriverBackupDto {
                package_count: m.packages.len(),
                size_bytes: m.total_size_bytes(),
                complete: m.is_complete(),
                id: m.id,
                created_at: m.created_at,
                reason: m.reason,
            })
            .collect())
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

#[tauri::command]
pub async fn driver_backup_create(reason: String) -> Result<DriverBackupDto, String> {
    #[cfg(windows)]
    {
        let reason = if reason.trim().is_empty() {
            "taken from the Odysync app".to_string()
        } else {
            reason.trim().chars().take(200).collect()
        };
        let m = odysync_backends::driver_backup::create_backup(&reason)
            .await
            .map_err(|e| e.to_string())?;
        Ok(DriverBackupDto {
            package_count: m.packages.len(),
            size_bytes: m.total_size_bytes(),
            complete: m.is_complete(),
            id: m.id,
            created_at: m.created_at,
            reason: m.reason,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = reason;
        Err("driver backups are a Windows feature".into())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverRestoreDto {
    pub restored: Vec<String>,
    pub already_current: Vec<String>,
    pub failed: Vec<String>,
    pub reboot_required: bool,
}

#[tauri::command]
pub async fn driver_backup_restore(id: String) -> Result<DriverRestoreDto, String> {
    #[cfg(windows)]
    {
        if !odysync_core::platform::is_elevated() {
            return Err("restoring drivers needs administrator rights — use Run as admin".into());
        }
        let r = odysync_backends::driver_backup::restore(&id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(DriverRestoreDto {
            restored: r.restored,
            already_current: r.already_current,
            failed: r.failed,
            reboot_required: r.reboot_required,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = id;
        Err("driver backups are a Windows feature".into())
    }
}

#[tauri::command]
pub async fn driver_backup_prune(keep: usize) -> Result<usize, String> {
    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || odysync_backends::driver_backup::prune(keep.max(1)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = keep;
        Ok(0)
    }
}

#[tauri::command]
pub async fn disk_optimization_plan()
-> Result<Vec<odysync_backends::cleanup::VolumeOptimization>, String> {
    odysync_backends::cleanup::plan_disk_optimization()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disk_optimization_run() -> Result<Vec<String>, String> {
    let plan = odysync_backends::cleanup::plan_disk_optimization()
        .await
        .map_err(|e| e.to_string())?;
    Ok(odysync_backends::cleanup::run_disk_optimization(&plan).await)
}
