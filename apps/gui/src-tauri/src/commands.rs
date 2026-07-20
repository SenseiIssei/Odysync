use crate::state::AppState;
use serde::{Deserialize, Serialize};
use odysync_core::config::Config;
use odysync_core::maintenance::MaintenanceKind;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::report::RunReport;
use odysync_core::runner::{ProgressEmitter, ProgressEvent, Runner};
use tauri::{AppHandle, Emitter, State};

/// Bridge between the Runner's progress events and Tauri's event system.
struct TauriProgressEmitter {
    app: AppHandle,
}

impl ProgressEmitter for TauriProgressEmitter {
    fn emit_progress(&self, event: ProgressEvent) {
        let _ = self.app.emit("apply-progress", &event);
    }
}

// ── DTOs for the frontend ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ScanResult {
    pub actionable: Vec<UpdateDto>,
    pub skipped: Vec<SkippedDto>,
    pub total: usize,
}

#[derive(Serialize, Deserialize)]
pub struct UpdateDto {
    pub backend: String,
    pub id: String,
    pub name: String,
    pub installed: String,
    pub available: String,
    pub size_bytes: Option<u64>,
}

#[derive(Serialize)]
pub struct SkippedDto {
    pub backend: String,
    pub id: String,
    pub name: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct BackendDto {
    pub kind: String,
    pub name: String,
    pub available: bool,
}

#[derive(Serialize)]
pub struct SystemInfoDto {
    pub os: String,
    pub elevated: bool,
    pub version: String,
}

#[derive(Serialize)]
pub struct ApplyResultDto {
    pub updated: usize,
    pub failed: usize,
    pub skipped: usize,
    pub reboot_required: bool,
    pub exit_code: i32,
    pub entries: Vec<ApplyEntryDto>,
}

#[derive(Serialize)]
pub struct ApplyEntryDto {
    pub name: String,
    pub outcome: String,
}

#[derive(Deserialize)]
pub struct ApplyRequest {
    pub updates: Vec<UpdateDto>,
    pub dry_run: bool,
    pub restore_point: bool,
}

#[derive(Deserialize)]
pub struct HoldRequest {
    pub backend: String,
    pub id: String,
}

#[derive(Deserialize)]
pub struct ScheduleRequest {
    pub frequency: String,
    pub time: String,
    pub task_name: Option<String>,
}

// ── Helper conversions ───────────────────────────────────────────────────────

fn backend_kind_from_str(s: &str) -> Option<BackendKind> {
    match s {
        "winget" => Some(BackendKind::Winget),
        "msstore" => Some(BackendKind::MsStore),
        "windows-drivers" => Some(BackendKind::WindowsDrivers),
        "homebrew" => Some(BackendKind::Homebrew),
        "softwareupdate" => Some(BackendKind::MacSoftwareUpdate),
        "apt" => Some(BackendKind::Apt),
        "dnf" => Some(BackendKind::Dnf),
        "pacman" => Some(BackendKind::Pacman),
        "flatpak" => Some(BackendKind::Flatpak),
        "nvidia-gpu" => Some(BackendKind::NvidiaGpu),
        "amd-gpu" => Some(BackendKind::AmdGpu),
        "intel-gpu" => Some(BackendKind::IntelGpu),
        "dell-command-update" => Some(BackendKind::DellCommandUpdate),
        "hp-image-assistant" => Some(BackendKind::HpImageAssistant),
        "lenovo-system-update" => Some(BackendKind::LenovoSystemUpdate),
        "msi-center" => Some(BackendKind::MsiCenter),
        "fwupd" => Some(BackendKind::Fwupd),
        "mac-firmware" => Some(BackendKind::MacFirmware),
        "snap" => Some(BackendKind::Snap),
        "zypper" => Some(BackendKind::Zypper),
        "chocolatey" => Some(BackendKind::Chocolatey),
        "scoop" => Some(BackendKind::Scoop),
        "nix" => Some(BackendKind::Nix),
        "appimage" => Some(BackendKind::AppImage),
        "asus-armoury" => Some(BackendKind::AsusArmoury),
        "gigabyte-control-center" => Some(BackendKind::GigabyteControlCenter),
        "acer-care-center" => Some(BackendKind::AcerCareCenter),
        "razer-synapse" => Some(BackendKind::RazerSynapse),
        "qualcomm-gpu" => Some(BackendKind::QualcommGpu),
        "virtualization-guest" => Some(BackendKind::VirtualizationGuest),
        "pip" => Some(BackendKind::Pip),
        "cargo" => Some(BackendKind::Cargo),
        "npm" => Some(BackendKind::Npm),
        "go" => Some(BackendKind::Go),
        "dotnet-tool" => Some(BackendKind::DotnetTool),
        "vscode-extension" => Some(BackendKind::VscodeExtension),
        "powershell-module" => Some(BackendKind::PowerShellModule),
        "nvidia-geforce-experience" => Some(BackendKind::NvidiaGeForceExperience),
        "intel-dsa" => Some(BackendKind::IntelDsa),
        "jetbrains-plugin" => Some(BackendKind::JetbrainsPlugin),
        "windows-optional-feature" => Some(BackendKind::WindowsOptionalFeature),
        "dell-firmware" => Some(BackendKind::DellFirmware),
        "hp-firmware" => Some(BackendKind::HpFirmware),
        "lenovo-firmware" => Some(BackendKind::LenovoFirmware),
        _ => None,
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan(state: State<'_, AppState>) -> Result<ScanResult, String> {
    let config = state.config.lock().unwrap().clone();
    let backends = odysync_backends::detect_backends(&config).await;

    let results: Vec<(String, odysync_core::error::Result<Vec<UpdateCandidate>>)> = futures::future::join_all(
        backends.iter().map(|b| async move {
            let kind_id = b.kind().id().to_string();
            let result = b.scan().await;
            (kind_id, result)
        }),
    )
    .await;

    let mut actionable = Vec::new();
    let mut skipped = Vec::new();
    let mut scan_cache = state.scan_cache.lock().unwrap();
    scan_cache.clear();

    for (i, (kind_id, result)) in results.into_iter().enumerate() {
        let candidates = match result {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(backend = %kind_id, error = %e, "scan failed");
                continue;
            }
        };
        // Cache candidates for apply so we don't re-scan.
        scan_cache.insert(kind_id.clone(), candidates.clone());
        let plan = config.policy.plan(candidates);

        for entry in &plan {
            match &entry.blocked_by {
                Some(reason) => {
                    skipped.push(SkippedDto {
                        backend: backends[i].kind().id().to_string(),
                        id: entry.candidate.id.to_string(),
                        name: entry.candidate.name.clone(),
                        reason: reason.to_string(),
                    });
                }
                None => {
                    actionable.push(UpdateDto {
                        backend: backends[i].kind().id().to_string(),
                        id: entry.candidate.id.to_string(),
                        name: entry.candidate.name.clone(),
                        installed: entry.candidate.installed.raw().to_string(),
                        available: entry.candidate.available.raw().to_string(),
                        size_bytes: entry.candidate.size_bytes,
                    });
                }
            }
        }
    }

    let total = actionable.len() + skipped.len();
    Ok(ScanResult {
        actionable,
        skipped,
        total,
    })
}

#[tauri::command]
pub async fn apply(
    app: AppHandle,
    request: ApplyRequest,
    state: State<'_, AppState>,
) -> Result<ApplyResultDto, String> {
    let mut config = state.config.lock().unwrap().clone();
    config.policy.elevated = odysync_core::platform::is_elevated();

    // Use cached scan results instead of re-scanning.
    let cache = state.scan_cache.lock().unwrap().clone();

    let mut candidates_to_apply: Vec<UpdateCandidate> = Vec::new();
    for req_update in &request.updates {
        let Some(candidates) = cache.get(&req_update.backend) else {
            continue;
        };
        for c in candidates {
            if c.id.to_string() == req_update.id {
                candidates_to_apply.push(c.clone());
            }
        }
    }

    if candidates_to_apply.is_empty() {
        return Err("no matching candidates found in scan cache — please scan first".into());
    }

    // Detect backends for applying.
    let backends = odysync_backends::detect_backends(&config).await;

    let plan = config.policy.plan(candidates_to_apply);
    let mut runner = Runner::new(backends.iter().map(|b| b.as_ref()), request.dry_run);

    // Emit progress events during apply.
    let emitter = TauriProgressEmitter { app: app.clone() };
    let mut report = RunReport::new();
    runner.run_with_progress(&plan, &mut report, request.restore_point, Some(&emitter)).await;
    report.finish();

    let entries: Vec<ApplyEntryDto> = report
        .entries
        .iter()
        .map(|e| ApplyEntryDto {
            name: e.name.clone(),
            outcome: format!("{:?}", e.outcome),
        })
        .collect();

    Ok(ApplyResultDto {
        updated: report.updated(),
        failed: report.failed(),
        skipped: report.skipped(),
        reboot_required: report.reboot_required,
        exit_code: report.exit_code(),
        entries,
    })
}

#[tauri::command]
pub async fn list_backends(state: State<'_, AppState>) -> Result<Vec<BackendDto>, String> {
    let config = state.config.lock().unwrap().clone();
    let backends = odysync_backends::detect_backends(&config).await;

    let mut result = Vec::new();
    for backend in &backends {
        result.push(BackendDto {
            kind: backend.kind().id().to_string(),
            name: backend.display_name().to_string(),
            available: backend.is_available().await,
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    let config = state.config.lock().unwrap().clone();
    Ok(config)
}

#[tauri::command]
pub async fn save_config(
    config: Config,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let path = state.config_path.clone();
    config.save(&path).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
pub async fn hold(
    request: HoldRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().unwrap().clone();
    let Some(kind) = backend_kind_from_str(&request.backend) else {
        return Err(format!("unknown backend: {}", request.backend));
    };
    let id = PackageId::new(kind, request.id);
    config.policy.holds.retain(|h| h.package != id.to_string());
    config.policy.holds.push(odysync_core::policy::Hold {
        package: id.to_string(),
        pin: None,
        note: None,
    });
    let path = state.config_path.clone();
    config.save(&path).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
pub async fn unhold(
    request: HoldRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().unwrap().clone();
    let Some(kind) = backend_kind_from_str(&request.backend) else {
        return Err(format!("unknown backend: {}", request.backend));
    };
    let id = PackageId::new(kind, request.id);
    config.policy.holds.retain(|h| h.package != id.to_string());
    let path = state.config_path.clone();
    config.save(&path).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
pub async fn run_maintenance(
    action: String,
) -> Result<String, String> {
    let kind = match action.as_str() {
        "temp_cleanup" => MaintenanceKind::TempCleanup,
        "clean_recycle_bin" => MaintenanceKind::CleanRecycleBin,
        "system_health" => MaintenanceKind::SystemHealth,
        "startup_programs" => MaintenanceKind::StartupPrograms,
        _ => return Err(format!("unknown maintenance action: {action}")),
    };

    let result = odysync_backends::maintenance::run_maintenance(kind)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.summary)
}

#[tauri::command]
pub async fn list_maintenance() -> Result<Vec<String>, String> {
    Ok(vec![
        "temp_cleanup".to_string(),
        "clean_recycle_bin".to_string(),
        "system_health".to_string(),
        "startup_programs".to_string(),
    ])
}

#[tauri::command]
pub async fn create_schedule(
    request: ScheduleRequest,
) -> Result<String, String> {
    use odysync_backends::scheduler::{create_schedule, ScheduleFrequency, ScheduleSpec};

    let freq = match request.frequency.as_str() {
        "daily" => ScheduleFrequency::Daily,
        "weekly" => ScheduleFrequency::Weekly,
        _ => return Err(format!("unknown frequency: {}", request.frequency)),
    };

    let task_name = request.task_name.unwrap_or_else(|| {
        odysync_backends::scheduler::DEFAULT_TASK_NAME.to_string()
    });

    let spec = ScheduleSpec {
        frequency: freq,
        time: request.time,
        task_name: task_name.clone(),
        extra_args: Vec::new(),
    };

    create_schedule(&spec)
        .await
        .map_err(|e| e.to_string())?;

    Ok(task_name)
}

#[tauri::command]
pub async fn remove_schedule(task_name: String) -> Result<bool, String> {
    let existed = odysync_backends::scheduler::schedule_exists(&task_name).await;
    if existed {
        odysync_backends::scheduler::remove_schedule(&task_name)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(existed)
}

#[tauri::command]
pub async fn check_schedule(task_name: String) -> Result<bool, String> {
    Ok(odysync_backends::scheduler::schedule_exists(&task_name).await)
}

#[tauri::command]
pub async fn create_diagnostics(
    out_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    let path = std::path::PathBuf::from(out_path);
    odysync_backends::diagnostics::create_diagnostics(&path, &config, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfoDto, String> {
    Ok(SystemInfoDto {
        os: odysync_core::platform::os_label().to_string(),
        elevated: odysync_core::platform::is_elevated(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
pub async fn background_scan(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let result = scan(state).await?;

    if !result.actionable.is_empty() {
        let _ = app.emit(
            "updates-available",
            serde_json::json!({
                "count": result.actionable.len(),
            }),
        );
    }

    Ok(result)
}

// ── Update History ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct HistoryEntryDto {
    pub timestamp: String,
    pub package: String,
    pub backend: String,
    pub from_version: String,
    pub to_version: String,
    pub outcome: String,
}

#[tauri::command]
pub async fn get_update_history() -> Result<Vec<HistoryEntryDto>, String> {
    let history = odysync_core::history::UpdateHistory::load();
    let entries = history.entries().iter().rev().map(|e| HistoryEntryDto {
        timestamp: e.timestamp.to_rfc3339(),
        package: e.package_name.clone(),
        backend: e.backend.id().to_string(),
        from_version: e.from_version.clone(),
        to_version: e.to_version.clone(),
        outcome: format!("{:?}", e.outcome),
    }).collect();
    Ok(entries)
}

#[tauri::command]
pub async fn clear_update_history() -> Result<(), String> {
    let mut history = odysync_core::history::UpdateHistory::load();
    history.clear();
    history.save().map_err(|e| e.to_string())
}

// ── Hardware Info ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct HardwareInfoDto {
    pub cpu: String,
    pub cpu_cores: u32,
    pub total_memory_gb: f64,
    pub os: String,
    pub gpu: Vec<GpuInfoDto>,
    pub disks: Vec<DiskInfoDto>,
}

#[derive(Serialize)]
pub struct GpuInfoDto {
    pub name: String,
    pub driver_version: String,
    pub vendor: String,
}

#[derive(Serialize)]
pub struct DiskInfoDto {
    pub name: String,
    pub size_gb: f64,
    pub filesystem: String,
}

#[tauri::command]
pub async fn get_hardware_info() -> Result<HardwareInfoDto, String> {
    let os = odysync_core::platform::os_label().to_string();

    let cpu = if cfg!(windows) {
        let out = proc::run("powershell", &["-NoProfile", "-Command",
            "Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name"], std::time::Duration::from_secs(10)).await;
        match out {
            Ok(o) => o.stdout.trim().to_string(),
            Err(_) => "Unknown".to_string(),
        }
    } else {
        "Unknown".to_string()
    };

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let total_memory_gb = if cfg!(windows) {
        let out = proc::run("powershell", &["-NoProfile", "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB"], std::time::Duration::from_secs(10)).await;
        match out {
            Ok(o) => o.stdout.trim().parse::<f64>().unwrap_or(0.0),
            Err(_) => 0.0,
        }
    } else {
        0.0
    };

    let gpu = if cfg!(windows) {
        let out = proc::run("powershell", &["-NoProfile", "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, AdapterCompatibility | ConvertTo-Json"], std::time::Duration::from_secs(10)).await;
        match out {
            Ok(o) => {
                let gpus: Vec<Win32Gpu> = serde_json::from_str(&o.stdout).unwrap_or_default();
                gpus.into_iter().map(|g| GpuInfoDto {
                    name: g.name.unwrap_or_default(),
                    driver_version: g.driver_version.unwrap_or_default(),
                    vendor: g.adapter_compatibility.unwrap_or_default(),
                }).collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let disks = if cfg!(windows) {
        let out = proc::run("powershell", &["-NoProfile", "-Command",
            "Get-CimInstance Win32_LogicalDisk | Select-Object DeviceID, Size, FileSystem | ConvertTo-Json"], std::time::Duration::from_secs(10)).await;
        match out {
            Ok(o) => {
                let disks: Vec<Win32Disk> = serde_json::from_str(&o.stdout).unwrap_or_default();
                disks.into_iter().map(|d| DiskInfoDto {
                    name: d.device_id.unwrap_or_default(),
                    size_gb: d.size.map(|s| s / 1_073_741_824.0).unwrap_or(0.0),
                    filesystem: d.filesystem.unwrap_or_default(),
                }).collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Ok(HardwareInfoDto {
        cpu,
        cpu_cores,
        total_memory_gb,
        os,
        gpu,
        disks,
    })
}

#[derive(serde::Deserialize)]
struct Win32Gpu {
    name: Option<String>,
    driver_version: Option<String>,
    adapter_compatibility: Option<String>,
}

#[derive(serde::Deserialize)]
struct Win32Disk {
    device_id: Option<String>,
    size: Option<f64>,
    filesystem: Option<String>,
}

// ── Installed Packages ───────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct InstalledPackageDto {
    pub backend: String,
    pub id: String,
    pub name: String,
    pub version: String,
}

#[tauri::command]
pub async fn list_installed_packages(state: State<'_, AppState>) -> Result<Vec<InstalledPackageDto>, String> {
    let config = state.config.lock().unwrap().clone();
    let backends = odysync_backends::detect_backends(&config).await;
    let mut packages = Vec::new();

    for backend in &backends {
        let candidates = backend.scan().await;
        if let Ok(candidates) = candidates {
            for c in candidates {
                packages.push(InstalledPackageDto {
                    backend: backend.kind().id().to_string(),
                    id: c.id.to_string(),
                    name: c.name,
                    version: c.installed.raw().to_string(),
                });
            }
        }
    }

    Ok(packages)
}

// ── Logs ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct LogEntryDto {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

#[tauri::command]
pub async fn get_logs() -> Result<Vec<LogEntryDto>, String> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| "could not resolve data directory".to_string())?;
    let log_path = dirs.data_dir().join("logs/odysync.log");
    if !log_path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&log_path).map_err(|e| e.to_string())?;
    let all_lines: Vec<&str> = content.lines().collect();
    let entries = all_lines.iter().rev().take(200).rev().map(|line| {
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() >= 4 {
            LogEntryDto {
                timestamp: parts[0].to_string(),
                level: parts[1].to_string(),
                message: parts[3].to_string(),
            }
        } else {
            LogEntryDto {
                timestamp: String::new(),
                level: "INFO".to_string(),
                message: line.to_string(),
            }
        }
    }).collect();
    Ok(entries)
}

// ── Profile Manager ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct ProfileDto {
    pub name: String,
    pub packages: Vec<String>,
}

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<ProfileDto>, String> {
    let config = state.config.lock().unwrap();
    Ok(config.profiles.iter().map(|p| ProfileDto {
        name: p.name.clone(),
        packages: p.packages.clone(),
    }).collect())
}

#[tauri::command]
pub async fn create_profile(
    name: String,
    packages: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().unwrap().clone();
    if config.profiles.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("profile '{}' already exists", name));
    }
    config.profiles.push(odysync_core::config::Profile { name, packages });
    config.save(&state.config_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config.lock().unwrap().clone();
    config.profiles.retain(|p| !p.name.eq_ignore_ascii_case(&name));
    config.save(&state.config_path).map_err(|e| e.to_string())
}

// ── Offline Cache Status ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct OfflineCacheStatusDto {
    pub entry_count: usize,
    pub cache_size_bytes: u64,
}

#[tauri::command]
pub async fn get_offline_cache_status() -> Result<OfflineCacheStatusDto, String> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| "could not resolve data directory".to_string())?;
    let cache_dir = dirs.cache_dir().join("version-cache");
    let cache_file = cache_dir.join("version_cache.json");

    let entry_count = if cache_file.exists() {
        let content = std::fs::read_to_string(&cache_file).map_err(|e| e.to_string())?;
        let cache: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        cache.get("entries")
            .and_then(|e| e.as_object())
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };

    let cache_size_bytes = if cache_dir.exists() {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    } else {
        0
    };

    Ok(OfflineCacheStatusDto {
        entry_count,
        cache_size_bytes,
    })
}

#[tauri::command]
pub async fn clear_offline_cache() -> Result<(), String> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| "could not resolve data directory".to_string())?;
    let cache_dir = dirs.cache_dir().join("version-cache");
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Offline Cache Manager ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct OfflineManifestEntryDto {
    pub package_id: String,
    pub backend: String,
    pub version: String,
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub cached_at: String,
}

#[tauri::command]
pub async fn list_offline_cache() -> Result<Vec<OfflineManifestEntryDto>, String> {
    let manifest = odysync_backends::offline::CacheManifest::load();
    Ok(manifest.entries.iter().map(|e| OfflineManifestEntryDto {
        package_id: e.package_id.clone(),
        backend: e.backend.clone(),
        version: e.version.clone(),
        filename: e.filename.clone(),
        sha256: e.sha256.clone(),
        size_bytes: e.size_bytes,
        cached_at: e.cached_at.clone(),
    }).collect())
}

#[tauri::command]
pub async fn clear_offline_manifest() -> Result<(), String> {
    let mut manifest = odysync_backends::offline::CacheManifest::load();
    manifest.clear().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_offline_entry(package_id: String, backend: String) -> Result<(), String> {
    let mut manifest = odysync_backends::offline::CacheManifest::load();
    manifest.remove(&package_id, &backend).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn download_offline_installer(
    url: String,
    package_id: String,
    backend: String,
    version: String,
    expected_sha256: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    let proxy = config.proxy_url.as_deref();
    odysync_backends::offline::download_and_cache(
        &url, &package_id, &backend, &version,
        expected_sha256.as_deref(), proxy,
    ).await.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn verify_offline_cache() -> Result<Vec<bool>, String> {
    let manifest = odysync_backends::offline::CacheManifest::load();
    let mut results = Vec::new();
    for entry in &manifest.entries {
        let ok = odysync_backends::offline::verify_cached_file(entry).await
            .unwrap_or(false);
        results.push(ok);
    }
    Ok(results)
}
