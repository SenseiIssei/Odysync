import { invoke } from "@tauri-apps/api/core";
import type {
  ScanResult,
  BackendDto,
  SystemInfoDto,
  ApplyResultDto,
  ApplyRequest,
  HoldRequest,
  ScheduleRequest,
  Config,
  HistoryEntryDto,
  HardwareInfoDto,
  InstalledPackageDto,
  LogEntryDto,
  ProfileDto,
  OfflineCacheStatusDto,
  OfflineManifestEntryDto,
  StartupProgramDto,
  BackupDto,
  ScanReport,
  DefenderStatusDto,
  Remediation,
  AutostartConfig,
} from "./types";

export async function scan(): Promise<ScanResult> {
  return invoke<ScanResult>("scan");
}

export async function apply(request: ApplyRequest): Promise<ApplyResultDto> {
  return invoke<ApplyResultDto>("apply", { request });
}

export async function listBackends(): Promise<BackendDto[]> {
  return invoke<BackendDto[]>("list_backends");
}

/** Force a fresh availability probe; `listBackends` serves a cached result. */
export async function refreshBackends(): Promise<BackendDto[]> {
  return invoke<BackendDto[]>("refresh_backends");
}

export async function getConfig(): Promise<Config> {
  return invoke<Config>("get_config");
}

export async function saveConfig(config: Config): Promise<void> {
  return invoke<void>("save_config", { config });
}

export async function hold(request: HoldRequest): Promise<void> {
  return invoke<void>("hold", { request });
}

export async function unhold(request: HoldRequest): Promise<void> {
  return invoke<void>("unhold", { request });
}

export async function runMaintenance(action: string): Promise<string> {
  return invoke<string>("run_maintenance", { action });
}

export async function listMaintenance(): Promise<string[]> {
  return invoke<string[]>("list_maintenance");
}

export async function createSchedule(request: ScheduleRequest): Promise<string> {
  return invoke<string>("create_schedule", { request });
}

export async function removeSchedule(taskName: string): Promise<boolean> {
  return invoke<boolean>("remove_schedule", { taskName });
}

export async function checkSchedule(taskName: string): Promise<boolean> {
  return invoke<boolean>("check_schedule", { taskName });
}

export async function createDiagnostics(outPath: string): Promise<void> {
  return invoke<void>("create_diagnostics", { outPath });
}

export async function getSystemInfo(): Promise<SystemInfoDto> {
  return invoke<SystemInfoDto>("get_system_info");
}

/**
 * Record a frontend crash in odysync.log.
 *
 * Never throws: this runs from an error handler, and a failure here would
 * replace the real error with a misleading one.
 */
export async function reportFrontendError(
  context: string,
  message: string,
  stack?: string,
): Promise<void> {
  try {
    await invoke<void>("report_frontend_error", { context, message, stack: stack ?? null });
  } catch (e) {
    console.error("[odysync] could not report frontend error:", e);
  }
}

export async function getUpdateHistory(): Promise<HistoryEntryDto[]> {
  return invoke<HistoryEntryDto[]>("get_update_history");
}

export async function clearUpdateHistory(): Promise<void> {
  return invoke<void>("clear_update_history");
}

export async function getHardwareInfo(): Promise<HardwareInfoDto> {
  return invoke<HardwareInfoDto>("get_hardware_info");
}

export async function listInstalledPackages(): Promise<InstalledPackageDto[]> {
  return invoke<InstalledPackageDto[]>("list_installed_packages");
}

export async function getLogs(): Promise<LogEntryDto[]> {
  return invoke<LogEntryDto[]>("get_logs");
}

/** Absolute path of the directory holding odysync.log. */
export async function getLogFolder(): Promise<string> {
  return invoke<string>("open_log_folder");
}

export async function listProfiles(): Promise<ProfileDto[]> {
  return invoke<ProfileDto[]>("list_profiles");
}

export async function createProfile(name: string, packages: string[]): Promise<void> {
  return invoke<void>("create_profile", { name, packages });
}

export async function deleteProfile(name: string): Promise<void> {
  return invoke<void>("delete_profile", { name });
}

export async function getOfflineCacheStatus(): Promise<OfflineCacheStatusDto> {
  return invoke<OfflineCacheStatusDto>("get_offline_cache_status");
}

export async function listOfflineCache(): Promise<OfflineManifestEntryDto[]> {
  return invoke<OfflineManifestEntryDto[]>("list_offline_cache");
}

export async function clearOfflineManifest(): Promise<void> {
  return invoke<void>("clear_offline_manifest");
}

/** Drop manifest entries whose cached file is gone. Returns how many. */
export async function pruneOfflineCache(): Promise<number> {
  return invoke<number>("prune_offline_cache");
}

export async function removeOfflineEntry(packageId: string, backend: string): Promise<void> {
  return invoke<void>("remove_offline_entry", { packageId, backend });
}

export async function downloadOfflineInstaller(
  url: string,
  packageId: string,
  backend: string,
  version: string,
  expectedSha256?: string,
): Promise<void> {
  return invoke<void>("download_offline_installer", {
    url,
    packageId,
    backend,
    version,
    expectedSha256: expectedSha256 ?? null,
  });
}

export async function verifyOfflineCache(): Promise<boolean[]> {
  return invoke<boolean[]>("verify_offline_cache");
}

export async function restartAsAdmin(): Promise<void> {
  return invoke<void>("restart_as_admin");
}

export async function quitApp(): Promise<void> {
  return invoke<void>("quit_app");
}

export async function listStartupPrograms(): Promise<StartupProgramDto[]> {
  return invoke<StartupProgramDto[]>("list_startup_programs");
}

export async function toggleStartupProgram(name: string, location: string, enable: boolean): Promise<void> {
  return invoke<void>("toggle_startup_program", { name, location, enable });
}

export async function listBackups(): Promise<BackupDto[]> {
  return invoke<BackupDto[]>("list_backups");
}

export async function createBackup(description: string): Promise<void> {
  return invoke<void>("create_backup", { description });
}

export async function restoreBackup(sequenceNumber: number): Promise<void> {
  return invoke<void>("restore_backup", { sequenceNumber });
}

export async function isSystemProtectionEnabled(): Promise<boolean> {
  return invoke<boolean>("is_system_protection_enabled");
}

// ── Security ─────────────────────────────────────────────────────────────────

/** Full posture + indicator audit. Slow: shells out to Defender, WMI and CIM. */
export async function securityScan(): Promise<ScanReport> {
  return invoke<ScanReport>("security_scan");
}

export async function getDefenderStatus(): Promise<DefenderStatusDto> {
  return invoke<DefenderStatusDto>("get_defender_status");
}

export async function defenderQuickScan(): Promise<string> {
  return invoke<string>("defender_quick_scan");
}

/** Reads every file on every drive; can run for hours. */
export async function defenderFullScan(): Promise<string> {
  return invoke<string>("defender_full_scan");
}

export async function updateDefenderSignatures(): Promise<string> {
  return invoke<string>("update_defender_signatures");
}

/** Returns a human-readable description of what was actually changed. */
export async function applyRemediation(remediation: Remediation): Promise<string> {
  return invoke<string>("apply_remediation", { remediation });
}

// ── Autostart ────────────────────────────────────────────────────────────────

export async function getAutostart(): Promise<AutostartConfig> {
  return invoke<AutostartConfig>("get_autostart");
}

export async function enableAutostart(minimized: boolean): Promise<void> {
  return invoke<void>("enable_autostart", { minimized });
}

export async function disableAutostart(): Promise<void> {
  return invoke<void>("disable_autostart");
}
