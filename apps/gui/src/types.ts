export interface UpdateDto {
  backend: string;
  id: string;
  name: string;
  installed: string;
  available: string;
  size_bytes: number | null;
}

export interface SkippedDto {
  backend: string;
  id: string;
  name: string;
  reason: string;
}

export interface ScanResult {
  actionable: UpdateDto[];
  skipped: SkippedDto[];
  total: number;
}

export interface BackendDto {
  kind: string;
  name: string;
  available: boolean;
}

export interface SystemInfoDto {
  os: string;
  elevated: boolean;
  version: string;
}

export interface ApplyEntryDto {
  name: string;
  outcome: string;
  reboot_required: boolean;
}

export interface ApplyResultDto {
  updated: number;
  failed: number;
  skipped: number;
  reboot_required: boolean;
  exit_code: number;
  entries: ApplyEntryDto[];
}

export interface ApplyRequest {
  updates: UpdateDto[];
  dry_run: boolean;
  restore_point: boolean;
}

export interface HoldRequest {
  backend: string;
  id: string;
}

export interface ScheduleRequest {
  frequency: string;
  time: string;
  task_name: string | null;
}

export interface Hold {
  package: string;
  pin: string | null;
  note: string | null;
}

export interface Profile {
  name: string;
  packages: string[];
}

export interface Config {
  policy: {
    stable_only: boolean;
    require_known_versions: boolean;
    elevated: boolean;
    exclude: string[];
    holds: Hold[];
  };
  disabled_backends: string[];
  profiles: Profile[];
  restore_point: boolean;
  scan_interval_hours: number;
  concurrency: number;
  proxy_url: string | null;
  auto_apply: boolean;
  notifications: boolean;
  skip_prerelease: boolean;
  max_retries: number;
  backend_timeout_secs: number;
}

export interface HistoryEntryDto {
  timestamp: string;
  package: string;
  backend: string;
  from_version: string;
  to_version: string;
  outcome: string;
}

export interface GpuInfoDto {
  name: string;
  driver_version: string;
  vendor: string;
}

export interface DiskInfoDto {
  name: string;
  size_gb: number;
  filesystem: string;
}

export interface HardwareInfoDto {
  cpu: string;
  cpu_cores: number;
  total_memory_gb: number;
  os: string;
  gpu: GpuInfoDto[];
  disks: DiskInfoDto[];
}

export interface InstalledPackageDto {
  backend: string;
  id: string;
  name: string;
  version: string;
}

export interface LogEntryDto {
  level: string;
  message: string;
  timestamp: string;
}

export interface ProfileDto {
  name: string;
  packages: string[];
}

export interface OfflineCacheStatusDto {
  entry_count: number;
  cache_size_bytes: number;
}

export interface OfflineManifestEntryDto {
  package_id: string;
  backend: string;
  version: string;
  filename: string;
  sha256: string;
  size_bytes: number;
  cached_at: string;
}
