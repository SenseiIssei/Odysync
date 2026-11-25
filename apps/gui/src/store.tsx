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

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import * as api from "./api";
import { coalesce } from "./coalesce";
import { safeListen } from "./events";
import { errorText, isApplyRunning } from "./format";
import { scanHealth } from "./scan-health";
import {
  StoreContext,
  type AppStore,
  type ApplyOutcome,
  type ApplyState,
  type BackupData,
  type OfflineData,
  type Resource,
  type ScanProgress,
  type ScanResource,
  type SecurityResource,
} from "./store-context";
import type {
  ApplyProgress,
  ApplyRequest,
  AutostartConfig,
  BackendDto,
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
  ProfileDto,
  ScanProgressEvent,
  ScanReport,
  ScanResult,
  SectionProgress,
  StartupProgramDto,
  SystemInfoDto,
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

interface ResourceOptions {
  /** Route the loader through the shared concurrency queue. */
  heavy?: boolean;
  /**
   * Remember the last result across restarts, so a slow source (a full
   * update scan takes a minute or more) shows what it found last time at once
   * instead of an empty page, and is refreshed in the background.
   */
  persist?: boolean;
}

/** Bump when a persisted shape changes; older entries are then ignored. */
const CACHE_VERSION = 1;
/** Older than this, a remembered result says more about the past than now. */
const CACHE_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const cacheKey = (name: string) => `odysync.cache.${name}`;

function readCache<T>(name: string): { at: number; data: T } | null {
  try {
    const raw = localStorage.getItem(cacheKey(name));
    if (!raw) return null;
    const entry = JSON.parse(raw) as { v: number; at: number; data: T };
    if (entry.v !== CACHE_VERSION || Date.now() - entry.at > CACHE_MAX_AGE_MS) return null;
    return { at: entry.at, data: entry.data };
  } catch {
    return null;
  }
}

function writeCache<T>(name: string, data: T) {
  try {
    localStorage.setItem(cacheKey(name), JSON.stringify({ v: CACHE_VERSION, at: Date.now(), data }));
  } catch {
    // Storage full or blocked: the cache is a convenience, never a requirement.
  }
}

function useResource<T>(
  name: string,
  loader: () => Promise<T>,
  initial: T,
  options: ResourceOptions = {},
): Resource<T> {
  const persist = options.persist ?? false;
  // Read once, on the first render.
  const [cached] = useState(() => (persist ? readCache<T>(name) : null));
  const [data, setData] = useState<T>(cached ? cached.data : initial);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadedAt, setLoadedAt] = useState<number | null>(cached ? cached.at : null);
  const [fromCache, setFromCache] = useState(cached !== null);

  // Built once, from the first render's loader: every loader is a plain call
  // to the command layer and closes over nothing that changes.
  const [{ refresh, reload, mount }] = useState(() => {
    const heavy = options.heavy ?? false;
    let everLoaded = cached !== null;
    let mounted = true;
    const ifMounted = (update: () => void) => {
      if (mounted) update();
    };
    const loads = coalesce<T>({
      load: (begin) => {
        const fetch = () => {
          begin();
          return loader();
        };
        return heavy ? heavyQueue(fetch) : fetch();
      },
      // A quiet poll stays invisible once there is something to show.
      onRequest: (quiet) => {
        if (!quiet || !everLoaded) setLoading(true);
      },
      onValue: (value) =>
        ifMounted(() => {
          everLoaded = true;
          setData(value);
          setError(null);
          setLoadedAt(Date.now());
          setFromCache(false);
          if (persist) writeCache(name, value);
        }),
      onError: (e) =>
        ifMounted(() => {
          console.error(`[odysync] ${name} failed:`, e);
          setError(errorText(e));
        }),
      onEnd: () => ifMounted(() => setLoading(false)),
    });
    return {
      ...loads,
      /** An effect: nothing is stored or remembered after an unmount. */
      mount: () => {
        mounted = true;
        return () => {
          mounted = false;
        };
      },
    };
  });
  useEffect(mount, [mount]);

  // A local edit is not a load: when the data was fetched, and whether that
  // went well, stay what they were.
  const set = useCallback((value: T) => setData(value), []);

  return useMemo(
    () => ({ data, loading, error, loadedAt, refresh, reload, set, fromCache }),
    [data, loading, error, loadedAt, refresh, reload, set, fromCache],
  );
}

