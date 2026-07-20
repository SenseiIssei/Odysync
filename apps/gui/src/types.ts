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

export interface Config {
  policy: {
    stable_only: boolean;
    require_known_versions: boolean;
    elevated: boolean;
    exclude: string[];
    holds: string[];
    pins: Record<string, string>;
  };
  disabled_backends: string[];
  profiles: Record<string, string[]>;
  restore_point: boolean;
}
