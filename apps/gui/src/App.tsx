import { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  RefreshCw,
  Download,
  Settings,
  Wrench,
  Calendar,
  Stethoscope,
  Shield,
  Sun,
  Moon,
  Package,
  AlertTriangle,
  CheckCircle,
  XCircle,
  Clock,
  HardDrive,
} from "lucide-react";
import "./App.css";
import type {
  ScanResult,
  UpdateDto,
  BackendDto,
  SystemInfoDto,
  ApplyResultDto,
  Config,
} from "./types";
import * as api from "./api";

type Tab = "updates" | "maintenance" | "schedule" | "settings";

export default function App() {
  const [tab, setTab] = useState<Tab>("updates");
  const [dark, setDark] = useState(
    () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true,
  );

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);

  useEffect(() => {
    const unlisten = listen("tray-scan", () => {
      setTab("updates");
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-950 text-gray-900 dark:text-gray-100 flex flex-col">
      <header className="flex items-center justify-between px-6 py-4 border-b border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900">
        <div className="flex items-center gap-3">
          <Shield className="w-7 h-7 text-accent" />
          <div>
            <h1 className="text-lg font-semibold">Sensei's Updater</h1>
            <p className="text-xs text-gray-500 dark:text-gray-400">
              Safe, verified updates for your system
            </p>
          </div>
        </div>
        <button
          onClick={() => setDark(!dark)}
          className="p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
          title={dark ? "Switch to light mode" : "Switch to dark mode"}
        >
          {dark ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
        </button>
      </header>

      <nav className="flex gap-1 px-6 py-2 border-b border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900">
        <TabButton active={tab === "updates"} onClick={() => setTab("updates")} icon={<Package className="w-4 h-4" />} label="Updates" />
        <TabButton active={tab === "maintenance"} onClick={() => setTab("maintenance")} icon={<Wrench className="w-4 h-4" />} label="Maintenance" />
        <TabButton active={tab === "schedule"} onClick={() => setTab("schedule")} icon={<Calendar className="w-4 h-4" />} label="Schedule" />
        <TabButton active={tab === "settings"} onClick={() => setTab("settings")} icon={<Settings className="w-4 h-4" />} label="Settings" />
      </nav>

      <main className="flex-1 overflow-y-auto p-6">
        {tab === "updates" && <UpdatesTab />}
        {tab === "maintenance" && <MaintenanceTab />}
        {tab === "schedule" && <ScheduleTab />}
        {tab === "settings" && <SettingsTab />}
      </main>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
        active
          ? "bg-accent text-white"
          : "text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800"
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

function UpdatesTab() {
  const [scanning, setScanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [result, setResult] = useState<ScanResult | null>(null);
  const [applyResult, setApplyResult] = useState<ApplyResultDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [dryRun, setDryRun] = useState(false);
  const [restorePoint, setRestorePoint] = useState(false);
  const [sysInfo, setSysInfo] = useState<SystemInfoDto | null>(null);

  useEffect(() => {
    api.getSystemInfo().then(setSysInfo).catch(() => {});
  }, []);

  const doScan = useCallback(async () => {
    setScanning(true);
    setError(null);
    setApplyResult(null);
    try {
      const r = await api.scan();
      setResult(r);
      setSelected(new Set(r.actionable.map((u) => u.id)));
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  }, []);

  const doApply = useCallback(async () => {
    if (!result) return;
    setApplying(true);
    setError(null);
    try {
      const updates = result.actionable.filter((u) => selected.has(u.id));
      const r = await api.apply({
        updates,
        dry_run: dryRun,
        restore_point: restorePoint,
      });
      setApplyResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  }, [result, selected, dryRun, restorePoint]);

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    if (!result) return;
    if (selected.size === result.actionable.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(result.actionable.map((u) => u.id)));
    }
  };

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      {sysInfo && (
        <div className="flex items-center gap-4 text-sm text-gray-500 dark:text-gray-400 bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 px-4 py-3">
          <span>OS: {sysInfo.os}</span>
          <span>v{sysInfo.version}</span>
          <span className={sysInfo.elevated ? "text-amber-600 dark:text-amber-400" : ""}>
            {sysInfo.elevated ? "Elevated" : "Unelevated"}
          </span>
        </div>
      )}

      <div className="flex items-center gap-3">
        <button
          onClick={doScan}
          disabled={scanning}
          className="flex items-center gap-2 px-4 py-2 rounded-lg bg-accent text-white font-medium text-sm hover:bg-accent-hover disabled:opacity-50 transition-colors"
        >
          <RefreshCw className={`w-4 h-4 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "Scanning..." : "Scan for Updates"}
        </button>

        {result && result.actionable.length > 0 && (
          <>
            <button
              onClick={doApply}
              disabled={applying || selected.size === 0}
              className="flex items-center gap-2 px-4 py-2 rounded-lg bg-green-600 text-white font-medium text-sm hover:bg-green-700 disabled:opacity-50 transition-colors"
            >
              <Download className={`w-4 h-4 ${applying ? "animate-pulse" : ""}`} />
              {applying
                ? "Applying..."
                : `Apply ${selected.size} Update${selected.size !== 1 ? "s" : ""}`}
            </button>

            <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
              <input type="checkbox" checked={dryRun} onChange={(e) => setDryRun(e.target.checked)} className="rounded" />
              Dry run
            </label>

            <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
              <input type="checkbox" checked={restorePoint} onChange={(e) => setRestorePoint(e.target.checked)} className="rounded" />
              Restore point
            </label>
          </>
        )}
      </div>

      {error && (
        <div className="flex items-center gap-2 text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-950/30 rounded-lg px-4 py-3 border border-red-200 dark:border-red-900">
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </div>
      )}

      {applyResult && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
          <h3 className="font-semibold text-sm">Apply Results</h3>
          <div className="flex gap-6 text-sm">
            <span className="text-green-600 dark:text-green-400">{applyResult.updated} updated</span>
            <span className="text-red-600 dark:text-red-400">{applyResult.failed} failed</span>
            <span className="text-gray-500">{applyResult.skipped} skipped</span>
            {applyResult.reboot_required && (
              <span className="text-amber-600 dark:text-amber-400 font-medium">Reboot required</span>
            )}
          </div>
          {applyResult.entries.length > 0 && (
            <div className="space-y-1">
              {applyResult.entries.map((e, i) => (
                <div key={i} className="flex items-center gap-2 text-sm py-1 border-t border-gray-100 dark:border-gray-800">
                  {e.outcome.includes("Updated") ? (
                    <CheckCircle className="w-4 h-4 text-green-500" />
                  ) : e.outcome.includes("Failed") ? (
                    <XCircle className="w-4 h-4 text-red-500" />
                  ) : (
                    <Clock className="w-4 h-4 text-gray-400" />
                  )}
                  <span className="flex-1">{e.name}</span>
                  <span className="text-gray-500 text-xs">{e.outcome}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {result && (
        <div className="space-y-2">
          {result.actionable.length > 0 && (
            <>
              <div className="flex items-center justify-between">
                <h3 className="font-semibold text-sm">
                  {result.actionable.length} update{result.actionable.length !== 1 ? "s" : ""} available
                </h3>
                <button onClick={toggleAll} className="text-xs text-accent hover:underline">
                  {selected.size === result.actionable.length ? "Deselect all" : "Select all"}
                </button>
              </div>
              {result.actionable.map((u) => (
                <UpdateCard key={u.id} update={u} checked={selected.has(u.id)} onToggle={() => toggleSelect(u.id)} />
              ))}
            </>
          )}

          {result.skipped.length > 0 && (
            <>
              <h3 className="font-semibold text-sm pt-4 text-gray-500">
                {result.skipped.length} skipped by policy
              </h3>
              {result.skipped.map((s) => (
                <SkippedCard key={s.id} skipped={s} />
              ))}
            </>
          )}

          {result.total === 0 && (
            <div className="text-center py-12 text-gray-400">
              <CheckCircle className="w-12 h-12 mx-auto mb-3 text-green-500" />
              <p className="text-sm">Everything is up to date!</p>
            </div>
          )}
        </div>
      )}

      {!result && !scanning && !error && (
        <div className="text-center py-12 text-gray-400">
          <Package className="w-12 h-12 mx-auto mb-3" />
          <p className="text-sm">Click "Scan for Updates" to check for available updates.</p>
        </div>
      )}
    </div>
  );
}

function UpdateCard({
  update,
  checked,
  onToggle,
}: {
  update: UpdateDto;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className="flex items-start gap-3 p-4 bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 cursor-pointer hover:border-accent transition-colors">
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-1 rounded" />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-sm truncate">{update.name}</span>
          <span className="text-xs px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-500">
            {update.backend}
          </span>
        </div>
        <div className="flex items-center gap-3 mt-1 text-xs text-gray-500 dark:text-gray-400">
          <span>
            {update.installed} {"->"} <span className="text-accent font-medium">{update.available}</span>
          </span>
          {update.size_bytes != null && update.size_bytes > 0 && (
            <span className="flex items-center gap-1">
              <HardDrive className="w-3 h-3" />
              {formatSize(update.size_bytes)}
            </span>
          )}
        </div>
      </div>
    </label>
  );
}

function SkippedCard({ skipped }: { skipped: { backend: string; id: string; name: string; reason: string } }) {
  return (
    <div className="flex items-start gap-3 p-3 bg-gray-50 dark:bg-gray-900/50 rounded-lg border border-gray-200 dark:border-gray-800">
      <Clock className="w-4 h-4 text-gray-400 mt-0.5 flex-shrink-0" />
      <div className="flex-1 min-w-0">
        <span className="text-sm font-medium">{skipped.name}</span>
        <p className="text-xs text-gray-500 mt-0.5">Skipped: {skipped.reason}</p>
      </div>
    </div>
  );
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function MaintenanceTab() {
  const [running, setRunning] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const actions = [
    { id: "temp_cleanup", label: "Temp Cleanup", icon: <Wrench className="w-5 h-5" />, desc: "Clean temporary files and folders" },
    { id: "clean_recycle_bin", label: "Empty Recycle Bin", icon: <HardDrive className="w-5 h-5" />, desc: "Permanently delete recycled files" },
    { id: "system_health", label: "System Health (DISM/SFC)", icon: <Stethoscope className="w-5 h-5" />, desc: "Scan and repair system file integrity" },
    { id: "startup_programs", label: "View Startup Programs", icon: <Package className="w-5 h-5" />, desc: "List programs that start with Windows" },
  ];

  const runAction = async (action: string) => {
    setRunning(action);
    setError(null);
    setResult(null);
    try {
      const r = await api.runMaintenance(action);
      setResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(null);
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <h2 className="text-lg font-semibold">System Maintenance</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400">
        These actions are not package updates — they clean and inspect your system.
      </p>

      <div className="grid gap-3">
        {actions.map((a) => (
          <div key={a.id} className="flex items-center gap-4 p-4 bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800">
            <div className="text-accent">{a.icon}</div>
            <div className="flex-1">
              <h3 className="font-medium text-sm">{a.label}</h3>
              <p className="text-xs text-gray-500 dark:text-gray-400">{a.desc}</p>
            </div>
            <button
              onClick={() => runAction(a.id)}
              disabled={running !== null}
              className="px-3 py-1.5 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-hover disabled:opacity-50 transition-colors"
            >
              {running === a.id ? "Running..." : "Run"}
            </button>
          </div>
        ))}
      </div>

      {result && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4">
          <h3 className="font-semibold text-sm mb-2">Result</h3>
          <pre className="text-xs whitespace-pre-wrap text-gray-600 dark:text-gray-400">{result}</pre>
        </div>
      )}

      {error && (
        <div className="flex items-center gap-2 text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-950/30 rounded-lg px-4 py-3 border border-red-200 dark:border-red-900">
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </div>
      )}
    </div>
  );
}

function ScheduleTab() {
  const [frequency, setFrequency] = useState("daily");
  const [time, setTime] = useState("09:00");
  const [taskName, setTaskName] = useState("SenseisUpdater");
  const [scheduled, setScheduled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api.checkSchedule(taskName).then(setScheduled).catch(() => {});
  }, [taskName]);

  const create = async () => {
    setError(null);
    setMessage(null);
    try {
      await api.createSchedule({ frequency, time, task_name: taskName });
      setScheduled(true);
      setMessage(`Scheduled "${taskName}" ${frequency} at ${time}.`);
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async () => {
    setError(null);
    setMessage(null);
    try {
      const existed = await api.removeSchedule(taskName);
      setScheduled(false);
      setMessage(existed ? `Removed "${taskName}".` : "No schedule existed.");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <h2 className="text-lg font-semibold">Scheduled Updates</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400">
        Automatically scan and apply updates on a schedule.
      </p>

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-4">
        <div>
          <label className="block text-sm font-medium mb-1">Frequency</label>
          <select
            value={frequency}
            onChange={(e) => setFrequency(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-sm"
          >
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
          </select>
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Time (HH:MM, 24h)</label>
          <input
            type="time"
            value={time}
            onChange={(e) => setTime(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-sm"
          />
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Task name</label>
          <input
            type="text"
            value={taskName}
            onChange={(e) => setTaskName(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-sm"
          />
        </div>

        <div className="flex items-center gap-2 text-sm">
          <span
            className={`px-2 py-0.5 rounded-full text-xs font-medium ${
              scheduled
                ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
                : "bg-gray-100 text-gray-500 dark:bg-gray-800"
            }`}
          >
            {scheduled ? "Scheduled" : "Not scheduled"}
          </span>
        </div>

        <div className="flex gap-3">
          <button
            onClick={create}
            className="px-4 py-2 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-hover transition-colors"
          >
            Create Schedule
          </button>
          <button
            onClick={remove}
            className="px-4 py-2 rounded-lg border border-gray-200 dark:border-gray-700 text-sm font-medium hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
          >
            Remove
          </button>
        </div>
      </div>

      {message && (
        <div className="text-sm text-green-600 dark:text-green-400 bg-green-50 dark:bg-green-950/30 rounded-lg px-4 py-3">
          {message}
        </div>
      )}
      {error && (
        <div className="flex items-center gap-2 text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-950/30 rounded-lg px-4 py-3 border border-red-200 dark:border-red-900">
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </div>
      )}
    </div>
  );
}

function SettingsTab() {
  const [config, setConfig] = useState<Config | null>(null);
  const [backends, setBackends] = useState<BackendDto[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api.getConfig().then(setConfig).catch(setError);
    api.listBackends().then(setBackends).catch(() => {});
  }, []);

  const updateConfig = (patch: Partial<Config>) => {
    if (!config) return;
    setConfig({ ...config, ...patch });
    setSaved(false);
  };

  const updatePolicy = (patch: Partial<Config["policy"]>) => {
    if (!config) return;
    setConfig({ ...config, policy: { ...config.policy, ...patch } });
    setSaved(false);
  };

  const save = async () => {
    if (!config) return;
    setError(null);
    try {
      await api.saveConfig(config);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  if (!config) return <div className="text-center py-12 text-gray-400">Loading config...</div>;

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <h2 className="text-lg font-semibold">Settings</h2>

      {backends.length > 0 && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4">
          <h3 className="font-medium text-sm mb-3">Backends</h3>
          <div className="space-y-2">
            {backends.map((b) => (
              <div key={b.kind} className="flex items-center justify-between text-sm">
                <span>{b.name}</span>
                <span
                  className={`text-xs px-2 py-0.5 rounded-full ${
                    b.available
                      ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
                      : "bg-gray-100 text-gray-400 dark:bg-gray-800"
                  }`}
                >
                  {b.available ? "Available" : "Unavailable"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-4">
        <h3 className="font-medium text-sm">Policy</h3>

        <Toggle
          label="Stable releases only"
          description="Block pre-release versions"
          checked={config.policy.stable_only}
          onChange={(v) => updatePolicy({ stable_only: v })}
        />
        <Toggle
          label="Require known versions"
          description="Refuse updates with unparseable version strings"
          checked={config.policy.require_known_versions}
          onChange={(v) => updatePolicy({ require_known_versions: v })}
        />
        <Toggle
          label="Allow elevated installs"
          description="Permit updates that require admin/root"
          checked={config.policy.elevated}
          onChange={(v) => updatePolicy({ elevated: v })}
        />
        <Toggle
          label="Create restore point before apply"
          description="Windows only — creates a system restore point"
          checked={config.restore_point}
          onChange={(v) => updateConfig({ restore_point: v })}
        />
      </div>

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4">
        <h3 className="font-medium text-sm mb-2">Excluded packages</h3>
        <textarea
          value={config.policy.exclude.join("\n")}
          onChange={(e) =>
            updatePolicy({
              exclude: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean),
            })
          }
          rows={4}
          className="w-full px-3 py-2 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-sm font-mono"
          placeholder="One package ID per line"
        />
      </div>

      <div className="flex items-center gap-3">
        <button
          onClick={save}
          className="px-4 py-2 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-hover transition-colors"
        >
          Save Settings
        </button>
        {saved && (
          <span className="text-sm text-green-600 dark:text-green-400 flex items-center gap-1">
            <CheckCircle className="w-4 h-4" />
            Saved
          </span>
        )}
      </div>

      {error && (
        <div className="flex items-center gap-2 text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-950/30 rounded-lg px-4 py-3 border border-red-200 dark:border-red-900">
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </div>
      )}
    </div>
  );
}

function Toggle({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center justify-between cursor-pointer">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-gray-500 dark:text-gray-400">{description}</div>
      </div>
      <button
        onClick={() => onChange(!checked)}
        className={`relative w-11 h-6 rounded-full transition-colors ${
          checked ? "bg-accent" : "bg-gray-300 dark:bg-gray-700"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform ${
            checked ? "translate-x-5" : ""
          }`}
        />
      </button>
    </label>
  );
}
