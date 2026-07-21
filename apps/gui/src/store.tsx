/**
 * Central application store.
 *
 * Every page reads its data from here rather than fetching on mount, so that:
 *   - all data is loaded once in the background at startup,
 *   - switching pages never triggers a refetch or loses in-flight work,
 *   - a page that is opened before its data has landed shows a live loading
 *     state instead of an empty screen.
 *
 * Loaders are funnelled through a small concurrency queue. Several of the
 * backing Tauri commands shell out to PowerShell or winget and take seconds;
 * firing fourteen of them at once made the window unresponsive at startup.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import * as api from "./api";
import { safeListen } from "./events";
import type {
  AutostartConfig,
  BackupDto,
  Config,
  DefenderStatusDto,
  ScanReport,
  HardwareInfoDto,
  HistoryEntryDto,
  InstalledPackageDto,
  LogEntryDto,
  OfflineCacheStatusDto,
  OfflineManifestEntryDto,
  ProfileDto,
  ScanResult,
  StartupProgramDto,
  SystemInfoDto,
  BackendDto,
} from "./types";

// ── Concurrency queue ────────────────────────────────────────────────────────

function createQueue(limit: number) {
  let active = 0;
  const pending: Array<() => void> = [];

  const pump = () => {
    while (active < limit && pending.length > 0) {
      const task = pending.shift()!;
      active++;
      task();
    }
  };

  return function enqueue<T>(fn: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      pending.push(() => {
        fn()
          .then(resolve, reject)
          .finally(() => {
            active--;
            pump();
          });
      });
      pump();
    });
  };
}

/** Heavy loaders (winget / PowerShell) run at most three at a time. */
const heavyQueue = createQueue(3);

// ── Resource primitive ───────────────────────────────────────────────────────

export interface Resource<T> {
  data: T;
  loading: boolean;
  /** Error text from the most recent failed load, cleared on success. */
  error: string | null;
  /** Epoch millis of the last successful load, or null if never loaded. */
  loadedAt: number | null;
  refresh: () => Promise<void>;
  /** Replace the data locally without a round-trip (optimistic updates). */
  set: (value: T) => void;
}

interface ResourceOptions {
  /** Route the loader through the shared concurrency queue. */
  heavy?: boolean;
}

function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

function useResource<T>(
  name: string,
  loader: () => Promise<T>,
  initial: T,
  options: ResourceOptions = {},
): Resource<T> {
  const [data, setData] = useState<T>(initial);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadedAt, setLoadedAt] = useState<number | null>(null);

  // Keep the latest loader without making `refresh` unstable.
  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  // Coalesces concurrent refreshes (including React StrictMode's double
  // effect invocation in dev) into a single request.
  const inFlight = useRef<Promise<void> | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const heavy = options.heavy ?? false;

  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) return inFlight.current;

    setLoading(true);
    const run = async () => {
      try {
        const value = heavy
          ? await heavyQueue(() => loaderRef.current())
          : await loaderRef.current();
        if (!mounted.current) return;
        setData(value);
        setError(null);
        setLoadedAt(Date.now());
      } catch (e) {
        if (!mounted.current) return;
        const text = errorText(e);
        console.error(`[odysync] ${name} failed:`, e);
        setError(text);
      } finally {
        if (mounted.current) setLoading(false);
        inFlight.current = null;
      }
    };

    const promise = run();
    inFlight.current = promise;
    return promise;
  }, [heavy, name]);

  const set = useCallback((value: T) => {
    setData(value);
    setLoadedAt(Date.now());
    setError(null);
  }, []);

  return useMemo(
    () => ({ data, loading, error, loadedAt, refresh, set }),
    [data, loading, error, loadedAt, refresh, set],
  );
}

// ── Store shape ──────────────────────────────────────────────────────────────

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
}

