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

export async function clearOfflineCache(): Promise<void> {
  return invoke<void>("clear_offline_cache");
}

export async function listOfflineCache(): Promise<OfflineManifestEntryDto[]> {
  return invoke<OfflineManifestEntryDto[]>("list_offline_cache");
}

export async function clearOfflineManifest(): Promise<void> {
  return invoke<void>("clear_offline_manifest");
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
