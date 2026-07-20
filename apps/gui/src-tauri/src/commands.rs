use crate::state::AppState;
use serde::{Deserialize, Serialize};
use sensei_core::config::Config;
use sensei_core::maintenance::MaintenanceKind;
use sensei_core::model::{BackendKind, PackageId, UpdateCandidate};
use sensei_core::report::RunReport;
use sensei_core::runner::Runner;
use tauri::{AppHandle, Emitter, State};

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

fn backend_kind_from_str(s: &str) -> BackendKind {
    match s {
        "winget" => BackendKind::Winget,
        "msstore" => BackendKind::MsStore,
        "windows_drivers" => BackendKind::WindowsDrivers,
        "homebrew" => BackendKind::Homebrew,
        "apt" => BackendKind::Apt,
        "flatpak" => BackendKind::Flatpak,
        _ => BackendKind::Winget,
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan(state: State<'_, AppState>) -> Result<ScanResult, String> {
    let config = state.config.lock().unwrap().clone();
    let backends = sensei_backends::detect_backends(&config).await;

    let mut actionable = Vec::new();
    let mut skipped = Vec::new();

    for backend in &backends {
        let candidates = backend.scan().await.map_err(|e| e.to_string())?;
        let plan = config.policy.plan(candidates);

        for entry in &plan {
            match &entry.blocked_by {
                Some(reason) => {
                    skipped.push(SkippedDto {
                        backend: format!("{:?}", backend.kind()),
                        id: entry.candidate.id.to_string(),
                        name: entry.candidate.name.clone(),
                        reason: format!("{reason:?}"),
                    });
                }
                None => {
                    actionable.push(UpdateDto {
                        backend: format!("{:?}", backend.kind()),
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
    request: ApplyRequest,
    state: State<'_, AppState>,
) -> Result<ApplyResultDto, String> {
    let mut config = state.config.lock().unwrap().clone();
    config.policy.elevated = sensei_core::platform::is_elevated();
    let backends = sensei_backends::detect_backends(&config).await;

    // Rebuild UpdateCandidates from the DTOs by re-scanning and matching.
    let mut candidates_to_apply: Vec<UpdateCandidate> = Vec::new();
    for backend in &backends {
        let candidates = backend.scan().await.map_err(|e| e.to_string())?;
        for req_update in &request.updates {
            let req_kind = backend_kind_from_str(&req_update.backend);
            if backend.kind() != req_kind {
                continue;
            }
            for c in &candidates {
                if c.id.to_string() == req_update.id {
                    candidates_to_apply.push(c.clone());
                }
            }
        }
    }

    let plan = config.policy.plan(candidates_to_apply);
    let runner = Runner::new(backends.iter().map(|b| b.as_ref()), request.dry_run);
    let mut report = RunReport::new();
    runner.run(&plan, &mut report, request.restore_point).await;
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
    let backends = sensei_backends::detect_backends(&config).await;

    let mut result = Vec::new();
    for backend in &backends {
        result.push(BackendDto {
            kind: format!("{:?}", backend.kind()),
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
    let kind = backend_kind_from_str(&request.backend);
    let id = PackageId::new(kind, request.id);
    config.policy.holds.retain(|h| h.package != id.to_string());
    config.policy.holds.push(sensei_core::policy::Hold {
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
    let kind = backend_kind_from_str(&request.backend);
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

    let result = sensei_backends::maintenance::run_maintenance(kind)
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
    use sensei_backends::scheduler::{create_schedule, ScheduleFrequency, ScheduleSpec};

    let freq = match request.frequency.as_str() {
        "daily" => ScheduleFrequency::Daily,
        "weekly" => ScheduleFrequency::Weekly,
        _ => return Err(format!("unknown frequency: {}", request.frequency)),
    };

    let task_name = request.task_name.unwrap_or_else(|| {
        sensei_backends::scheduler::DEFAULT_TASK_NAME.to_string()
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
    let existed = sensei_backends::scheduler::schedule_exists(&task_name).await;
    if existed {
        sensei_backends::scheduler::remove_schedule(&task_name)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(existed)
}

#[tauri::command]
pub async fn check_schedule(task_name: String) -> Result<bool, String> {
    Ok(sensei_backends::scheduler::schedule_exists(&task_name).await)
}

#[tauri::command]
pub async fn create_diagnostics(
    out_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    let path = std::path::PathBuf::from(out_path);
    sensei_backends::diagnostics::create_diagnostics(&path, &config, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfoDto, String> {
    Ok(SystemInfoDto {
        os: sensei_core::platform::os_label().to_string(),
        elevated: sensei_core::platform::is_elevated(),
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