export interface AppStore {
  sysInfo: Resource<SystemInfoDto | null>;
  security: SecurityResource;
  defender: Resource<DefenderStatusDto | null>;
  autostart: Resource<AutostartConfig | null>;
  backends: Resource<BackendDto[]>;
  config: Resource<Config | null>;
  scan: Resource<ScanResult | null>;
  hardware: Resource<HardwareInfoDto | null>;
  packages: Resource<InstalledPackageDto[]>;
  history: Resource<HistoryEntryDto[]>;
  logs: Resource<LogEntryDto[]>;
  profiles: Resource<ProfileDto[]>;
  offline: Resource<OfflineData>;
  startup: Resource<StartupProgramDto[]>;
  backup: Resource<BackupData>;
  /** True until the initial background load of every resource has settled. */
  booting: boolean;
  /** Fraction of the initial background load that has completed, 0..1. */
  bootProgress: number;
  /** Refresh everything (used by the global refresh button). */
  refreshAll: () => Promise<void>;
}

const StoreContext = createContext<AppStore | null>(null);

export function useStore(): AppStore {
  const ctx = useContext(StoreContext);
  if (!ctx) throw new Error("useStore must be used within <StoreProvider>");
  return ctx;
}

const EMPTY_OFFLINE: OfflineData = { status: null, entries: [] };
const EMPTY_BACKUP: BackupData = { backups: [], protectionEnabled: false };
const SCAN_ON_STARTUP_KEY = "odysync.securityScanOnStartup";

/** Defender state is one cheap CIM read; poll it often enough to feel live. */
const DEFENDER_POLL_MS = 60_000;

/** The full audit spawns many PowerShell queries — keep it well clear of idle. */
const SECURITY_POLL_MS = 10 * 60_000;

