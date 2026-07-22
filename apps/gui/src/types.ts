/**
 * Wire types shared with the Tauri command layer.
 *
 * These mirror the DTOs in `src-tauri/src/commands.rs` exactly. They are NOT
 * the core Rust types: `odysync_core::Config` serializes as kebab-case, so the
 * command layer exposes an explicit snake_case DTO instead. Keep both sides in
 * step — a mismatch here shows up as silently `undefined` fields, not an error.
 */

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

export interface BackendErrorDto {
  backend: string;
  error: string;
}

export interface ScanResult {
  actionable: UpdateDto[];
  skipped: SkippedDto[];
  total: number;
  failed_backends: BackendErrorDto[];
}

export interface BackendDto {
  kind: string;
  name: string;
  available: boolean;
  enabled: boolean;
}

export interface SystemInfoDto {
  os: string;
  elevated: boolean;
  version: string;
  config_error: string | null;
}

/** Stable outcome discriminants emitted by the command layer. */
export type ApplyStatus =
  | "updated"
  | "did-not-converge"
  | "verification-failed"
  | "failed"
  | "skipped";

export interface ApplyEntryDto {
  name: string;
  status: ApplyStatus;
  detail: string;
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

export interface PolicyConfig {
  stable_only: boolean;
  require_known_versions: boolean;
  exclude: string[];
  /** Read-only: managed through the hold/unhold commands, never written back. */
  holds: Hold[];
}

export interface Config {
  policy: PolicyConfig;
  disabled_backends: string[];
  /** Read-only: managed through the profile commands. */
  profiles: Profile[];
  restore_point: boolean;
  scan_interval_hours: number;
  concurrency: number;
  proxy_url: string | null;
  auto_apply: boolean;
  notifications: boolean;
  max_retries: number;
  backend_timeout_secs: number;
}

export interface HistoryEntryDto {
  timestamp: string;
  package: string;
  backend: string;
  from_version: string;
  to_version: string;
  status: ApplyStatus;
  detail: string;
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

export interface StartupProgramDto {
  name: string;
  command: string;
  location: string;
  enabled: boolean;
}

export interface BackupDto {
  name: string;
  created_at: string;
  sequence_number: number;
  backup_type: string;
}

// ── Security ─────────────────────────────────────────────────────────────────

export type Severity = "critical" | "high" | "medium" | "low" | "info";

/**
 * Mirrors `Remediation` in `odysync_backends::security`, which is
 * `#[serde(tag = "kind", rename_all = "kebab-case")]`.
 *
 * Note the asymmetry: `rename_all` on an enum renames the *variants*, so the
 * tags are kebab-case while the fields inside each variant stay snake_case.
 * `security_wire_format_matches_the_frontend` in the command layer pins this.
 */
export type Remediation =
  | { kind: "remove-defender-threat"; threat_id: string }
  | { kind: "disable-run-key"; hive: string; name: string }
  | { kind: "disable-scheduled-task"; task_path: string }
  | { kind: "delete-file"; path: string }
  | { kind: "stop-and-disable-service"; name: string }
  | { kind: "reset-hosts-file" }
  | { kind: "manual"; instructions: string };

export interface SecurityFinding {
  id: string;
  severity: Severity;
  /** "malware" | "persistence" | "network" | "posture" | "integrity" | "account" */
  category: string;
  title: string;
  detail: string;
  evidence: string[];
  remediation: Remediation | null;
}

/** camelCase on the wire — see `SectionResult` in `security::mod`. */
export interface SectionResult {
  name: string;
  ok: boolean;
  error: string | null;
  durationMs: number;
}

/** camelCase on the wire — see `ScanReport` in `security::mod`. */
export interface ScanReport {
  findings: SecurityFinding[];
  scannedAt: string;
  sections: SectionResult[];
}

/** Live per-section progress from the `security-progress` event. */
export interface SectionProgress {
  name: string;
  /** "started" | "done" | "failed" */
  state: string;
  durationMs: number | null;
  findings: number | null;
}

export interface DefenderStatusDto {
  real_time_protection: boolean;
  tamper_protection: boolean;
  antivirus_enabled: boolean;
  signature_age_days: number;
  signature_version: string;
  last_quick_scan: string | null;
  last_full_scan: string | null;
}

export interface AutostartConfig {
  enabled: boolean;
  minimized: boolean;
}