// ── Scan progress ────────────────────────────────────────────────────────────

/** What the sources of the running scan have reported, by backend id. */
type SourceStates = Map<string, ScanProgressEvent>;

function summarise(sources: SourceStates): ScanProgress | null {
  if (sources.size === 0) return null;
  const events = [...sources.values()];
  return {
    // Events from parallel sources can arrive out of order; counts only grow.
    done: Math.max(...events.map((e) => e.done)),
    total: Math.max(...events.map((e) => e.total)),
    found: events.reduce((n, e) => n + e.found, 0),
    waiting: events.filter((e) => e.state === "started").map((e) => e.name),
  };
}

// ── Installing ───────────────────────────────────────────────────────────────

const NOT_APPLYING = {
  running: false,
  dryRun: false,
  progress: null as ApplyProgress | null,
  cancelling: false,
};

/** `onChanged` runs after anything that may have changed what is installed. */
function useApply(onChanged: () => void): ApplyState {
  const [state, setState] = useState(NOT_APPLYING);
  const running = useRef(false);
  const cancelRequested = useRef(false);

  useEffect(
    () =>
      safeListen<ApplyProgress>("apply-progress", (e) =>
        setState((s) => (s.running ? { ...s, progress: e.payload } : s)),
      ),
    [],
  );

  // A run started from this window reports back through `run` below; the event
  // matters for one that was not.
  useEffect(
    () =>
      safeListen("apply-finished", () => {
        if (!running.current) onChanged();
      }),
    [onChanged],
  );

  const run = useCallback(
    async (request: ApplyRequest): Promise<ApplyOutcome> => {
      // The state is only visible after the next render; two clicks on two
      // pages can be faster than that.
      if (running.current) throw new Error("apply-running");
      running.current = true;
      cancelRequested.current = false;
      setState({ ...NOT_APPLYING, running: true, dryRun: request.dry_run });
      let changed = !request.dry_run;
      try {
        const result = await api.apply(request);
        return {
          result,
          cancelled: cancelRequested.current && result.entries.some((e) => e.status === "skipped"),
        };
      } catch (e) {
        if (isApplyRunning(e)) changed = false;
        throw e;
      } finally {
        running.current = false;
        setState(NOT_APPLYING);
        if (changed) onChanged();
      }
    },
    [onChanged],
  );

  const cancel = useCallback(async () => {
    if (!running.current) return;
    cancelRequested.current = true;
    setState((s) => ({ ...s, cancelling: true }));
    try {
      await api.cancelApply();
    } catch (e) {
      cancelRequested.current = false;
      setState((s) => ({ ...s, cancelling: false }));
      throw e;
    }
  }, []);

  return useMemo(() => ({ ...state, run, cancel }), [state, run, cancel]);
}

const EMPTY_OFFLINE: OfflineData = { status: null, entries: [] };
const EMPTY_BACKUP: BackupData = { backups: [], protectionEnabled: false };
const SCAN_ON_STARTUP_KEY = "odysync.securityScanOnStartup";

/** Defender state is one cheap CIM read; poll it often enough to feel live. */
const DEFENDER_POLL_MS = 60_000;

