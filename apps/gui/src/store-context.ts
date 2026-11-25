/**
 * The shape of the application store and the hook that reads it.
 *
 * Kept apart from `store.tsx`, which holds the provider component, so that a
 * change to the provider can be hot-reloaded on its own.
 */
import { createContext, useContext } from "react";
import type { LoadOptions } from "./coalesce";
import type { ScanHealth } from "./scan-health";
import type {
  ApplyProgress,
  ApplyRequest,
  ApplyResultDto,
  AutostartConfig,
  BackendDto,
  BackupDto,
  CleanupItemDto,
  ComponentStatus,
  Config,
  DefenderStatusDto,
  DeviceProblem,
  DriverBackupDto,
  HardwareInfoDto,
  HealthReportDto,
  HistoryEntryDto,
  InstalledPackageDto,
  LogEntryDto,
  OfflineCacheStatusDto,
  OfflineManifestEntryDto,
  ProfileDto,
  ScanReport,
  ScanResult,
  SectionProgress,
  StartupProgramDto,
  SystemInfoDto,
} from "./types";

export interface Resource<T> {
  data: T;
  /** A load somebody is waiting for. Background polls do not set it. */
  loading: boolean;
  /** Error text from the most recent failed load, cleared on success. */
  error: string | null;
  /** Epoch millis of the last successful load, or null if never loaded. */
  loadedAt: number | null;
  /** Load again, or join the load that is already running. */
  refresh: (options?: LoadOptions) => Promise<void>;
  /** Load again after a change: a load that is already running is not enough. */
  reload: () => Promise<void>;
  /** Replace the data locally, without a round-trip and until the next load. */
  set: (value: T) => void;
  /**
   * The data is what the previous session found, shown while the first load
   * of this session is still running.
   */
  fromCache: boolean;
}

export interface OfflineData {
  status: OfflineCacheStatusDto | null;
  entries: OfflineManifestEntryDto[];
}

export interface BackupData {
  backups: BackupDto[];
  protectionEnabled: boolean;
}

/** The security audit, plus the "run it at startup" preference. */
export interface SecurityResource extends Resource<ScanReport | null> {
  scanOnStartup: boolean;
  setScanOnStartup: (v: boolean) => void;
  /** Live per-section progress of the running audit, by section name. */
  sections: Record<string, SectionProgress>;
}

/** How far the running scan has got. */
export interface ScanProgress {
  done: number;
  total: number;
  /** Updates found so far. */
  found: number;
  /** Names of the sources that have started and not answered. */
  waiting: string[];
}

export interface ScanResource extends Resource<ScanResult | null> {
  health: ScanHealth;
  /** Null until the first source reports in. */
  progress: ScanProgress | null;
  /** A cancel was asked for and has not taken effect yet. */
  cancelling: boolean;
  cancel: () => Promise<void>;
}

export interface ApplyOutcome {
  result: ApplyResultDto;
  /** The user cancelled; packages not yet started were skipped. */
  cancelled: boolean;
}

/**
 * Installing updates. Shared, because Updates, Drivers and Home all start it
 * and only one run may be going at a time.
 */
export interface ApplyState {
  running: boolean;
  /** The running apply is a dry run. */
  dryRun: boolean;
  progress: ApplyProgress | null;
  cancelling: boolean;
  /** Rejects with "apply-running" when a run is already going. */
  run: (request: ApplyRequest) => Promise<ApplyOutcome>;
  cancel: () => Promise<void>;
}

export interface AppStore {
  sysInfo: Resource<SystemInfoDto | null>;
  security: SecurityResource;
  defender: Resource<DefenderStatusDto | null>;
  autostart: Resource<AutostartConfig | null>;
  backends: Resource<BackendDto[]>;
  config: Resource<Config | null>;
  scan: ScanResource;
  apply: ApplyState;
  hardware: Resource<HardwareInfoDto | null>;
  packages: Resource<InstalledPackageDto[]>;
  history: Resource<HistoryEntryDto[]>;
  logs: Resource<LogEntryDto[]>;
  profiles: Resource<ProfileDto[]>;
  offline: Resource<OfflineData>;
  startup: Resource<StartupProgramDto[]>;
  backup: Resource<BackupData>;
  cleanup: Resource<CleanupItemDto[]>;
  components: Resource<ComponentStatus[]>;
  devices: Resource<DeviceProblem[]>;
  driverBackups: Resource<DriverBackupDto[]>;
  health: Resource<HealthReportDto | null>;
  /** True until the initial background load of every resource has settled. */
  booting: boolean;
  /** Fraction of the initial background load that has completed, 0..1. */
  bootProgress: number;
  /** Refresh everything (used by the global refresh button). */
  refreshAll: () => Promise<void>;
}

export const StoreContext = createContext<AppStore | null>(null);

export function useStore(): AppStore {
  const ctx = useContext(StoreContext);
  if (!ctx) throw new Error("useStore must be used within <StoreProvider>");
  return ctx;
}
