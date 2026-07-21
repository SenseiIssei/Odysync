use crate::state::AppState;
use odysync_backends::security;
use odysync_core::config::Config;
use odysync_core::history::HistoryOutcome;
use odysync_core::maintenance::MaintenanceKind;
use odysync_core::model::{ApplyOutcome, BackendKind, PackageId, UpdateCandidate};
// Every `proc::powershell` call site is inside a `cfg(windows)` block, so on
// other platforms this import is unused — which `-D warnings` rejects.
#[cfg(windows)]
use odysync_core::proc;
use odysync_core::report::RunReport;
use odysync_core::runner::{ProgressEmitter, ProgressEvent, Runner};
use serde::{Deserialize, Serialize};
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

// ── Diagnostics ──────────────────────────────────────────────────────────────

/// Record a frontend crash in the application log.
///
/// A webview has no console the user can reach in a release build, so a React
/// render error would otherwise be invisible: the error boundary shows a
/// message on screen and nothing survives to diagnose afterwards.
#[tauri::command]
pub async fn report_frontend_error(
    context: String,
    message: String,
    stack: Option<String>,
) -> Result<(), String> {
    tracing::error!(
        context = %context,
        message = %message,
        stack = %stack.as_deref().unwrap_or("(none)"),
        "frontend error"
    );
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Quote `s` as a PowerShell single-quoted literal.
///
/// Values reaching these scripts come from the registry and the Startup folder,
/// which any user-level process can write. Interpolating them raw let a name
/// containing an apostrophe close the literal and run arbitrary PowerShell.
/// Called only from the Windows PowerShell paths (and its own test), so on
/// other platforms a non-test build sees it as dead code.
#[cfg_attr(not(windows), allow(dead_code))]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A stable machine-readable status plus human detail for an apply outcome.
///
/// The frontend used to receive `format!("{:?}", outcome)` and substring-match
/// on it, which broke as soon as a detail string contained the word it looked
/// for.
fn outcome_parts(outcome: &ApplyOutcome) -> (&'static str, String) {
    match outcome {
        ApplyOutcome::Updated { from, to } => ("updated", format!("{from} -> {to}")),
        ApplyOutcome::DidNotConverge { expected, actual } => (
            "did-not-converge",
            format!("expected {expected}, found {actual}"),
        ),
        ApplyOutcome::VerificationFailed { detail } => ("verification-failed", detail.clone()),
        ApplyOutcome::Failed { detail } => ("failed", detail.clone()),
        ApplyOutcome::Skipped { reason } => ("skipped", reason.to_string()),
    }
}

// ── DTOs for the frontend ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ScanResult {
    pub actionable: Vec<UpdateDto>,
    pub skipped: Vec<SkippedDto>,
    pub total: usize,
    /// Backends whose scan returned an error, so a partial result is not
    /// mistaken for "nothing to update".
    pub failed_backends: Vec<BackendErrorDto>,
}