export function StoreProvider({ children }: { children: ReactNode }) {
  const sysInfo = useResource<SystemInfoDto | null>(
    "system info",
    () => api.getSystemInfo(),
    null,
  );

  // The first load fills the backend cache on the Rust side; every later
  // refresh is an explicit re-probe, which is what the user means by "refresh".
  const backendsProbed = useRef(false);
  const backends = useResource<BackendDto[]>(
    "backends",
    async () => {
      const result = backendsProbed.current
        ? await api.refreshBackends()
        : await api.listBackends();
      backendsProbed.current = true;
      return result;
    },
    [],
    { heavy: true },
  );

  const config = useResource<Config | null>("config", () => api.getConfig(), null);

  const scan = useResource<ScanResult | null>("scan", () => api.scan(), null, {
    heavy: true,
  });

  const hardware = useResource<HardwareInfoDto | null>(
    "hardware info",
    () => api.getHardwareInfo(),
    null,
    { heavy: true },
  );

  const packages = useResource<InstalledPackageDto[]>(
    "installed packages",
    () => api.listInstalledPackages(),
    [],
    { heavy: true },
  );

  const history = useResource<HistoryEntryDto[]>(
    "update history",
    () => api.getUpdateHistory(),
    [],
  );

  const logs = useResource<LogEntryDto[]>("logs", () => api.getLogs(), []);

  const profiles = useResource<ProfileDto[]>(
    "profiles",
    () => api.listProfiles(),
    [],
  );

  const offline = useResource<OfflineData>(
    "offline cache",
    async () => {
      const [status, entries] = await Promise.all([
        api.getOfflineCacheStatus(),
        api.listOfflineCache(),
      ]);
      return { status, entries };
    },
    EMPTY_OFFLINE,
  );

  const startup = useResource<StartupProgramDto[]>(
    "startup programs",
    () => api.listStartupPrograms(),
    [],
    { heavy: true },
  );

  const defender = useResource<DefenderStatusDto | null>(
    "defender status",
    () => api.getDefenderStatus(),
    null,
    { heavy: true },
  );

  const autostart = useResource<AutostartConfig | null>(
    "autostart",
    () => api.getAutostart(),
    null,
  );

  const securityScan = useResource<ScanReport | null>(
    "security audit",
    () => api.securityScan(),
    null,
    { heavy: true },
  );

  const backup = useResource<BackupData>(
    "backups",
    async () => {
      const [backups, protectionEnabled] = await Promise.all([
        api.listBackups(),
        api.isSystemProtectionEnabled(),
      ]);
      return { backups, protectionEnabled };
    },
    EMPTY_BACKUP,
    { heavy: true },
  );

  // Whether the security audit runs as part of the startup load. It is the
  // slowest thing in the app, so it is opt-in — but on by default, because a
  // machine that has already been compromised should be told, not asked.
  const [scanOnStartup, setScanOnStartupState] = useState(
    () => localStorage.getItem(SCAN_ON_STARTUP_KEY) !== "false",
  );
  const setScanOnStartup = useCallback((v: boolean) => {
    localStorage.setItem(SCAN_ON_STARTUP_KEY, String(v));
    setScanOnStartupState(v);
  }, []);

  const resources = useMemo(
    () => [
      sysInfo,
      config,
      autostart,
      backends,
      history,
      logs,
      profiles,
      offline,
      hardware,
      defender,
      packages,
      startup,
      backup,
      scan,
      ...(scanOnStartup ? [securityScan] : []),
    ],
    [
      sysInfo,
      config,
      autostart,
      backends,
      history,
      logs,
      profiles,
      offline,
      hardware,
      defender,
      packages,
      startup,
      backup,
      scan,
      securityScan,
      scanOnStartup,
    ],
  );

  // Keep a stable handle on the refresh functions so the boot effect runs once.
  const refreshFns = useRef<Array<() => Promise<void>>>([]);
  refreshFns.current = resources.map((r) => r.refresh);

  const [booting, setBooting] = useState(true);
  const [bootDone, setBootDone] = useState(0);
  const bootTotal = resources.length;

  // Background-load everything, once, at startup. Cheap metadata first so the
  // shell fills in immediately; the expensive scans follow behind the queue.
  const booted = useRef(false);
  useEffect(() => {
    if (booted.current) return;
    booted.current = true;

    let cancelled = false;
    const tasks = refreshFns.current.map((fn) =>
      fn().finally(() => {
        if (!cancelled) setBootDone((n) => n + 1);
      }),
    );

    Promise.allSettled(tasks).then(() => {
      if (!cancelled) setBooting(false);
    });

    return () => {
      cancelled = true;
    };
  }, []);

  const refreshAll = useCallback(async () => {
    await Promise.allSettled(refreshFns.current.map((fn) => fn()));
  }, []);

  // A finished apply invalidates almost everything.
  const refreshScan = scan.refresh;
  const refreshHistory = history.refresh;
  const refreshPackages = packages.refresh;
  const refreshLogs = logs.refresh;
  useEffect(
    () =>
      safeListen("apply-finished", () => {
        void refreshScan();
        void refreshHistory();
        void refreshPackages();
        void refreshLogs();
      }),
    [refreshScan, refreshHistory, refreshPackages, refreshLogs],
  );

  const security: SecurityResource = useMemo(
    () => ({ ...securityScan, scanOnStartup, setScanOnStartup }),
    [securityScan, scanOnStartup, setScanOnStartup],
  );

  // Keep the security view live rather than a snapshot from startup. Defender's
  // own state is cheap to read and changes on its own (a scan finishes, a
  // signature update lands, something gets quarantined), so it polls often; the
  // full audit shells out to WMI and CIM, so it refreshes far more slowly.
  const refreshDefender = defender.refresh;
  const refreshSecurity = securityScan.refresh;
  useEffect(() => {
    const defenderTimer = setInterval(() => void refreshDefender(), DEFENDER_POLL_MS);
    const auditTimer = setInterval(() => void refreshSecurity(), SECURITY_POLL_MS);
    return () => {
      clearInterval(defenderTimer);
      clearInterval(auditTimer);
    };
  }, [refreshDefender, refreshSecurity]);

  const value: AppStore = useMemo(
    () => ({
      sysInfo,
      security,
      defender,
      autostart,
      backends,
      config,
      scan,
      hardware,
      packages,
      history,
      logs,
      profiles,
      offline,
      startup,
      backup,
      booting,
      bootProgress: bootTotal === 0 ? 1 : bootDone / bootTotal,
      refreshAll,
    }),
    [
      sysInfo,
      security,
      defender,
      autostart,
      backends,
      config,
      scan,
      hardware,
      packages,
      history,
      logs,
      profiles,
      offline,
      startup,
      backup,
      booting,
      bootDone,
      bootTotal,
      refreshAll,
    ],
  );

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}