/** The full audit spawns many PowerShell queries; keep it well clear of idle. */
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
    { heavy: true, persist: true },
  );

  const config = useResource<Config | null>("config", () => api.getConfig(), null);

  const [scanSources, setScanSources] = useState<SourceStates>(() => new Map());
  const [scanCancelling, setScanCancelling] = useState(false);
  const scanBase = useResource<ScanResult | null>(
    "scan",
    async () => {
      setScanSources(new Map());
      try {
        return await api.scan();
      } finally {
        setScanCancelling(false);
      }
    },
    null,
    { heavy: true, persist: true },
  );
  useEffect(
    () =>
      safeListen<ScanProgressEvent>("scan-progress", (e) =>
        setScanSources((prev) => new Map(prev).set(e.payload.backend, e.payload)),
      ),
    [],
  );
  // The timer in the tray process: look again without making the window busy.
  const refreshQuietly = scanBase.refresh;
  useEffect(
    () => safeListen("background-scan", () => void refreshQuietly({ quiet: true })),
    [refreshQuietly],
  );
  const cancelScan = useCallback(async () => {
    setScanCancelling(true);
    try {
      await api.cancelScan();
    } catch (e) {
      setScanCancelling(false);
      throw e;
    }
  }, []);
  const scan: ScanResource = useMemo(
    () => ({
      ...scanBase,
      health: scanHealth(scanBase, backends.data),
      progress: scanBase.loading ? summarise(scanSources) : null,
      cancelling: scanCancelling,
      cancel: cancelScan,
    }),
    [scanBase, backends.data, scanSources, scanCancelling, cancelScan],
  );

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
    { heavy: true, persist: true },
  );

  const autostart = useResource<AutostartConfig | null>(
    "autostart",
    () => api.getAutostart(),
    null,
  );

  // Cleared when an audit starts, so the board never shows the last one's ticks.
  const [auditSections, setAuditSections] = useState<Record<string, SectionProgress>>({});
  const securityScan = useResource<ScanReport | null>(
    "security audit",
    () => {
      setAuditSections({});
      return api.securityScan();
    },
    null,
    { heavy: true, persist: true },
  );
  useEffect(
    () =>
      safeListen<SectionProgress>("security-progress", (e) =>
        setAuditSections((prev) => ({ ...prev, [e.payload.name]: e.payload })),
      ),
    [],
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

  const cleanup = useResource<CleanupItemDto[]>("cleanup", () => api.cleanupScan(), [], {
    heavy: true,
    persist: true,
  });
  const components = useResource<ComponentStatus[]>(
    "components",
    () => api.componentsStatus(),
    [],
    { heavy: true, persist: true },
  );
  const devices = useResource<DeviceProblem[]>("devices", () => api.deviceProblems(), [], {
    heavy: true,
    persist: true,
  });
  const driverBackups = useResource<DriverBackupDto[]>(
    "driverBackups",
    () => api.driverBackups(),
    [],
  );
  const health = useResource<HealthReportDto | null>(
    "hardware health",
    () => api.deviceHealth(),
    null,
    { heavy: true, persist: true },
  );

  // Whether the security audit runs as part of the startup load. It is the
  // slowest thing in the app, so it can be turned off, but it is on by default: a
  // machine that has already been compromised should be told, not asked.
  const [scanOnStartup, setScanOnStartupState] = useState(
    () => localStorage.getItem(SCAN_ON_STARTUP_KEY) !== "false",
  );
  const setScanOnStartup = useCallback((v: boolean) => {
    localStorage.setItem(SCAN_ON_STARTUP_KEY, String(v));
    setScanOnStartupState(v);
  }, []);

  // Everything that is loaded at startup. The order is the order of the heavy
  // queue: the update scan is what the user is waiting for and the slowest by
  // far, so it goes first and the rest shares the remaining slots.
  const startupLoads = useMemo(
    () => [
      scanBase.refresh,
      sysInfo.refresh,
      config.refresh,
      autostart.refresh,
      backends.refresh,
      history.refresh,
      logs.refresh,
      profiles.refresh,
      offline.refresh,
      hardware.refresh,
      defender.refresh,
      packages.refresh,
      startup.refresh,
      backup.refresh,
      driverBackups.refresh,
      components.refresh,
      devices.refresh,
      health.refresh,
      cleanup.refresh,
    ],
    [
      scanBase.refresh,
      sysInfo.refresh,
      config.refresh,
      autostart.refresh,
      backends.refresh,
      history.refresh,
      logs.refresh,
      profiles.refresh,
      offline.refresh,
      hardware.refresh,
      defender.refresh,
      packages.refresh,
      startup.refresh,
      backup.refresh,
      driverBackups.refresh,
      components.refresh,
      devices.refresh,
      health.refresh,
      cleanup.refresh,
    ],
  );

  const refreshSecurity = securityScan.refresh;
  const [booting, setBooting] = useState(true);
  const [bootDone, setBootDone] = useState(0);
  const bootTotal = startupLoads.length;

  // Background-load everything, once, at startup.
  //
  // The full audit is the heaviest thing the app does (about 15 PowerShell and
  // CIM calls) and the least time-critical. In the initial batch it slowed the
  // update scan the user is actually watching, because both fight for the same
  // heavy-queue slots. It starts once the rest of the boot has settled.
  const booted = useRef(false);
  useEffect(() => {
    if (booted.current) return;
    booted.current = true;

    const tasks = startupLoads.map((load) => load().finally(() => setBootDone((n) => n + 1)));
    void Promise.allSettled(tasks).then(() => {
      setBooting(false);
      if (scanOnStartup) void refreshSecurity();
    });
    // Runs once: `booted` guards it, whatever changes later.
  }, [startupLoads, scanOnStartup, refreshSecurity]);

  // A finished apply invalidates almost everything.
  const reloadScan = scanBase.reload;
  const reloadHistory = history.reload;
  const reloadPackages = packages.reload;
  const reloadBackups = backup.reload;
  const refreshLogs = logs.refresh;
  const onChanged = useCallback(() => {
    void reloadScan();
    void reloadHistory();
    void reloadPackages();
    void reloadBackups();
    void refreshLogs({ quiet: true });
  }, [reloadScan, reloadHistory, reloadPackages, reloadBackups, refreshLogs]);
  const apply = useApply(onChanged);

  // While an update is being installed a scan would look at a machine that is
  // changing under it; the one after the install is the one that counts.
  const applying = apply.running;
  const refreshScan = scanBase.refresh;
  const refreshAll = useCallback(async () => {
    const loads = startupLoads.filter((load) => !(applying && load === refreshScan));
    if (scanOnStartup) loads.push(refreshSecurity);
    await Promise.allSettled(loads.map((load) => load()));
  }, [startupLoads, applying, refreshScan, scanOnStartup, refreshSecurity]);

  const security: SecurityResource = useMemo(
    () => ({ ...securityScan, scanOnStartup, setScanOnStartup, sections: auditSections }),
    [securityScan, scanOnStartup, setScanOnStartup, auditSections],
  );

  // Keep the security view live rather than a snapshot from startup. Defender's
  // own state is cheap to read and changes on its own (a scan finishes, a
  // signature update lands, something gets quarantined), so it polls often; the
  // full audit shells out to WMI and CIM, so it refreshes far more slowly, and
  // not at all for someone who turned the startup audit off.
  //
  // Both are quiet: a poll nobody asked for must not turn Home into "Checking
  // your PC" every minute.
  const refreshDefender = defender.refresh;
  useEffect(() => {
    const timer = setInterval(() => void refreshDefender({ quiet: true }), DEFENDER_POLL_MS);
    return () => clearInterval(timer);
  }, [refreshDefender]);
  useEffect(() => {
    if (!scanOnStartup) return;
    const timer = setInterval(() => void refreshSecurity({ quiet: true }), SECURITY_POLL_MS);
    return () => clearInterval(timer);
  }, [scanOnStartup, refreshSecurity]);

  const value: AppStore = useMemo(
    () => ({
      sysInfo,
      security,
      defender,
      autostart,
      backends,
      config,
      scan,
      apply,
      hardware,
      packages,
      history,
      logs,
      profiles,
      offline,
      startup,
      backup,
      cleanup,
      components,
      devices,
      driverBackups,
      health,
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
      apply,
      hardware,
      packages,
      history,
      logs,
      profiles,
      offline,
      startup,
      backup,
      cleanup,
      components,
      devices,
      driverBackups,
      health,
      booting,
      bootDone,
      bootTotal,
      refreshAll,
    ],
  );

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}