#[derive(Serialize)]
pub struct BackendErrorDto {
    pub backend: String,
    pub error: String,
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
    /// Present and usable on this machine.
    pub available: bool,
    /// Not switched off by the user in config.
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct SystemInfoDto {
    pub os: String,
    pub elevated: bool,
    pub version: String,
    /// Non-null when the config file on disk could not be parsed at startup.
    pub config_error: Option<String>,
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
    /// One of `updated`, `did-not-converge`, `verification-failed`,
    /// `failed`, `skipped`.
    pub status: String,
    pub detail: String,
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

// ── Config DTO ───────────────────────────────────────────────────────────────
//
// `Config` serializes as kebab-case and has `#[serde(default)]` on the
// container with no `deny_unknown_fields`. Handing it straight to the frontend
// meant every multi-word field arrived as `undefined`, and posting a snake_case
// object back was silently accepted as "all fields absent" — which reset the
// entire config to defaults and wiped the user's holds and exclusions.
//
// So the wire format is an explicit DTO, and saving *patches* the current
// config rather than replacing it. Anything the UI does not own is preserved by
// construction.

#[derive(Serialize, Deserialize)]
pub struct HoldDto {
    pub package: String,
    pub pin: Option<String>,
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct PolicyDto {
    pub stable_only: bool,
    pub require_known_versions: bool,
    pub exclude: Vec<String>,
    /// Shown in the UI but never written back — holds are managed through the
    /// `hold`/`unhold` commands, and a stale copy must not be able to drop one.
    #[serde(skip_deserializing)]
    pub holds: Vec<HoldDto>,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileDto {
    pub name: String,
    pub packages: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigDto {
    pub policy: PolicyDto,
    pub disabled_backends: Vec<String>,
    /// Read-only here; use the profile commands to change them.
    #[serde(skip_deserializing)]
    pub profiles: Vec<ProfileDto>,
    pub restore_point: bool,
    pub scan_interval_hours: u32,
    pub concurrency: u32,
    pub proxy_url: Option<String>,
    pub auto_apply: bool,
    pub notifications: bool,
    pub max_retries: u32,
    pub backend_timeout_secs: u32,
}

impl From<&Config> for ConfigDto {
    fn from(c: &Config) -> Self {
        Self {
            policy: PolicyDto {
                stable_only: c.policy.stable_only,
                require_known_versions: c.policy.require_known_versions,
                exclude: c.policy.exclude.clone(),
                holds: c
                    .policy
                    .holds
                    .iter()
                    .map(|h| HoldDto {
                        package: h.package.clone(),
                        pin: h.pin.clone(),
                        note: h.note.clone(),
                    })
                    .collect(),
            },
            disabled_backends: c.disabled_backends.clone(),
            profiles: c
                .profiles
                .iter()
                .map(|p| ProfileDto {
                    name: p.name.clone(),
                    packages: p.packages.clone(),
                })
                .collect(),
            restore_point: c.restore_point,
            scan_interval_hours: c.scan_interval_hours,
            concurrency: c.concurrency,
            proxy_url: c.proxy_url.clone(),
            auto_apply: c.auto_apply,
            notifications: c.notifications,
            max_retries: c.max_retries,
            backend_timeout_secs: c.backend_timeout_secs,
        }
    }
}

impl ConfigDto {
    /// Apply the fields the UI owns onto `config`, leaving everything else —
    /// holds, profiles, and the runtime-detected `elevated` flag — alone.
    fn apply_to(self, config: &mut Config) {
        config.policy.stable_only = self.policy.stable_only;
        config.policy.require_known_versions = self.policy.require_known_versions;
        config.policy.exclude = self
            .policy
            .exclude
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        config.disabled_backends = self.disabled_backends;
        config.restore_point = self.restore_point;
        config.scan_interval_hours = self.scan_interval_hours.min(24 * 7);
        config.concurrency = self.concurrency.clamp(1, 16);
        config.proxy_url = self.proxy_url.filter(|s| !s.trim().is_empty());
        config.auto_apply = self.auto_apply;
        config.notifications = self.notifications;
        config.max_retries = self.max_retries.min(10);
        config.backend_timeout_secs = self.backend_timeout_secs.clamp(10, 3600);
        // `skip_prerelease` is documented as an alias for `policy.stable_only`;
        // keep them from drifting apart rather than exposing both.
        config.skip_prerelease = config.policy.stable_only;
    }
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
    let config = state.config();
    let probed = state.probed_backends().await;
    let usable: Vec<_> = probed.iter().filter(|p| p.usable()).collect();
    tracing::info!(backends = usable.len(), "starting scan");

    let results = futures::future::join_all(usable.iter().map(|p| async {
        let kind_id = p.backend.kind().id().to_string();
        (kind_id, p.backend.scan().await)
    }))
    .await;

    let mut actionable = Vec::new();
    let mut skipped = Vec::new();
    let mut failed_backends = Vec::new();
    let mut cache = crate::state::ScanCache::new();

    for (kind_id, result) in results {
        let candidates = match result {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(backend = %kind_id, error = %e, "scan failed");
                failed_backends.push(BackendErrorDto {
                    backend: kind_id,
                    error: e.sanitize(),
                });
                continue;
            }
        };

        // Cache candidates so `apply` can act without re-scanning.
        cache.insert(kind_id.clone(), candidates.clone());

        for entry in config.policy.plan(candidates) {
            match &entry.blocked_by {
                Some(reason) => skipped.push(SkippedDto {
                    backend: kind_id.clone(),
                    id: entry.candidate.id.to_string(),
                    name: entry.candidate.name.clone(),
                    reason: reason.to_string(),
                }),
                None => actionable.push(UpdateDto {
                    backend: kind_id.clone(),
                    id: entry.candidate.id.to_string(),
                    name: entry.candidate.name.clone(),
                    installed: entry.candidate.installed.raw().to_string(),
                    available: entry.candidate.available.raw().to_string(),
                    size_bytes: entry.candidate.size_bytes,
                }),
            }
        }
    }

    state.replace_scan_cache(cache);

    let total = actionable.len() + skipped.len();
    tracing::info!(
        total,
        actionable = actionable.len(),
        skipped = skipped.len(),
        failed = failed_backends.len(),
        "scan complete"
    );
    Ok(ScanResult {
        actionable,
        skipped,
        total,
        failed_backends,
    })
}

#[tauri::command]
pub async fn apply(
    app: AppHandle,
    request: ApplyRequest,
    state: State<'_, AppState>,
) -> Result<ApplyResultDto, String> {
    tracing::info!(
        count = request.updates.len(),
        dry_run = request.dry_run,
        "starting apply"
    );

    let mut config = state.config();
    config.policy.elevated = odysync_core::platform::is_elevated();

    // Act on cached scan results so the versions applied are exactly the ones
    // the user saw and approved.
    let cache = state.scan_cache();

    let mut candidates_to_apply: Vec<UpdateCandidate> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for req in &request.updates {
        let found = cache
            .get(&req.backend)
            .map(|candidates| {
                let before = candidates_to_apply.len();
                for c in candidates {
                    if c.id.to_string() == req.id {
                        candidates_to_apply.push(c.clone());
                    }
                }
                candidates_to_apply.len() > before
            })
            .unwrap_or(false);
        if !found {
            missing.push(req.name.as_str());
        }
    }

    if candidates_to_apply.is_empty() {
        return Err(
            "None of the selected updates are in the current scan results. \
             Scan again and retry."
                .into(),
        );
    }
    if !missing.is_empty() {
        tracing::warn!(
            missing = missing.len(),
            "some selected updates were not in the scan cache and were skipped"
        );
    }

    let probed = state.probed_backends().await;
    let backends: Vec<&dyn odysync_core::backend::Backend> = probed
        .iter()
        .filter(|p| p.usable())
        .map(|p| p.backend.as_ref())
        .collect();

    let plan = config.policy.plan(candidates_to_apply);

    let mut runner = Runner::new(backends, request.dry_run).with_max_retries(config.max_retries);
    if !request.dry_run {
        // Record what happened so the History page has something to show.
        runner = runner.with_history(odysync_core::history::UpdateHistory::load());
    }

    let emitter = TauriProgressEmitter { app: app.clone() };
    let mut report = RunReport::new();
    runner
        .run_with_progress(
            &plan,
            &mut report,
            request.restore_point || config.restore_point,
            Some(&emitter),
        )
        .await;

    let entries: Vec<ApplyEntryDto> = report
        .entries
        .iter()
        .map(|e| {
            let (status, detail) = outcome_parts(&e.outcome);
            ApplyEntryDto {
                name: e.name.clone(),
                status: status.to_string(),
                detail,
            }
        })
        .collect();

    let result = ApplyResultDto {
        updated: report.updated(),
        failed: report.failed(),
        skipped: report.skipped(),
        reboot_required: report.reboot_required,
        exit_code: report.exit_code(),
        entries,
    };

    // Anything applied invalidates the scan; let every page know.
    if !request.dry_run {
        state.replace_scan_cache(crate::state::ScanCache::new());
        let _ = app.emit("apply-finished", ());
    }

    Ok(result)
}

#[tauri::command]
pub async fn list_backends(state: State<'_, AppState>) -> Result<Vec<BackendDto>, String> {
    let probed = state.probed_backends().await;
    Ok(probed
        .iter()
        .map(|p| BackendDto {
            kind: p.backend.kind().id().to_string(),
            name: p.backend.display_name().to_string(),
            available: p.available,
            enabled: p.enabled,
        })
        .collect())
}

/// Force a fresh availability probe (the results are otherwise cached).
#[tauri::command]
pub async fn refresh_backends(state: State<'_, AppState>) -> Result<Vec<BackendDto>, String> {
    state.invalidate_backends().await;
    list_backends(state).await
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<ConfigDto, String> {
    Ok(ConfigDto::from(&state.config()))
}

#[tauri::command]
pub async fn save_config(config: ConfigDto, state: State<'_, AppState>) -> Result<(), String> {
    let mut current = state.config();
    let disabled_before = current.disabled_backends.clone();
    config.apply_to(&mut current);
    let disabled_changed = current.disabled_backends != disabled_before;

    state.save_config(current).map_err(|e| e.to_string())?;

    if disabled_changed {
        state.invalidate_backends().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn hold(request: HoldRequest, state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config();
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
    state.save_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unhold(request: HoldRequest, state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config();
    let Some(kind) = backend_kind_from_str(&request.backend) else {
        return Err(format!("unknown backend: {}", request.backend));
    };
    let id = PackageId::new(kind, request.id);
    config.policy.holds.retain(|h| h.package != id.to_string());
    state.save_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_maintenance(action: String) -> Result<String, String> {
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
pub async fn create_schedule(request: ScheduleRequest) -> Result<String, String> {
    use odysync_backends::scheduler::{create_schedule, ScheduleFrequency, ScheduleSpec};

    let freq = match request.frequency.as_str() {
        "daily" => ScheduleFrequency::Daily,
        "weekly" => ScheduleFrequency::Weekly,
        _ => return Err(format!("unknown frequency: {}", request.frequency)),
    };

    let task_name = request
        .task_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| odysync_backends::scheduler::DEFAULT_TASK_NAME.to_string());

    let spec = ScheduleSpec {
        frequency: freq,
        time: request.time,
        task_name: task_name.clone(),
        extra_args: Vec::new(),
    };

    create_schedule(&spec).await.map_err(|e| e.to_string())?;
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
    let config = state.config();
    let path = std::path::PathBuf::from(out_path);
    odysync_backends::diagnostics::create_diagnostics(&path, &config, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_system_info(state: State<'_, AppState>) -> Result<SystemInfoDto, String> {
    Ok(SystemInfoDto {
        os: odysync_core::platform::os_label().to_string(),
        elevated: odysync_core::platform::is_elevated(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        config_error: state.config_load_error.clone(),
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
            serde_json::json!({ "count": result.actionable.len() }),
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
    pub status: String,
    pub detail: String,
}

#[tauri::command]
pub async fn get_update_history() -> Result<Vec<HistoryEntryDto>, String> {
    let history = odysync_core::history::UpdateHistory::load();
    Ok(history
        .entries()
        .iter()
        .rev()
        .map(|e| HistoryEntryDto {
            timestamp: e.timestamp.to_rfc3339(),
            package: e.package_name.clone(),
            backend: e.backend.id().to_string(),
            from_version: e.from_version.clone(),
            to_version: e.to_version.clone(),
            // History stores a simplified outcome; map it onto the same
            // discriminants the apply path emits so the UI has one vocabulary.
            status: match e.outcome {
                HistoryOutcome::Updated => "updated",
                HistoryOutcome::Failed => "failed",
                HistoryOutcome::Skipped => "skipped",
                HistoryOutcome::DidNotConverge => "did-not-converge",
            }
            .to_string(),
            detail: format!("{} -> {}", e.from_version, e.to_version),
        })
        .collect())
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

/// Parse a `ConvertTo-Json` payload that may be a single object or an array.
///
/// PowerShell emits a bare object when a query returns exactly one row, which
/// is why every call site here needs both shapes.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_ps_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Vec<T> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Vec::new();
    }
    if stdout.starts_with('[') {
        serde_json::from_str::<Vec<T>>(stdout).unwrap_or_default()
    } else {
        serde_json::from_str::<T>(stdout)
            .map(|v| vec![v])
            .unwrap_or_default()
    }
}

#[tauri::command]
pub async fn get_hardware_info() -> Result<HardwareInfoDto, String> {
    tracing::info!("fetching hardware info");
    let os = odysync_core::platform::os_label().to_string();

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    // No `return`: on non-Windows the block below is compiled out, so this is
    // the tail expression and an explicit return is a clippy error under
    // `-D warnings`.
    #[cfg(not(windows))]
    {
        Ok(HardwareInfoDto {
            cpu: "Unknown".to_string(),
            cpu_cores,
            total_memory_gb: 0.0,
            os,
            gpu: Vec::new(),
            disks: Vec::new(),
        })
    }

    #[cfg(windows)]
    {
        let timeout = std::time::Duration::from_secs(20);

        // One PowerShell process for everything: four separate spawns took
        // several seconds and made the Hardware page the slowest in the app.
        let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$cpu  = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$mem  = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$gpu  = @(Get-CimInstance Win32_VideoController |
          Select-Object Name, DriverVersion, AdapterCompatibility)
$disk = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' |
          Select-Object DeviceID, VolumeName, Size, FileSystem)
[PSCustomObject]@{
    Cpu     = $cpu
    MemoryBytes = [string]$mem
    Gpu     = $gpu
    Disks   = $disk
} | ConvertTo-Json -Depth 4 -Compress
"#;

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct HardwareJson {
            cpu: Option<String>,
            /// Serialized as a string: TotalPhysicalMemory exceeds the range
            /// PowerShell's JSON writer represents exactly as a number.
            memory_bytes: Option<String>,
            #[serde(default)]
            gpu: Vec<Win32Gpu>,
            #[serde(default)]
            disks: Vec<Win32Disk>,
        }

        let raw = match proc::powershell(script, timeout).await {
            Ok(o) => o.stdout,
            Err(e) => {
                tracing::warn!(error = %e, "hardware query failed");
                return Ok(HardwareInfoDto {
                    cpu: "Unknown".to_string(),
                    cpu_cores,
                    total_memory_gb: 0.0,
                    os,
                    gpu: Vec::new(),
                    disks: Vec::new(),
                });
            }
        };

        let parsed: HardwareJson = serde_json::from_str(raw.trim()).unwrap_or(HardwareJson {
            cpu: None,
            memory_bytes: None,
            gpu: Vec::new(),
            disks: Vec::new(),
        });

        let total_memory_gb = parsed
            .memory_bytes
            .as_deref()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|b| (b / 1_073_741_824.0 * 100.0).round() / 100.0)
            .unwrap_or(0.0);

        let gpu = parsed
            .gpu
            .into_iter()
            .map(|g| GpuInfoDto {
                name: g.name.unwrap_or_default(),
                driver_version: g.driver_version.unwrap_or_default(),
                vendor: g.adapter_compatibility.unwrap_or_default(),
            })
            .filter(|g| !g.name.is_empty())
            .collect();

        let disks = parsed
            .disks
            .into_iter()
            .map(|d| DiskInfoDto {
                name: format!(
                    "{} {}",
                    d.device_id.unwrap_or_default(),
                    d.volume_name.unwrap_or_default().trim()
                )
                .trim()
                .to_string(),
                size_gb: d
                    .size
                    .and_then(|s| s.as_f64())
                    .map(|s| (s / 1_073_741_824.0 * 100.0).round() / 100.0)
                    .unwrap_or(0.0),
                filesystem: d.filesystem.unwrap_or_default(),
            })
            .collect();

        Ok(HardwareInfoDto {
            cpu: parsed
                .cpu
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "Unknown".to_string()),
            cpu_cores,
            total_memory_gb,
            os,
            gpu,
            disks,
        })
    }
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32Gpu {
    name: Option<String>,
    driver_version: Option<String>,
    adapter_compatibility: Option<String>,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32Disk {
    device_id: Option<String>,
    volume_name: Option<String>,
    /// Number or string depending on magnitude, so kept as a raw JSON value.
    size: Option<serde_json::Value>,
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
pub async fn list_installed_packages(
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPackageDto>, String> {
    let probed = state.probed_backends().await;
    let usable: Vec<_> = probed.iter().filter(|p| p.usable()).collect();

    // Concurrent, like the scan path — these are independent process spawns and
    // running them in sequence made the Packages page take minutes.
    let results = futures::future::join_all(usable.iter().map(|p| async {
        let kind_id = p.backend.kind().id().to_string();
        (kind_id, p.backend.list_installed().await)
    }))
    .await;

    let mut packages = Vec::new();
    for (kind_id, result) in results {
        match result {
            Ok(list) => packages.extend(list.into_iter().map(|p| InstalledPackageDto {
                backend: kind_id.clone(),
                id: p.id.to_string(),
                name: p.name,
                version: p.version,
            })),
            Err(e) => tracing::warn!(backend = %kind_id, error = %e, "listing installed failed"),
        }
    }

    packages.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.backend.cmp(&b.backend))
    });
    Ok(packages)
}

// ── Logs ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct LogEntryDto {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

const LOG_TAIL_LINES: usize = 500;

fn log_path() -> Result<std::path::PathBuf, String> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| "could not resolve data directory".to_string())?;
    Ok(dirs.data_dir().join("logs/odysync.log"))
}

/// Split one `tracing` fmt line into timestamp / level / message.
///
/// The default layout is `<rfc3339>  <LEVEL> <target>: <message>`, with the
/// level right-padded to five characters — so splitting on single spaces put
/// the padding in the level field and the target in the message.
fn parse_log_line(line: &str) -> LogEntryDto {
    const LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

    let mut parts = line.split_whitespace();
    let timestamp = parts.next().unwrap_or_default();
    let level = parts.next().unwrap_or_default();

    if !LEVELS.contains(&level) {
        return LogEntryDto {
            timestamp: String::new(),
            level: "INFO".to_string(),
            message: line.trim().to_string(),
        };
    }

    // Everything after the level, minus the `target:` prefix if present.
    let rest = line
        .split_once(level)
        .map(|(_, r)| r.trim_start())
        .unwrap_or("");
    let message = match rest.split_once(": ") {
        Some((target, msg)) if !target.contains(' ') => msg,
        _ => rest,
    };

    LogEntryDto {
        timestamp: timestamp.to_string(),
        level: level.to_string(),
        message: message.trim().to_string(),
    }
}

#[tauri::command]
pub async fn get_logs() -> Result<Vec<LogEntryDto>, String> {
    let path = log_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    // Off the async runtime: the log file grows without bound and reading it
    // synchronously stalled every other command.
    let content = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(LOG_TAIL_LINES);
    Ok(lines[start..].iter().map(|l| parse_log_line(l)).collect())
}

#[tauri::command]
pub async fn open_log_folder() -> Result<String, String> {
    let path = log_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| "log directory has no parent".to_string())?
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

// ── Profile Manager ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<ProfileDto>, String> {
    Ok(state
        .config()
        .profiles
        .iter()
        .map(|p| ProfileDto {
            name: p.name.clone(),
            packages: p.packages.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn create_profile(
    name: String,
    packages: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("profile name cannot be empty".into());
    }

    let mut config = state.config();
    if config
        .profiles
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("profile '{name}' already exists"));
    }
    config.profiles.push(odysync_core::config::Profile {
        name,
        packages: packages
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    });
    // Persist *and* adopt: writing to disk alone left `list_profiles` reading a
    // stale in-memory config, so a new profile did not appear until restart.
    state.save_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config();
    let before = config.profiles.len();
    config
        .profiles
        .retain(|p| !p.name.eq_ignore_ascii_case(&name));
    if config.profiles.len() == before {
        return Err(format!("no profile named '{name}'"));
    }
    state.save_config(config).map_err(|e| e.to_string())
}

// ── Offline Cache ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct OfflineCacheStatusDto {
    pub entry_count: usize,
    pub cache_size_bytes: u64,
}

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

/// Status of the offline *installer* cache.
///
/// This used to read `version-cache/version_cache.json`, which belongs to
/// `odysync-version-source` and holds version metadata, not installers — so the
/// count never matched the list rendered directly underneath it.
#[tauri::command]
pub async fn get_offline_cache_status() -> Result<OfflineCacheStatusDto, String> {
    let manifest = odysync_backends::offline::CacheManifest::load_async().await;
    Ok(OfflineCacheStatusDto {
        entry_count: manifest.entries.len(),
        // Real bytes on disk rather than the sizes the manifest claims, so a
        // partial download or an externally deleted file shows up honestly.
        cache_size_bytes: odysync_backends::offline::total_size_bytes(),
    })
}

/// Drop manifest entries whose cached file is gone, returning how many.
#[tauri::command]
pub async fn prune_offline_cache() -> Result<usize, String> {
    let dropped = odysync_backends::offline::prune_missing()
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(dropped, "pruned offline cache entries with no file on disk");
    Ok(dropped)
}

#[tauri::command]
pub async fn list_offline_cache() -> Result<Vec<OfflineManifestEntryDto>, String> {
    let manifest = odysync_backends::offline::CacheManifest::load();
    Ok(manifest
        .entries
        .iter()
        .map(|e| OfflineManifestEntryDto {
            package_id: e.package_id.clone(),
            backend: e.backend.clone(),
            version: e.version.clone(),
            filename: e.filename.clone(),
            sha256: e.sha256.clone(),
            size_bytes: e.size_bytes,
            cached_at: e.cached_at.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn clear_offline_manifest() -> Result<(), String> {
    let mut manifest = odysync_backends::offline::CacheManifest::load();
    manifest.clear().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_offline_entry(package_id: String, backend: String) -> Result<(), String> {
    let mut manifest = odysync_backends::offline::CacheManifest::load();
    manifest
        .remove(&package_id, &backend)
        .map_err(|e| e.to_string())
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
    let config = state.config();
    let proxy = config.proxy_url.clone();
    odysync_backends::offline::download_and_cache(
        &url,
        &package_id,
        &backend,
        &version,
        expected_sha256.as_deref(),
        proxy.as_deref(),
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn verify_offline_cache() -> Result<Vec<bool>, String> {
    let manifest = odysync_backends::offline::CacheManifest::load();
    let mut results = Vec::with_capacity(manifest.entries.len());
    for entry in &manifest.entries {
        results.push(
            odysync_backends::offline::verify_cached_file(entry)
                .await
                .unwrap_or(false),
        );
    }
    Ok(results)
}

// ── App lifecycle ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn quit_app(app: AppHandle) -> Result<(), String> {
    tracing::info!("user requested quit, exiting application");
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub async fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        if odysync_core::platform::is_elevated() {
            return Ok(());
        }

        let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_path = current_exe.to_string_lossy().to_string();

        let wide = |s: &str| -> Vec<u16> {
            OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let verb = wide("runas");
        let file = wide(&exe_path);

        // Declared here rather than pulled from the `windows` crate to keep the
        // GUI's dependency surface small; shell32 is linked explicitly so the
        // symbol resolves at link time on MSVC.
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show_cmd: i32,
            ) -> *mut std::ffi::c_void;
        }

        const SW_SHOWNORMAL: i32 = 1;
        const SE_ERR_CANCELLED: isize = 1223; // ERROR_CANCELLED — user said No at the UAC prompt

        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;

        // ShellExecuteW returns a value greater than 32 on success.
        if result <= 32 {
            return Err(if result == SE_ERR_CANCELLED {
                "Elevation was cancelled.".to_string()
            } else {
                format!("Failed to restart as administrator (code {result}).")
            });
        }

        // The elevated instance takes over from here.
        app.exit(0);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Admin restart is only available on Windows".to_string())
    }
}

// ── Security ─────────────────────────────────────────────────────────────────

/// Full posture and indicator audit.
///
/// Infallible by construction: a section that fails is recorded in the report's
/// `sections` rather than aborting the sweep, so a machine that blocks one
/// query still gets an answer for everything else — and the UI can say the
/// audit was incomplete instead of implying the machine is clean.
#[tauri::command]
pub async fn security_scan() -> Result<security::ScanReport, String> {
    tracing::info!("starting security audit");
    let report = security::scan().await;

    // Log the shape of the result, not just the total. A count on its own hides
    // severity inflation: "131 findings" reads as an emergency when 121 of them
    // are routine. The per-title tally makes a single over-eager rule obvious.
    let counts = report.counts();
    // Tally by the finding id's rule prefix ("persistence-task", "network-listener",
    // ...), not just the category. A category tells you *where* the noise is;
    // only the rule name tells you *which check* is over-firing, which is the
    // thing you actually need to fix.
    let mut by_title: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for f in &report.findings {
        if f.severity != security::Severity::Info {
            let rule = f.id.split(':').next().unwrap_or(f.id.as_str());
            *by_title.entry(rule).or_insert(0) += 1;
        }
    }
    tracing::info!(
        findings = report.findings.len(),
        critical = counts.get(&security::Severity::Critical).copied().unwrap_or(0),
        high = counts.get(&security::Severity::High).copied().unwrap_or(0),
        medium = counts.get(&security::Severity::Medium).copied().unwrap_or(0),
        low = counts.get(&security::Severity::Low).copied().unwrap_or(0),
        watching = counts.get(&security::Severity::Info).copied().unwrap_or(0),
        actionable_by_rule = ?by_title,
        incomplete = report.incomplete(),
        "security audit complete"
    );
    Ok(report)
}

#[derive(Serialize)]
pub struct DefenderStatusDto {
    pub real_time_protection: bool,
    pub tamper_protection: bool,
    pub antivirus_enabled: bool,
    pub signature_age_days: u32,
    pub signature_version: String,
    pub last_quick_scan: Option<String>,
    pub last_full_scan: Option<String>,
}

/// Render a Defender "age in days" value for humans.
///
/// Defender reports `4294967295` (`u32::MAX`) to mean "never happened", which
/// rendered as "4294967295 day(s) ago".
fn describe_scan_age(days: Option<i64>) -> Option<String> {
    match days? {
        d if d < 0 || d >= u32::MAX as i64 => Some("never".to_string()),
        0 => Some("today".to_string()),
        1 => Some("yesterday".to_string()),
        d => Some(format!("{d} days ago")),
    }
}

#[tauri::command]
pub async fn get_defender_status() -> Result<DefenderStatusDto, String> {
    let snapshot = security::defender::snapshot()
        .await
        .map_err(|e| e.to_string())?;
    let s = snapshot.status.unwrap_or_default();

    // Defender reports ages in days, and "unknown" as an absent field. Treat
    // absent as 0 rather than as stale so a machine we cannot read is not
    // painted red on a guess.
    Ok(DefenderStatusDto {
        real_time_protection: s.real_time_protection_enabled.unwrap_or(false),
        tamper_protection: s.is_tamper_protected.unwrap_or(false),
        antivirus_enabled: s.antivirus_enabled.unwrap_or(false),
        signature_age_days: s.antivirus_signature_age.unwrap_or(0).max(0) as u32,
        signature_version: s.antivirus_signature_version.unwrap_or_default(),
        last_quick_scan: describe_scan_age(s.quick_scan_age),
        last_full_scan: describe_scan_age(s.full_scan_age),
    })
}

#[tauri::command]
pub async fn defender_quick_scan() -> Result<String, String> {
    tracing::info!("starting Defender quick scan");
    security::defender::quick_scan()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn defender_full_scan() -> Result<String, String> {
    tracing::info!("starting Defender full scan (this can run for hours)");
    security::defender::full_scan()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_defender_signatures() -> Result<String, String> {
    security::defender::update_signatures()
        .await
        .map_err(|e| e.to_string())
}

/// Apply one remediation the user explicitly authorised.
///
/// The payload is a closed enum, not a command string, so every parameter is
/// validated in `security::remediate` before anything is touched.
#[tauri::command]
pub async fn apply_remediation(remediation: security::Remediation) -> Result<String, String> {
    tracing::warn!(?remediation, "applying user-authorised remediation");
    let outcome = security::remediate::apply(&remediation)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(outcome = %outcome, "remediation applied");
    Ok(outcome)
}

// ── Autostart (start Odysync with Windows) ───────────────────────────────────

#[derive(Serialize)]
pub struct AutostartDto {
    pub enabled: bool,
    pub minimized: bool,
}

#[tauri::command]
pub async fn get_autostart() -> Result<AutostartDto, String> {
    let cfg = odysync_backends::autostart::status()
        .await
        .map_err(|e| e.to_string())?;
    Ok(AutostartDto {
        enabled: cfg.enabled,
        minimized: cfg.minimized,
    })
}

#[tauri::command]
pub async fn enable_autostart(minimized: bool) -> Result<(), String> {
    tracing::info!(minimized, "enabling autostart");
    odysync_backends::autostart::enable(minimized)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disable_autostart() -> Result<(), String> {
    tracing::info!("disabling autostart");
    odysync_backends::autostart::disable()
        .await
        .map_err(|e| e.to_string())
}

// ── Startup Programs ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct StartupProgramDto {
    pub name: String,
    pub command: String,
    pub location: String,
    pub enabled: bool,
}

#[tauri::command]
pub async fn list_startup_programs() -> Result<Vec<StartupProgramDto>, String> {
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }

    #[cfg(windows)]
    {
        tracing::info!("listing startup programs");
        // The StartupApproved keys hold a 12-byte blob per entry whose first
        // byte encodes the state: even values are enabled, odd are disabled.
        // The previous version compared `$_.Name -eq $_.Name` (always true) and
        // rebound `$_` inside the loop, so every entry reported "enabled".
        let script = r#"
$ErrorActionPreference = 'SilentlyContinue'

function Get-ApprovalMap([string]$approvedPath) {
    $map = @{}
    if (Test-Path $approvedPath) {
        $key = Get-Item $approvedPath
        foreach ($valueName in $key.GetValueNames()) {
            $bytes = $key.GetValue($valueName)
            if ($bytes -is [byte[]] -and $bytes.Length -gt 0) {
                $map[$valueName] = (($bytes[0] % 2) -eq 0)
            }
        }
    }
    return $map
}

$result = New-Object System.Collections.ArrayList

$runKeys = @(
    @{ Path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
       Approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' },
    @{ Path = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Run'
       Approved = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' }
)

foreach ($entry in $runKeys) {
    if (-not (Test-Path $entry.Path)) { continue }
    $approved = Get-ApprovalMap $entry.Approved
    $key = Get-Item $entry.Path
    foreach ($valueName in $key.GetValueNames()) {
        if ([string]::IsNullOrEmpty($valueName)) { continue }
        $enabled = $true
        if ($approved.ContainsKey($valueName)) { $enabled = $approved[$valueName] }
        [void]$result.Add([PSCustomObject]@{
            Name     = $valueName
            Command  = [string]$key.GetValue($valueName)
            Location = $entry.Path
            Enabled  = $enabled
        })
    }
}

$startupFolders = @(
    [Environment]::GetFolderPath('Startup'),
    (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Startup')
)
foreach ($folder in $startupFolders) {
    if ([string]::IsNullOrEmpty($folder) -or -not (Test-Path $folder)) { continue }
    foreach ($file in Get-ChildItem -LiteralPath $folder -File) {
        # `.disabled` is the marker this app writes when turning an entry off.
        $isDisabled = $file.Name -like '*.disabled'
        $displayName = $file.BaseName -replace '\.disabled$', ''
        [void]$result.Add([PSCustomObject]@{
            Name     = $displayName
            Command  = $file.FullName
            Location = $folder
            Enabled  = (-not $isDisabled)
        })
    }
}

@($result | Sort-Object Name) | ConvertTo-Json -Depth 3 -Compress
"#;
        let out = proc::powershell(script, std::time::Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;

        let programs: Vec<StartupProgramJson> = parse_ps_json(&out.stdout);
        Ok(programs
            .into_iter()
            .map(|p| StartupProgramDto {
                name: p.name,
                command: p.command.unwrap_or_default(),
                location: p.location,
                enabled: p.enabled,
            })
            .collect())
    }
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StartupProgramJson {
    name: String,
    command: Option<String>,
    location: String,
    enabled: bool,
}

#[tauri::command]
pub async fn toggle_startup_program(
    name: String,
    location: String,
    enable: bool,
) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = (name, location, enable);
        Err("Startup program management is only available on Windows".to_string())
    }

    #[cfg(windows)]
    {
        // `$enable` used to be referenced but never defined, so every toggle
        // took the disable branch and an entry could never be switched back on.
        let script = format!(
            r#"
$ErrorActionPreference = 'Stop'
$name = {name}
$location = {location}
$enable = ${enable}

if ($location -like 'HKCU:*' -or $location -like 'HKLM:*') {{
    $hive = if ($location -like 'HKLM:*') {{ 'HKLM:' }} else {{ 'HKCU:' }}
    $approvedPath = "$hive\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
    if (-not (Test-Path $approvedPath)) {{
        New-Item -Path $approvedPath -Force | Out-Null
    }}
    # First byte even = enabled, odd = disabled. The remaining 11 bytes are a
    # timestamp Windows does not require us to fill in.
    $first = if ($enable) {{ 0x02 }} else {{ 0x03 }}
    $blob = [byte[]]($first,0,0,0,0,0,0,0,0,0,0,0)
    Set-ItemProperty -LiteralPath $approvedPath -Name $name -Value $blob -Type Binary
}} else {{
    $enabledPath  = Join-Path $location "$name.lnk"
    $disabledPath = Join-Path $location "$name.lnk.disabled"
    if ($enable) {{
        if (Test-Path -LiteralPath $disabledPath) {{
            Rename-Item -LiteralPath $disabledPath -NewName "$name.lnk" -Force
        }} elseif (-not (Test-Path -LiteralPath $enabledPath)) {{
            throw "No startup entry found for $name"
        }}
    }} else {{
        if (Test-Path -LiteralPath $enabledPath) {{
            Rename-Item -LiteralPath $enabledPath -NewName "$name.lnk.disabled" -Force
        }} elseif (-not (Test-Path -LiteralPath $disabledPath)) {{
            throw "No startup entry found for $name"
        }}
    }}
}}
Write-Output "ODYSYNC_OK"
"#,
            name = ps_quote(&name),
            location = ps_quote(&location),
            enable = if enable { "true" } else { "false" },
        );

        let out = proc::powershell(&script, std::time::Duration::from_secs(20))
            .await
            .map_err(|e| e.to_string())?;

        if out.stdout.contains("ODYSYNC_OK") {
            Ok(())
        } else {
            let detail = out.stderr.trim();
            Err(if detail.is_empty() {
                "Failed to change the startup entry.".to_string()
            } else {
                format!("Failed to change the startup entry: {detail}")
            })
        }
    }
}

// ── Backup / Restore Points ──────────────────────────────────────────────────

#[derive(Serialize)]
pub struct BackupDto {
    pub name: String,
    pub created_at: String,
    /// The restore point's sequence number — what `Restore-Computer` needs.
    pub sequence_number: i64,
    pub backup_type: String,
}

#[tauri::command]
pub async fn list_backups() -> Result<Vec<BackupDto>, String> {
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }

    #[cfg(windows)]
    {
        tracing::info!("listing restore points");
        // `Get-ComputerRestorePoint` fails with "access denied" when not
        // elevated. Suppressing that error produced an empty list, which the UI
        // then stated as fact — "No restore points found" — when the truth was
        // "could not look". So the script reports success separately from data.
        //
        // CreationTime comes back in WMI's `yyyyMMddHHmmss.ffffff±UUU` form,
        // which JavaScript's Date cannot parse, so convert it here.
        let script = r#"
$ErrorActionPreference = 'Stop'
try {
    $points = @(Get-ComputerRestorePoint | ForEach-Object {
        [PSCustomObject]@{
            Description    = [string]$_.Description
            SequenceNumber = [int]$_.SequenceNumber
            CreationTime   = try {
                ([Management.ManagementDateTimeConverter]::ToDateTime($_.CreationTime)).ToString('o')
            } catch { '' }
        }
    })
    [PSCustomObject]@{ Ok = $true; Points = $points; Error = $null } |
        ConvertTo-Json -Depth 4 -Compress
} catch {
    [PSCustomObject]@{ Ok = $false; Points = @(); Error = [string]$_.Exception.Message } |
        ConvertTo-Json -Depth 4 -Compress
}
"#;
        let out = proc::powershell(script, std::time::Duration::from_secs(60))
            .await
            .map_err(|e| e.to_string())?;

        let result: RestorePointQuery =
            parse_ps_json(&out.stdout)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    "Could not read the restore point list; Windows returned nothing usable."
                        .to_string()
                })?;

        if !result.ok {
            let detail = result.error.unwrap_or_default();
            return Err(
                if detail.contains("Zugriff") || detail.to_lowercase().contains("denied") {
                    "Windows refused to list restore points. Listing them requires \
                 administrator rights — use \"Run as Admin\"."
                        .to_string()
                } else {
                    format!("Could not list restore points: {detail}")
                },
            );
        }

        Ok(result
            .points
            .into_iter()
            .map(|p| BackupDto {
                name: if p.description.trim().is_empty() {
                    format!("Restore point {}", p.sequence_number)
                } else {
                    p.description
                },
                created_at: p.creation_time.unwrap_or_default(),
                sequence_number: p.sequence_number,
                backup_type: "System Restore Point".to_string(),
            })
            .collect())
    }
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct RestorePointQuery {
    ok: bool,
    points: Vec<RestorePointJson>,
    error: Option<String>,
}

#[cfg(windows)]
impl Default for RestorePointQuery {
    fn default() -> Self {
        Self {
            ok: false,
            points: Vec::new(),
            error: Some("no response from Windows".into()),
        }
    }
}

#[cfg(windows)]
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct RestorePointJson {
    description: String,
    sequence_number: i64,
    creation_time: Option<String>,
}

#[tauri::command]
pub async fn create_backup(description: String) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = description;
        Err("System restore points are only available on Windows".to_string())
    }

    #[cfg(windows)]
    {
        if !odysync_core::platform::is_elevated() {
            return Err("Creating a restore point requires administrator rights. \
                 Use \"Run as Admin\" and try again."
                .into());
        }

        tracing::info!(desc = %description, "creating restore point");
        // Checkpoint-Computer exits 0 without doing anything when Windows'
        // 24-hour throttle applies, so success is confirmed by observing a new
        // sequence number rather than by trusting the exit code.
        let script = format!(
            r#"
$ErrorActionPreference = 'Stop'
$before = @(Get-ComputerRestorePoint | ForEach-Object {{ [int]$_.SequenceNumber }})
$maxBefore = if ($before.Count -gt 0) {{ ($before | Measure-Object -Maximum).Maximum }} else {{ 0 }}
Checkpoint-Computer -Description {desc} -RestorePointType 'MODIFY_SETTINGS'
$after = @(Get-ComputerRestorePoint | ForEach-Object {{ [int]$_.SequenceNumber }})
$maxAfter = if ($after.Count -gt 0) {{ ($after | Measure-Object -Maximum).Maximum }} else {{ 0 }}
if ($maxAfter -gt $maxBefore) {{
    Write-Output "ODYSYNC_CREATED:$maxAfter"
}} elseif ($before.Count -eq 0) {{
    # Nothing was created *and* none exist. The 24-hour throttle cannot explain
    # that, so System Protection is almost certainly off for the system drive.
    Write-Output "ODYSYNC_NO_PROTECTION"
}} else {{
    Write-Output "ODYSYNC_THROTTLED"
}}
"#,
            desc = ps_quote(&description),
        );
        let out = proc::powershell(&script, std::time::Duration::from_secs(300))
            .await
            .map_err(|e| e.to_string())?;

        if out.stdout.contains("ODYSYNC_CREATED") {
            tracing::info!("restore point created");
            Ok(())
        } else if out.stdout.contains("ODYSYNC_NO_PROTECTION") {
            Err(
                "Windows created no restore point and none exist, so System Protection \
                 is switched off for this drive. Turn it on first: Start > \"Create a \
                 restore point\" > select your system drive > Configure > Turn on system \
                 protection, and give it some disk space."
                    .to_string(),
            )
        } else if out.stdout.contains("ODYSYNC_THROTTLED") {
            Err(
                "Windows did not create a new restore point. By default it skips one if \
                 another was created in the last 24 hours — an existing recent restore \
                 point is already listed below."
                    .to_string(),
            )
        } else {
            let detail = out.stderr.trim();
            Err(if detail.is_empty() {
                "Could not create a restore point. Check that System Protection is \
                 enabled for the system drive."
                    .to_string()
            } else {
                format!("Could not create a restore point: {detail}")
            })
        }
    }
}

#[tauri::command]
pub async fn restore_backup(sequence_number: i64) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = sequence_number;
        Err("System restore is only available on Windows".to_string())
    }

    #[cfg(windows)]
    {
        if !odysync_core::platform::is_elevated() {
            return Err("Restoring requires administrator rights.".into());
        }
        tracing::warn!(sequence_number, "restoring system to restore point");
        let script = format!(
            r#"
$ErrorActionPreference = 'Stop'
Restore-Computer -RestorePoint {seq} -Confirm:$false
Write-Output "ODYSYNC_OK"
"#,
            seq = sequence_number,
        );
        let out = proc::powershell(&script, std::time::Duration::from_secs(300))
            .await
            .map_err(|e| e.to_string())?;

        if out.stdout.contains("ODYSYNC_OK") {
            Ok(())
        } else {
            let detail = out.stderr.trim();
            Err(if detail.is_empty() {
                "Restore did not start.".to_string()
            } else {
                format!("Restore did not start: {detail}")
            })
        }
    }
}

#[tauri::command]
pub async fn is_system_protection_enabled() -> Result<bool, String> {
    #[cfg(not(windows))]
    {
        Ok(false)
    }

    #[cfg(windows)]
    {
        // Read the configuration directly rather than inferring it from whether
        // any restore point happens to exist — a freshly enabled system has
        // protection on and no points yet, and the old check called that "off".
        let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$disabled = (Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore' -Name 'DisableSR').DisableSR
if ($disabled -eq 1) { Write-Output 'false'; exit }
$drive = $env:SystemDrive
$cfg = Get-CimInstance -Namespace 'root/default' -ClassName SystemRestoreConfig
if ($null -ne $cfg) { Write-Output 'true'; exit }
$vol = Get-CimInstance -ClassName Win32_ShadowStorage
if ($null -ne $vol) { Write-Output 'true' } else { Write-Output 'false' }
"#;
        let out = proc::powershell(script, std::time::Duration::from_secs(20))
            .await
            .map_err(|e| e.to_string())?;
        Ok(out.stdout.trim().eq_ignore_ascii_case("true"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_quote_escapes_embedded_single_quotes() {
        assert_eq!(ps_quote("plain"), "'plain'");
        assert_eq!(ps_quote("it's"), "'it''s'");
        // The injection this guards against: closing the literal and appending
        // a second statement.
        assert_eq!(
            ps_quote("x'; Remove-Item C:\\ -Recurse; '"),
            "'x''; Remove-Item C:\\ -Recurse; '''"
        );
    }

    #[test]
    fn parses_a_standard_tracing_line() {
        let entry = parse_log_line(
            "2026-07-21T09:15:00.123456Z  INFO odysync_gui::commands: scan complete total=3",
        );
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.timestamp, "2026-07-21T09:15:00.123456Z");
        assert_eq!(entry.message, "scan complete total=3");
    }

    #[test]
    fn parses_a_warn_line_with_padding() {
        let entry =
            parse_log_line("2026-07-21T09:15:00.123456Z  WARN odysync_backends: scan failed");
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.message, "scan failed");
    }

    #[test]
    fn falls_back_for_unstructured_lines() {
        let entry = parse_log_line("a bare line with no level");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.timestamp, "");
        assert_eq!(entry.message, "a bare line with no level");
    }

    #[test]
    fn parse_ps_json_accepts_object_and_array() {
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Row {
            a: u32,
        }
        assert_eq!(parse_ps_json::<Row>(r#"{"a":1}"#), vec![Row { a: 1 }]);
        assert_eq!(
            parse_ps_json::<Row>(r#"[{"a":1},{"a":2}]"#),
            vec![Row { a: 1 }, Row { a: 2 }]
        );
        assert_eq!(parse_ps_json::<Row>("  "), Vec::<Row>::new());
    }

    #[test]
    fn config_dto_patch_preserves_holds_and_profiles() {
        let mut config = Config::default();
        config.policy.holds.push(odysync_core::policy::Hold {
            package: "winget:Mozilla.Firefox".into(),
            pin: None,
            note: None,
        });
        config.profiles.push(odysync_core::config::Profile {
            name: "dev".into(),
            packages: vec!["Git.Git".into()],
        });

        // What the frontend posts back: holds and profiles are skip_deserializing,
        // so they arrive empty regardless of what the UI had.
        let dto: ConfigDto = serde_json::from_str(
            r#"{
                "policy": {
                    "stable_only": false,
                    "require_known_versions": true,
                    "exclude": ["  Some.Package  ", ""]
                },
                "disabled_backends": ["scoop"],
                "restore_point": false,
                "scan_interval_hours": 6,
                "concurrency": 99,
                "proxy_url": "   ",
                "auto_apply": true,
                "notifications": false,
                "max_retries": 3,
                "backend_timeout_secs": 5
            }"#,
        )
        .unwrap();

        dto.apply_to(&mut config);

        assert_eq!(config.policy.holds.len(), 1, "holds must survive a save");
        assert_eq!(config.profiles.len(), 1, "profiles must survive a save");
        assert_eq!(config.policy.exclude, vec!["Some.Package".to_string()]);
        assert!(!config.policy.stable_only);
        assert!(!config.skip_prerelease, "kept in sync with stable_only");
        assert_eq!(config.disabled_backends, vec!["scoop".to_string()]);
        assert_eq!(config.concurrency, 16, "clamped to the supported range");
        assert_eq!(config.backend_timeout_secs, 10, "clamped to the minimum");
        assert_eq!(config.proxy_url, None, "blank proxy becomes None");
        assert_eq!(config.scan_interval_hours, 6);
    }

    #[test]
    fn config_dto_round_trips_field_names_the_frontend_uses() {
        let dto = ConfigDto::from(&Config::default());
        let json = serde_json::to_value(&dto).unwrap();
        for key in [
            "policy",
            "disabled_backends",
            "profiles",
            "restore_point",
            "scan_interval_hours",
            "concurrency",
            "proxy_url",
            "auto_apply",
            "notifications",
            "max_retries",
            "backend_timeout_secs",
        ] {
            assert!(
                json.get(key).is_some(),
                "missing `{key}` in the wire format"
            );
        }
        let policy = json.get("policy").unwrap();
        for key in ["stable_only", "require_known_versions", "exclude", "holds"] {
            assert!(
                policy.get(key).is_some(),
                "missing `policy.{key}` in the wire format"
            );
        }
    }

    /// Pins the security wire format against `apps/gui/src/types.ts`.
    ///
    /// `ScanReport`/`SectionResult` are camelCase while `Remediation`'s inner
    /// fields are snake_case — `rename_all` on an enum renames variants, not
    /// fields. That asymmetry is easy to "tidy up" into a silent breakage,
    /// because a mismatched key deserializes to `undefined` rather than
    /// erroring. Exactly how the settings page broke before.
    #[test]
    fn security_wire_format_matches_the_frontend() {
        use security::{Finding, Remediation, ScanReport, SectionResult, Severity};

        let report = ScanReport {
            findings: vec![
                Finding::new("test:1", Severity::Critical, "malware", "t", "d").with_remediation(
                    Remediation::RemoveDefenderThreat {
                        threat_id: "42".into(),
                    },
                ),
            ],
            scanned_at: "2026-07-21T00:00:00Z".into(),
            sections: vec![SectionResult {
                name: "defender".into(),
                ok: false,
                error: Some("nope".into()),
                duration_ms: 5,
            }],
        };

        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("scannedAt").is_some(), "ScanReport is camelCase");
        assert!(json.get("scanned_at").is_none());
        assert!(
            json["sections"][0].get("durationMs").is_some(),
            "SectionResult is camelCase"
        );

        let finding = &json["findings"][0];
        for key in ["id", "severity", "category", "title", "detail", "evidence"] {
            assert!(finding.get(key).is_some(), "missing finding.{key}");
        }
        assert_eq!(finding["severity"], "critical", "Severity is kebab-case");

        let rem = &finding["remediation"];
        assert_eq!(rem["kind"], "remove-defender-threat", "tag is kebab-case");
        assert!(
            rem.get("threat_id").is_some(),
            "variant fields stay snake_case"
        );

        // And it must survive the trip back, since `apply_remediation` takes it
        // as a command parameter.
        let round_tripped: Remediation =
            serde_json::from_value(rem.clone()).expect("remediation must deserialize");
        assert_eq!(
            round_tripped,
            Remediation::RemoveDefenderThreat {
                threat_id: "42".into()
            }
        );
    }

    #[test]
    fn outcome_status_is_stable_and_machine_readable() {
        let (status, detail) = outcome_parts(&ApplyOutcome::Updated {
            from: "1.0".into(),
            to: "1.1".into(),
        });
        assert_eq!(status, "updated");
        assert_eq!(detail, "1.0 -> 1.1");

        // A failure whose detail happens to contain the word "Updated" must not
        // be misread as a success, which substring matching on Debug output did.
        let (status, _) = outcome_parts(&ApplyOutcome::Failed {
            detail: "package Updated was not found".into(),
        });
        assert_eq!(status, "failed");
    }
}
