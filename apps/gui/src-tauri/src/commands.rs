use crate::state::AppState;
use serde::{Deserialize, Serialize};
use odysync_core::config::Config;
use odysync_core::maintenance::MaintenanceKind;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
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
