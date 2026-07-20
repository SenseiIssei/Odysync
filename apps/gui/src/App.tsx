import { useState, useEffect, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { motion, AnimatePresence } from "framer-motion";
import {
  RefreshCw,
  Download,
  Settings,
  Wrench,
  Calendar,
  Stethoscope,
  Shield,
  Package,
  AlertTriangle,
  CheckCircle,
  XCircle,
  Clock,
  HardDrive,
  Minus,
  X,
  Activity,
  History,
  Cpu,
  ScrollText,
  Layers,
  WifiOff,
  Info,
  Zap,
} from "lucide-react";
import "./App.css";
import type {
  ScanResult,
  UpdateDto,
  BackendDto,
  SystemInfoDto,
  ApplyResultDto,
  Config,
  HistoryEntryDto,
  HardwareInfoDto,
  InstalledPackageDto,
  LogEntryDto,
  ProfileDto,
  OfflineCacheStatusDto,
} from "./types";
import * as api from "./api";

type Tab =
  | "updates"
  | "maintenance"
  | "schedule"
  | "settings"
  | "dashboard"
  | "history"
  | "packages"
  | "hardware"
  | "logs"
  | "profiles"
  | "offline"
  | "about";

interface NavItem {
  id: Tab;
  label: string;
  icon: React.ReactNode;
  group: string;
}

const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Dashboard", icon: <Activity className="w-4 h-4" />, group: "Overview" },
  { id: "updates", label: "Updates", icon: <Package className="w-4 h-4" />, group: "Overview" },
  { id: "history", label: "History", icon: <History className="w-4 h-4" />, group: "Overview" },
  { id: "packages", label: "Packages", icon: <Layers className="w-4 h-4" />, group: "Overview" },
  { id: "hardware", label: "Hardware", icon: <Cpu className="w-4 h-4" />, group: "System" },
  { id: "maintenance", label: "Maintenance", icon: <Wrench className="w-4 h-4" />, group: "System" },
  { id: "logs", label: "Logs", icon: <ScrollText className="w-4 h-4" />, group: "System" },
  { id: "schedule", label: "Schedule", icon: <Calendar className="w-4 h-4" />, group: "Automation" },
  { id: "profiles", label: "Profiles", icon: <Layers className="w-4 h-4" />, group: "Automation" },
  { id: "offline", label: "Offline", icon: <WifiOff className="w-4 h-4" />, group: "Automation" },
  { id: "settings", label: "Settings", icon: <Settings className="w-4 h-4" />, group: "Config" },
  { id: "about", label: "About", icon: <Info className="w-4 h-4" />, group: "Config" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const appWindow = useRef(getCurrentWindow());

  useEffect(() => {
    const unlisten = listen("tray-scan", () => {
      setTab("updates");
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const minimizeWindow = () => appWindow.current.minimize();
  const closeWindow = () => appWindow.current.hide();

  const navGroups = [...new Set(NAV_ITEMS.map((n) => n.group))];

  return (
    <div className="app-window h-screen flex flex-col text-cyber-text bg-cyber-bg grid-bg">
      {/* Custom Titlebar */}
      <div className="titlebar flex items-center justify-between px-4 py-2.5 border-b border-cyber-border bg-cyber-surface/80 backdrop-blur-sm">
        <div className="flex items-center gap-3">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="text-cyber-text-dim hover:text-accent transition-colors p-1"
            title="Toggle sidebar"
          >
            <motion.div animate={{ rotate: sidebarOpen ? 0 : 180 }}>
              <Layers className="w-4 h-4" />
            </motion.div>
          </button>
          <div className="flex items-center gap-2">
            <Shield className="w-5 h-5 text-accent text-glow-cyan" />
            <span className="text-sm font-bold tracking-wide">ODYSYNC</span>
            <span className="text-xs text-cyber-text-faint">v2.0</span>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={minimizeWindow}
            className="p-1.5 rounded hover:bg-cyber-surface-2 text-cyber-text-dim hover:text-accent transition-all"
            title="Minimize"
          >
            <Minus className="w-4 h-4" />
          </button>
          <button
            onClick={closeWindow}
            className="p-1.5 rounded hover:bg-danger/20 text-cyber-text-dim hover:text-danger transition-all"
            title="Close to tray"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Main layout */}
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <AnimatePresence mode="sync">
          {sidebarOpen && (
            <motion.nav
              initial={{ width: 0, opacity: 0 }}
              animate={{ width: 200, opacity: 1 }}
              exit={{ width: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: "easeOut" }}
              className="border-r border-cyber-border bg-cyber-surface/50 overflow-hidden flex-shrink-0"
            >
              <div className="w-[200px] py-3 overflow-y-auto h-full">
                {navGroups.map((group) => (
                  <div key={group} className="mb-3">
                    <div className="px-4 mb-1.5 text-[10px] uppercase tracking-widest text-cyber-text-faint font-bold">
                      {group}
                    </div>
                    {NAV_ITEMS.filter((n) => n.group === group).map((item) => (
                      <button
                        key={item.id}
                        onClick={() => setTab(item.id)}
                        className={`relative w-full flex items-center gap-3 px-4 py-2 text-sm transition-all ${
                          tab === item.id
                            ? "text-accent text-glow-cyan"
                            : "text-cyber-text-dim hover:text-cyber-text hover:bg-cyber-surface-2"
                        }`}
                      >
                        {tab === item.id && (
                          <motion.div
                            layoutId="sidebar-active"
                            className="absolute left-0 top-0 bottom-0 w-[2px] bg-accent glow-cyan"
                          />
                        )}
                        {item.icon}
                        {item.label}
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </motion.nav>
          )}
        </AnimatePresence>

        {/* Content area */}
        <main className="flex-1 overflow-y-auto">
          <AnimatePresence mode="wait">
            <motion.div
              key={tab}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.2, ease: "easeOut" }}
              className="p-6"
            >
              {tab === "dashboard" && <DashboardTab onNavigate={setTab} />}
              {tab === "updates" && <UpdatesTab />}
              {tab === "maintenance" && <MaintenanceTab />}
              {tab === "schedule" && <ScheduleTab />}
              {tab === "settings" && <SettingsTab />}
              {tab === "history" && <HistoryTab />}
              {tab === "packages" && <PackagesTab />}
              {tab === "hardware" && <HardwareTab />}
              {tab === "logs" && <LogsTab />}
              {tab === "profiles" && <ProfilesTab />}
              {tab === "offline" && <OfflineTab />}
              {tab === "about" && <AboutTab />}
            </motion.div>
          </AnimatePresence>
        </main>
      </div>
    </div>
  );
}

function AboutTab() {
  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <div className="text-center py-8">
        <Shield className="w-16 h-16 mx-auto mb-4 text-accent text-glow-cyan" />
        <h2 className="text-2xl font-bold text-glow-cyan">Odysync</h2>
        <p className="text-sm text-cyber-text-dim mt-1">v2.0.0-alpha.1</p>
        <p className="text-xs text-cyber-text-faint mt-2">Safe, verified updates for your system</p>
      </div>
      <div className="gradient-border p-4 space-y-2">
        <h3 className="text-sm font-bold text-accent">Changelog</h3>
        <div className="text-xs text-cyber-text-dim space-y-1 font-mono">
          <div><span className="text-success">+</span> 30+ backends (winget, chocolatey, scoop, pip, cargo, npm...)</div>
          <div><span className="text-success">+</span> Cyberpunk UI with Framer Motion animations</div>
          <div><span className="text-success">+</span> Hybrid version discovery (online registry + vendor scraping)</div>
          <div><span className="text-success">+</span> Offline mode with driver cache</div>
          <div><span className="text-success">+</span> Error retry with exponential backoff</div>
          <div><span className="text-success">+</span> PackageId validation & output limits</div>
          <div><span className="text-success">+</span> Scan caching with TTL</div>
        </div>
      </div>
      <div className="text-center text-xs text-cyber-text-faint">
        <p>Built with Tauri, React, Rust, and lots of caffeine.</p>
        <p className="mt-1">github.com/SenseiIssei/Odysync</p>
      </div>
    </div>
  );
}

function DashboardTab({ onNavigate }: { onNavigate: (tab: Tab) => void }) {
  const [sysInfo, setSysInfo] = useState<SystemInfoDto | null>(null);
  const [backends, setBackends] = useState<BackendDto[]>([]);
  const [scanResult, setScanResult] = useState<ScanResult | null>(null);
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    api.getSystemInfo().then(setSysInfo).catch(() => {});
    api.listBackends().then(setBackends).catch(() => {});
  }, []);

  const quickScan = async () => {
    setScanning(true);
    try {
      const r = await api.scan();
      setScanResult(r);
    } catch (e) {
      console.error(e);
    } finally {
      setScanning(false);
    }
  };

  const availableCount = backends.filter((b) => b.available).length;
  const updateCount = scanResult?.actionable.length ?? 0;

  const stats = [
    { label: "Updates Available", value: scanning ? "..." : String(updateCount), icon: <Package className="w-5 h-5" />, color: "text-accent", glow: "glow-cyan" },
    { label: "Backends Active", value: `${availableCount}/${backends.length}`, icon: <Zap className="w-5 h-5" />, color: "text-purple-neon", glow: "glow-purple" },
    { label: "System", value: sysInfo?.os ?? "Detecting...", icon: <Cpu className="w-5 h-5" />, color: "text-success", glow: "glow-green" },
    { label: "Elevation", value: sysInfo?.elevated ? "Elevated" : "User", icon: <Shield className="w-5 h-5" />, color: sysInfo?.elevated ? "text-warning" : "text-cyber-text-dim", glow: "" },
  ];

  return (
    <div className="max-w-4xl mx-auto space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-glow-cyan">Dashboard</h1>
        <p className="text-sm text-cyber-text-dim mt-1">System overview at a glance</p>
      </div>

      {/* Stats grid */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {stats.map((stat, i) => (
          <motion.div
            key={stat.label}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: i * 0.05 }}
            className={`relative p-4 rounded-xl bg-cyber-surface border border-cyber-border ${stat.glow}`}
          >
            <div className={`${stat.color} mb-2`}>{stat.icon}</div>
            <div className="text-xl font-bold">{stat.value}</div>
            <div className="text-xs text-cyber-text-dim mt-0.5">{stat.label}</div>
          </motion.div>
        ))}
      </div>

      {/* Quick actions */}
      <div className="flex gap-3">
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={quickScan}
          disabled={scanning}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-accent/10 border border-accent/30 text-accent font-medium text-sm hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
        >
          <RefreshCw className={`w-4 h-4 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "Scanning..." : "Quick Scan"}
        </motion.button>
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={() => onNavigate("updates")}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-cyber-surface border border-cyber-border text-cyber-text font-medium text-sm hover:border-accent/30 transition-all"
        >
          <Package className="w-4 h-4" />
          View Updates
        </motion.button>
      </div>

      {/* Backend status */}
      <div className="rounded-xl bg-cyber-surface border border-cyber-border p-4">
        <h3 className="text-sm font-bold mb-3 flex items-center gap-2">
          <Zap className="w-4 h-4 text-accent" />
          Backend Status
        </h3>
        <div className="grid grid-cols-2 md:grid-cols-3 gap-2">
          {backends.length === 0 && (
            <div className="text-xs text-cyber-text-faint col-span-full py-4 text-center">Loading backends...</div>
          )}
          {backends.map((b, i) => (
            <motion.div
              key={b.kind}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: i * 0.02 }}
              className="flex items-center justify-between text-xs px-3 py-2 rounded-lg bg-cyber-bg/50 border border-cyber-border"
            >
              <span className="truncate text-cyber-text-dim">{b.name}</span>
              <span className={`w-2 h-2 rounded-full ${b.available ? "bg-success glow-green" : "bg-cyber-text-faint"}`} />
            </motion.div>
          ))}
        </div>
      </div>
    </div>
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
  const [progress, setProgress] = useState<{ package: string; current: number; total: number } | null>(null);

  useEffect(() => {
    api.getSystemInfo().then(setSysInfo).catch(() => {});
    const unlisten = listen<{ package: string; current: number; total: number }>("apply-progress", (e) => {
      setProgress(e.payload);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  const doScan = useCallback(async () => {
    setScanning(true);
    setError(null);
    setApplyResult(null);
    setProgress(null);
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
    setProgress(null);
    try {
      const updates = result.actionable.filter((u) => selected.has(u.id));
      const r = await api.apply({ updates, dry_run: dryRun, restore_point: restorePoint });
      setApplyResult(r);
      setProgress(null);
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
        <div className="flex items-center gap-4 text-xs text-cyber-text-dim bg-cyber-surface rounded-lg border border-cyber-border px-4 py-3">
          <span className="text-accent">{sysInfo.os}</span>
          <span>v{sysInfo.version}</span>
          <span className={sysInfo.elevated ? "text-warning" : "text-cyber-text-faint"}>
            {sysInfo.elevated ? "Elevated" : "Unelevated"}
          </span>
        </div>
      )}

      <div className="flex items-center gap-3 flex-wrap">
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={doScan}
          disabled={scanning}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-accent/10 border border-accent/30 text-accent font-medium text-sm hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
        >
          <RefreshCw className={`w-4 h-4 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "Scanning..." : "Scan for Updates"}
        </motion.button>

        {result && result.actionable.length > 0 && (
          <>
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={doApply}
              disabled={applying || selected.size === 0}
              className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-success/10 border border-success/30 text-success font-medium text-sm hover:bg-success/20 disabled:opacity-50 transition-all glow-green"
            >
              <Download className={`w-4 h-4 ${applying ? "animate-pulse" : ""}`} />
              {applying ? "Applying..." : `Apply ${selected.size} Update${selected.size !== 1 ? "s" : ""}`}
            </motion.button>

            <label className="flex items-center gap-2 text-xs text-cyber-text-dim cursor-pointer">
              <input type="checkbox" checked={dryRun} onChange={(e) => setDryRun(e.target.checked)} className="accent-accent" />
              Dry run
            </label>

            <label className="flex items-center gap-2 text-xs text-cyber-text-dim cursor-pointer">
              <input type="checkbox" checked={restorePoint} onChange={(e) => setRestorePoint(e.target.checked)} className="accent-accent" />
              Restore point
            </label>
          </>
        )}
      </div>

      {/* Progress bar */}
      {applying && progress && (
        <motion.div
          initial={{ opacity: 0, y: -5 }}
          animate={{ opacity: 1, y: 0 }}
          className="rounded-lg border border-accent/30 bg-accent/5 p-4 scan-overlay"
        >
          <div className="flex items-center justify-between text-xs mb-2">
            <span className="text-accent">{progress.package}</span>
            <span className="text-cyber-text-dim">{progress.current}/{progress.total}</span>
          </div>
          <div className="h-1.5 bg-cyber-bg rounded-full overflow-hidden">
            <motion.div
              className="h-full bg-accent glow-cyan"
              animate={{ width: `${(progress.current / progress.total) * 100}%` }}
              transition={{ duration: 0.3 }}
            />
          </div>
        </motion.div>
      )}

      {error && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="flex items-center gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30 glow-red"
        >
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </motion.div>
      )}

      {applyResult && (
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-3"
        >
          <h3 className="font-bold text-sm text-accent">Apply Results</h3>
          <div className="flex gap-6 text-xs">
            <span className="text-success">{applyResult.updated} updated</span>
            <span className="text-danger">{applyResult.failed} failed</span>
            <span className="text-cyber-text-dim">{applyResult.skipped} skipped</span>
            {applyResult.reboot_required && (
              <span className="text-warning font-medium">Reboot required</span>
            )}
          </div>
          {applyResult.entries.length > 0 && (
            <div className="space-y-1">
              {applyResult.entries.map((e, i) => (
                <div key={i} className="flex items-center gap-2 text-xs py-1 border-t border-cyber-border">
                  {e.outcome.includes("Updated") ? (
                    <CheckCircle className="w-4 h-4 text-success" />
                  ) : e.outcome.includes("Failed") ? (
                    <XCircle className="w-4 h-4 text-danger" />
                  ) : (
                    <Clock className="w-4 h-4 text-cyber-text-faint" />
                  )}
                  <span className="flex-1 text-cyber-text">{e.name}</span>
                  <span className="text-cyber-text-faint">{e.outcome}</span>
                </div>
              ))}
            </div>
          )}
        </motion.div>
      )}

      {result && (
        <div className="space-y-2">
          {result.actionable.length > 0 && (
            <>
              <div className="flex items-center justify-between">
                <h3 className="font-bold text-sm">
                  {result.actionable.length} update{result.actionable.length !== 1 ? "s" : ""} available
                </h3>
                <button onClick={toggleAll} className="text-xs text-accent hover:underline">
                  {selected.size === result.actionable.length ? "Deselect all" : "Select all"}
                </button>
              </div>
              {result.actionable.map((u, i) => (
                <UpdateCard key={u.id} update={u} checked={selected.has(u.id)} onToggle={() => toggleSelect(u.id)} index={i} />
              ))}
            </>
          )}

          {result.skipped.length > 0 && (
            <>
              <h3 className="font-bold text-sm pt-4 text-cyber-text-dim">
                {result.skipped.length} skipped by policy
              </h3>
              {result.skipped.map((s) => (
                <SkippedCard key={s.id} skipped={s} />
              ))}
            </>
          )}

          {result.total === 0 && (
            <div className="text-center py-12">
              <CheckCircle className="w-12 h-12 mx-auto mb-3 text-success glow-green" />
              <p className="text-sm text-cyber-text-dim">Everything is up to date!</p>
            </div>
          )}
        </div>
      )}

      {!result && !scanning && !error && (
        <div className="text-center py-12">
          <Package className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
          <p className="text-sm text-cyber-text-dim">Click "Scan for Updates" to check for available updates.</p>
        </div>
      )}
    </div>
  );
}

function UpdateCard({ update, checked, onToggle, index }: { update: UpdateDto; checked: boolean; onToggle: () => void; index: number }) {
  return (
    <motion.label
      initial={{ opacity: 0, x: -10 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ delay: index * 0.03 }}
      whileHover={{ scale: 1.01 }}
      className={`flex items-start gap-3 p-4 rounded-lg border cursor-pointer transition-all ${
        checked
          ? "bg-accent/5 border-accent/30 glow-cyan"
          : "bg-cyber-surface border-cyber-border hover:border-cyber-border-bright"
      }`}
    >
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-1 accent-accent" />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-sm truncate">{update.name}</span>
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono">
            {update.backend}
          </span>
        </div>
        <div className="flex items-center gap-3 mt-1 text-xs text-cyber-text-dim">
          <span className="font-mono">
            {update.installed} <span className="text-cyber-text-faint">-&gt;</span> <span className="text-accent font-medium">{update.available}</span>
          </span>
          {update.size_bytes != null && update.size_bytes > 0 && (
            <span className="flex items-center gap-1">
              <HardDrive className="w-3 h-3" />
              {formatSize(update.size_bytes)}
            </span>
          )}
        </div>
      </div>
    </motion.label>
  );
}

function SkippedCard({ skipped }: { skipped: { backend: string; id: string; name: string; reason: string } }) {
  return (
    <div className="flex items-start gap-3 p-3 bg-cyber-surface/50 rounded-lg border border-cyber-border">
      <Clock className="w-4 h-4 text-cyber-text-faint mt-0.5 flex-shrink-0" />
      <div className="flex-1 min-w-0">
        <span className="text-sm font-medium text-cyber-text-dim">{skipped.name}</span>
        <p className="text-xs text-cyber-text-faint mt-0.5">Skipped: {skipped.reason}</p>
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
      <h1 className="text-2xl font-bold text-glow-cyan">System Maintenance</h1>
      <p className="text-sm text-cyber-text-dim">
        These actions are not package updates — they clean and inspect your system.
      </p>

      <div className="grid gap-3">
        {actions.map((a, i) => (
          <motion.div
            key={a.id}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: i * 0.05 }}
            className="flex items-center gap-4 p-4 rounded-lg bg-cyber-surface border border-cyber-border hover:border-accent/20 transition-all"
          >
            <div className="text-accent">{a.icon}</div>
            <div className="flex-1">
              <h3 className="font-medium text-sm">{a.label}</h3>
              <p className="text-xs text-cyber-text-dim">{a.desc}</p>
            </div>
            <motion.button
              whileHover={{ scale: 1.05 }}
              whileTap={{ scale: 0.95 }}
              onClick={() => runAction(a.id)}
              disabled={running !== null}
              className="px-4 py-1.5 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all"
            >
              {running === a.id ? (
                <RefreshCw className="w-4 h-4 animate-spin" />
              ) : "Run"}
            </motion.button>
          </motion.div>
        ))}
      </div>

      {result && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="rounded-lg border border-cyber-border bg-cyber-surface p-4"
        >
          <h3 className="font-bold text-sm mb-2 text-accent">Result</h3>
          <pre className="text-xs whitespace-pre-wrap text-cyber-text-dim font-mono">{result}</pre>
        </motion.div>
      )}

      {error && (
        <div className="flex items-center gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30 glow-red">
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
  const [taskName, setTaskName] = useState("Odysync");
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
      <h1 className="text-2xl font-bold text-glow-cyan">Scheduled Updates</h1>
      <p className="text-sm text-cyber-text-dim">
        Automatically scan and apply updates on a schedule.
      </p>

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-4">
        <div>
          <label className="block text-sm font-medium mb-1">Frequency</label>
          <select
            value={frequency}
            onChange={(e) => setFrequency(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
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
            className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
          />
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Task name</label>
          <input
            type="text"
            value={taskName}
            onChange={(e) => setTaskName(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
          />
        </div>

        <div className="flex items-center gap-2 text-sm">
          <span
            className={`px-2 py-0.5 rounded-full text-xs font-medium ${
              scheduled
                ? "bg-success/20 text-success glow-green"
                : "bg-cyber-bg text-cyber-text-faint"
            }`}
          >
            {scheduled ? "Scheduled" : "Not scheduled"}
          </span>
        </div>

        <div className="flex gap-3">
          <motion.button
            whileHover={{ scale: 1.02 }}
            whileTap={{ scale: 0.98 }}
            onClick={create}
            className="px-5 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 transition-all glow-cyan"
          >
            Create Schedule
          </motion.button>
          <motion.button
            whileHover={{ scale: 1.02 }}
            whileTap={{ scale: 0.98 }}
            onClick={remove}
            className="px-5 py-2 rounded-lg border border-cyber-border text-sm font-medium hover:bg-cyber-surface-2 transition-all"
          >
            Remove
          </motion.button>
        </div>
      </div>

      {message && (
        <div className="text-sm text-success bg-success/5 rounded-lg px-4 py-3 border border-success/30">
          {message}
        </div>
      )}
      {error && (
        <div className="flex items-center gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30 glow-red">
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

  if (!config) return <div className="text-center py-12 text-cyber-text-dim">Loading config...</div>;

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <h1 className="text-2xl font-bold text-glow-cyan">Settings</h1>

      {backends.length > 0 && (
        <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4">
          <h3 className="font-bold text-sm mb-3 text-accent">Backends</h3>
          <div className="space-y-1.5">
            {backends.map((b) => (
              <div key={b.kind} className="flex items-center justify-between text-xs">
                <span className="text-cyber-text-dim">{b.name}</span>
                <span
                  className={`px-2 py-0.5 rounded-full text-[10px] font-medium ${
                    b.available
                      ? "bg-success/20 text-success"
                      : "bg-cyber-bg text-cyber-text-faint"
                  }`}
                >
                  {b.available ? "Available" : "Unavailable"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-4">
        <h3 className="font-bold text-sm text-accent">Policy</h3>

        <Toggle label="Stable releases only" description="Block pre-release versions" checked={config.policy.stable_only} onChange={(v) => updatePolicy({ stable_only: v })} />
        <Toggle label="Require known versions" description="Refuse updates with unparseable version strings" checked={config.policy.require_known_versions} onChange={(v) => updatePolicy({ require_known_versions: v })} />
        <Toggle label="Allow elevated installs" description="Permit updates that require admin/root" checked={config.policy.elevated} onChange={(v) => updatePolicy({ elevated: v })} />
        <Toggle label="Create restore point before apply" description="Windows only — creates a system restore point" checked={config.restore_point} onChange={(v) => updateConfig({ restore_point: v })} />
      </div>

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-4">
        <h3 className="font-bold text-sm text-accent">Automation</h3>

        <Toggle label="Auto-apply updates" description="Automatically apply all updates after scanning (dangerous)" checked={config.auto_apply} onChange={(v) => updateConfig({ auto_apply: v })} />
        <Toggle label="Desktop notifications" description="Show notifications for scan results and apply completion" checked={config.notifications} onChange={(v) => updateConfig({ notifications: v })} />

        <div>
          <label className="block text-sm font-medium mb-1">Scan interval (hours)</label>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={0}
              max={168}
              value={config.scan_interval_hours}
              onChange={(e) => updateConfig({ scan_interval_hours: parseInt(e.target.value) || 0 })}
              className="w-24 px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            />
            <span className="text-xs text-cyber-text-dim">0 = manual only</span>
          </div>
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Max retries</label>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={0}
              max={10}
              value={config.max_retries}
              onChange={(e) => updateConfig({ max_retries: parseInt(e.target.value) || 0 })}
              className="w-24 px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            />
            <span className="text-xs text-cyber-text-dim">Retry failed updates up to N times</span>
          </div>
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Backend timeout (seconds)</label>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={10}
              max={600}
              value={config.backend_timeout_secs}
              onChange={(e) => updateConfig({ backend_timeout_secs: parseInt(e.target.value) || 120 })}
              className="w-24 px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            />
            <span className="text-xs text-cyber-text-dim">Timeout for individual backend operations</span>
          </div>
        </div>
      </div>

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-4">
        <h3 className="font-bold text-sm text-accent">Network</h3>

        <div>
          <label className="block text-sm font-medium mb-1">HTTP proxy URL</label>
          <input
            type="text"
            value={config.proxy_url ?? ""}
            onChange={(e) => updateConfig({ proxy_url: e.target.value || null })}
            className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            placeholder="http://proxy.example.com:8080"
          />
          <p className="text-xs text-cyber-text-dim mt-1">Used for web crawlers and registry requests. Leave empty for direct connection.</p>
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">Concurrency</label>
          <div className="flex items-center gap-3">
            <input
              type="range"
              min={1}
              max={16}
              value={config.concurrency}
              onChange={(e) => updateConfig({ concurrency: parseInt(e.target.value) || 4 })}
              className="flex-1 accent-cyan-400"
            />
            <span className="text-sm font-mono w-8 text-center text-accent">{config.concurrency}</span>
            <span className="text-xs text-cyber-text-dim">Max concurrent backends</span>
          </div>
        </div>
      </div>

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4">
        <h3 className="font-bold text-sm mb-2 text-accent">Excluded packages</h3>
        <textarea
          value={config.policy.exclude.join("\n")}
          onChange={(e) =>
            updatePolicy({
              exclude: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean),
            })
          }
          rows={4}
          className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm font-mono focus:border-accent"
          placeholder="One package ID per line"
        />
      </div>

      <div className="flex items-center gap-3">
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={save}
          className="px-5 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 transition-all glow-cyan"
        >
          Save Settings
        </motion.button>
        {saved && (
          <motion.span
            initial={{ opacity: 0, x: -5 }}
            animate={{ opacity: 1, x: 0 }}
            className="text-sm text-success flex items-center gap-1"
          >
            <CheckCircle className="w-4 h-4" />
            Saved
          </motion.span>
        )}
      </div>

      {error && (
        <div className="flex items-center gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30 glow-red">
          <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          {error}
        </div>
      )}
    </div>
  );
}

function Toggle({ label, description, checked, onChange }: { label: string; description: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex items-center justify-between cursor-pointer">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-cyber-text-dim">{description}</div>
      </div>
      <button
        onClick={() => onChange(!checked)}
        className={`relative w-11 h-6 rounded-full transition-all ${
          checked ? "bg-accent/30 glow-cyan" : "bg-cyber-bg border border-cyber-border"
        }`}
      >
        <motion.span
          animate={{ x: checked ? 20 : 2 }}
          transition={{ type: "spring", stiffness: 500, damping: 30 }}
          className={`absolute top-0.5 w-5 h-5 rounded-full ${checked ? "bg-accent" : "bg-cyber-text-faint"}`}
        />
      </button>
    </label>
  );
}

// ── History Tab ──────────────────────────────────────────────────────────────

function HistoryTab() {
  const [entries, setEntries] = useState<HistoryEntryDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");

  useEffect(() => {
    api.getUpdateHistory().then(setEntries).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const filtered = entries.filter((e) =>
    e.package.toLowerCase().includes(search.toLowerCase()) ||
    e.backend.toLowerCase().includes(search.toLowerCase())
  );

  const successCount = entries.filter((e) => e.outcome.includes("Updated")).length;
  const failCount = entries.filter((e) => e.outcome.includes("Failed")).length;

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-glow-cyan">Update History</h1>
        {entries.length > 0 && (
          <motion.button
            whileHover={{ scale: 1.02 }}
            whileTap={{ scale: 0.98 }}
            onClick={() => { api.clearUpdateHistory(); setEntries([]); }}
            className="px-3 py-1.5 rounded-lg border border-danger/30 text-danger text-xs hover:bg-danger/10 transition-all"
          >
            Clear History
          </motion.button>
        )}
      </div>

      {entries.length > 0 && (
        <div className="flex gap-4 text-xs">
          <span className="text-success">{successCount} successful</span>
          <span className="text-danger">{failCount} failed</span>
          <span className="text-cyber-text-dim">{entries.length} total</span>
        </div>
      )}

      <input
        type="text"
        placeholder="Search history..."
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
      />

      {loading && <div className="text-center py-8 text-cyber-text-dim">Loading history...</div>}

      {!loading && entries.length === 0 && (
        <div className="text-center py-12">
          <History className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
          <p className="text-sm text-cyber-text-dim">No update history yet.</p>
        </div>
      )}

      {!loading && filtered.length > 0 && (
        <div className="space-y-1">
          {filtered.map((e, i) => (
            <motion.div
              key={i}
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: i * 0.02 }}
              className="flex items-center gap-3 p-3 rounded-lg bg-cyber-surface border border-cyber-border text-xs"
            >
              {e.outcome.includes("Updated") ? (
                <CheckCircle className="w-4 h-4 text-success flex-shrink-0" />
              ) : e.outcome.includes("Failed") ? (
                <XCircle className="w-4 h-4 text-danger flex-shrink-0" />
              ) : (
                <Clock className="w-4 h-4 text-cyber-text-faint flex-shrink-0" />
              )}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-cyber-text truncate">{e.package}</span>
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono">{e.backend}</span>
                </div>
                <div className="text-cyber-text-faint mt-0.5">
                  {e.from_version} -&gt; {e.to_version} · {new Date(e.timestamp).toLocaleString()}
                </div>
              </div>
              <span className="text-cyber-text-faint flex-shrink-0">{e.outcome}</span>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Packages Tab ─────────────────────────────────────────────────────────────

function PackagesTab() {
  const [packages, setPackages] = useState<InstalledPackageDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [backendFilter, setBackendFilter] = useState("");

  useEffect(() => {
    api.listInstalledPackages().then(setPackages).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const backends = [...new Set(packages.map((p) => p.backend))];

  const filtered = packages.filter((p) =>
    (p.name.toLowerCase().includes(search.toLowerCase()) || p.id.toLowerCase().includes(search.toLowerCase())) &&
    (backendFilter === "" || p.backend === backendFilter)
  );

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <h1 className="text-2xl font-bold text-glow-cyan">Installed Packages</h1>

      <div className="flex gap-3 flex-wrap">
        <input
          type="text"
          placeholder="Search packages..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="flex-1 px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
        />
        <select
          value={backendFilter}
          onChange={(e) => setBackendFilter(e.target.value)}
          className="px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
        >
          <option value="">All backends</option>
          {backends.map((b) => <option key={b} value={b}>{b}</option>)}
        </select>
      </div>

      {loading && <div className="text-center py-8 text-cyber-text-dim">Scanning installed packages...</div>}

      {!loading && filtered.length === 0 && (
        <div className="text-center py-12">
          <Layers className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
          <p className="text-sm text-cyber-text-dim">No packages found.</p>
        </div>
      )}

      {!loading && filtered.length > 0 && (
        <div className="space-y-1">
          <div className="text-xs text-cyber-text-dim mb-2">{filtered.length} packages</div>
          {filtered.map((p, i) => (
            <motion.div
              key={`${p.backend}-${p.id}`}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: Math.min(i * 0.01, 0.5) }}
              className="flex items-center gap-3 p-3 rounded-lg bg-cyber-surface border border-cyber-border text-xs hover:border-cyber-border-bright transition-all"
            >
              <Package className="w-4 h-4 text-accent flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <span className="font-medium text-cyber-text">{p.name}</span>
              </div>
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono">{p.backend}</span>
              <span className="text-cyber-text-dim font-mono flex-shrink-0">{p.version}</span>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Hardware Tab ─────────────────────────────────────────────────────────────

function HardwareTab() {
  const [info, setInfo] = useState<HardwareInfoDto | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.getHardwareInfo().then(setInfo).catch(() => {}).finally(() => setLoading(false));
  }, []);

  if (loading) return <div className="text-center py-12 text-cyber-text-dim">Detecting hardware...</div>;
  if (!info) return <div className="text-center py-12 text-danger">Failed to get hardware info.</div>;

  const cards = [
    { label: "CPU", value: info.cpu, sub: `${info.cpu_cores} cores`, icon: <Cpu className="w-5 h-5" /> },
    { label: "Memory", value: `${info.total_memory_gb.toFixed(1)} GB`, sub: "Total RAM", icon: <HardDrive className="w-5 h-5" /> },
    { label: "OS", value: info.os, sub: "Operating System", icon: <Info className="w-5 h-5" /> },
  ];

  return (
    <div className="max-w-4xl mx-auto space-y-6">
      <h1 className="text-2xl font-bold text-glow-cyan">Hardware Info</h1>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {cards.map((c, i) => (
          <motion.div
            key={c.label}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: i * 0.05 }}
            className="p-4 rounded-xl bg-cyber-surface border border-cyber-border glow-cyan"
          >
            <div className="text-accent mb-2">{c.icon}</div>
            <div className="text-sm font-bold truncate">{c.value}</div>
            <div className="text-xs text-cyber-text-dim mt-0.5">{c.sub}</div>
            <div className="text-[10px] text-cyber-text-faint mt-1 uppercase tracking-wider">{c.label}</div>
          </motion.div>
        ))}
      </div>

      {info.gpu.length > 0 && (
        <div className="rounded-xl bg-cyber-surface border border-cyber-border p-4">
          <h3 className="text-sm font-bold mb-3 text-accent">Graphics Cards</h3>
          <div className="space-y-2">
            {info.gpu.map((g, i) => (
              <div key={i} className="flex items-center gap-3 p-3 rounded-lg bg-cyber-bg/50 border border-cyber-border text-xs">
                <Zap className="w-4 h-4 text-purple-neon flex-shrink-0" />
                <div className="flex-1">
                  <div className="font-medium text-cyber-text">{g.name}</div>
                  <div className="text-cyber-text-faint">{g.vendor}</div>
                </div>
                <span className="text-cyber-text-dim font-mono">Driver: {g.driver_version}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {info.disks.length > 0 && (
        <div className="rounded-xl bg-cyber-surface border border-cyber-border p-4">
          <h3 className="text-sm font-bold mb-3 text-accent">Disks</h3>
          <div className="space-y-2">
            {info.disks.map((d, i) => (
              <div key={i} className="flex items-center gap-3 p-3 rounded-lg bg-cyber-bg/50 border border-cyber-border text-xs">
                <HardDrive className="w-4 h-4 text-success flex-shrink-0" />
                <span className="font-medium text-cyber-text flex-1">{d.name}</span>
                <span className="text-cyber-text-dim font-mono">{d.size_gb.toFixed(0)} GB</span>
                <span className="text-cyber-text-faint">{d.filesystem}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Logs Tab ─────────────────────────────────────────────────────────────────

function LogsTab() {
  const [logs, setLogs] = useState<LogEntryDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [levelFilter, setLevelFilter] = useState("");

  useEffect(() => {
    api.getLogs().then(setLogs).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const levels = [...new Set(logs.map((l) => l.level))];
  const filtered = levelFilter === "" ? logs : logs.filter((l) => l.level === levelFilter);

  const levelColor = (level: string) => {
    if (level.includes("ERROR") || level.includes("error") || level.includes("WARN")) return "text-warning";
    if (level.includes("INFO") || level.includes("info")) return "text-accent";
    if (level.includes("DEBUG") || level.includes("debug")) return "text-cyber-text-faint";
    return "text-cyber-text-dim";
  };

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-glow-cyan">Log Viewer</h1>
        <button
          onClick={() => api.getLogs().then(setLogs)}
          className="px-3 py-1.5 rounded-lg border border-cyber-border text-xs hover:bg-cyber-surface-2 transition-all"
        >
          Refresh
        </button>
      </div>

      {levels.length > 0 && (
        <div className="flex gap-2">
          <button
            onClick={() => setLevelFilter("")}
            className={`px-3 py-1 rounded-full text-xs ${levelFilter === "" ? "bg-accent/20 text-accent" : "bg-cyber-surface text-cyber-text-dim"}`}
          >
            All
          </button>
          {levels.map((l) => (
            <button
              key={l}
              onClick={() => setLevelFilter(l)}
              className={`px-3 py-1 rounded-full text-xs ${levelFilter === l ? "bg-accent/20 text-accent" : "bg-cyber-surface text-cyber-text-dim"}`}
            >
              {l}
            </button>
          ))}
        </div>
      )}

      {loading && <div className="text-center py-8 text-cyber-text-dim">Loading logs...</div>}

      {!loading && filtered.length === 0 && (
        <div className="text-center py-12">
          <ScrollText className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
          <p className="text-sm text-cyber-text-dim">No logs available.</p>
        </div>
      )}

      {!loading && filtered.length > 0 && (
        <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 max-h-[60vh] overflow-y-auto font-mono text-xs space-y-1">
          {filtered.map((l, i) => (
            <div key={i} className="flex gap-3">
              {l.timestamp && <span className="text-cyber-text-faint flex-shrink-0">{l.timestamp}</span>}
              <span className={`flex-shrink-0 ${levelColor(l.level)}`}>{l.level}</span>
              <span className="text-cyber-text-dim break-all">{l.message}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Profiles Tab ─────────────────────────────────────────────────────────────

function ProfilesTab() {
  const [profiles, setProfiles] = useState<ProfileDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPackages, setNewPackages] = useState("");

  const refresh = () => {
    api.listProfiles().then(setProfiles).catch(() => {}).finally(() => setLoading(false));
  };

  useEffect(refresh, []);

  const create = async () => {
    if (!newName.trim()) return;
    const pkgs = newPackages.split("\n").map((s) => s.trim()).filter(Boolean);
    try {
      await api.createProfile(newName.trim(), pkgs);
      setNewName("");
      setNewPackages("");
      setShowCreate(false);
      refresh();
    } catch (e) {
      console.error(e);
    }
  };

  const remove = async (name: string) => {
    try {
      await api.deleteProfile(name);
      refresh();
    } catch (e) {
      console.error(e);
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-glow-cyan">Update Profiles</h1>
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={() => setShowCreate(!showCreate)}
          className="px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 transition-all glow-cyan"
        >
          {showCreate ? "Cancel" : "New Profile"}
        </motion.button>
      </div>

      {showCreate && (
        <motion.div
          initial={{ opacity: 0, y: -10 }}
          animate={{ opacity: 1, y: 0 }}
          className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-3"
        >
          <div>
            <label className="block text-sm font-medium mb-1">Profile name</label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
              placeholder="e.g. Gaming, Development, Minimal"
            />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">Packages (one per line)</label>
            <textarea
              value={newPackages}
              onChange={(e) => setNewPackages(e.target.value)}
              rows={4}
              className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm font-mono focus:border-accent"
              placeholder={"Mozilla.Firefox\nGit.Git\nMicrosoft.VisualStudioCode"}
            />
          </div>
          <button
            onClick={create}
            className="px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 transition-all"
          >
            Create
          </button>
        </motion.div>
      )}

      {loading && <div className="text-center py-8 text-cyber-text-dim">Loading profiles...</div>}

      {!loading && profiles.length === 0 && !showCreate && (
        <div className="text-center py-12">
          <Layers className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
          <p className="text-sm text-cyber-text-dim">No profiles configured. Create one to group packages for targeted updates.</p>
        </div>
      )}

      {!loading && profiles.length > 0 && (
        <div className="space-y-2">
          {profiles.map((p, i) => (
            <motion.div
              key={p.name}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: i * 0.05 }}
              className="rounded-lg border border-cyber-border bg-cyber-surface p-4"
            >
              <div className="flex items-center justify-between mb-2">
                <h3 className="font-bold text-sm text-accent">{p.name}</h3>
                <button
                  onClick={() => remove(p.name)}
                  className="text-xs text-danger hover:underline"
                >
                  Delete
                </button>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {p.packages.map((pkg) => (
                  <span key={pkg} className="text-[10px] px-2 py-0.5 rounded-full bg-cyber-bg border border-cyber-border text-cyber-text-dim font-mono">
                    {pkg}
                  </span>
                ))}
              </div>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Offline Tab ──────────────────────────────────────────────────────────────

function OfflineTab() {
  const [status, setStatus] = useState<OfflineCacheStatusDto | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = () => {
    api.getOfflineCacheStatus().then(setStatus).catch(() => {}).finally(() => setLoading(false));
  };

  useEffect(refresh, []);

  const formatBytes = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <h1 className="text-2xl font-bold text-glow-cyan">Offline Mode</h1>
      <p className="text-sm text-cyber-text-dim">
        Manage cached driver and software installers for updating systems without internet access.
      </p>

      {loading && <div className="text-center py-8 text-cyber-text-dim">Checking cache...</div>}

      {!loading && status && (
        <>
          <div className="grid grid-cols-2 gap-4">
            <div className="p-4 rounded-xl bg-cyber-surface border border-cyber-border glow-cyan">
              <div className="text-accent mb-2"><WifiOff className="w-5 h-5" /></div>
              <div className="text-xl font-bold">{status.entry_count}</div>
              <div className="text-xs text-cyber-text-dim mt-0.5">Cached entries</div>
            </div>
            <div className="p-4 rounded-xl bg-cyber-surface border border-cyber-border glow-purple">
              <div className="text-purple-neon mb-2"><HardDrive className="w-5 h-5" /></div>
              <div className="text-xl font-bold">{formatBytes(status.cache_size_bytes)}</div>
              <div className="text-xs text-cyber-text-dim mt-0.5">Cache size</div>
            </div>
          </div>

          {status.entry_count > 0 && (
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={() => { api.clearOfflineCache(); refresh(); }}
              className="px-4 py-2 rounded-lg border border-danger/30 text-danger text-sm font-medium hover:bg-danger/10 transition-all"
            >
              Clear Cache
            </motion.button>
          )}

          {status.entry_count === 0 && (
            <div className="text-center py-8">
              <WifiOff className="w-12 h-12 mx-auto mb-3 text-cyber-text-faint" />
              <p className="text-sm text-cyber-text-dim">No cached installers. Run a scan to populate the cache.</p>
            </div>
          )}
        </>
      )}
    </div>
  );
}
