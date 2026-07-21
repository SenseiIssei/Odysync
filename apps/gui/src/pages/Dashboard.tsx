import { useState } from "react";
import { motion } from "framer-motion";
import {
  Cpu,
  Download,
  Package,
  PlayCircle,
  RefreshCw,
  Rocket,
  Shield,
  Zap,
} from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import { Card, PageHeader, StatCard, errorText } from "../components/ui";
import type { Tab } from "../nav";

export default function Dashboard({ onNavigate }: { onNavigate: (tab: Tab) => void }) {
  const { sysInfo, backends, scan, history, packages, backup } = useStore();
  const [busyLabel, setBusyLabel] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const scanResult = scan.data;
  const updateCount = scanResult?.actionable.length ?? 0;
  const availableCount = backends.data.filter((b) => b.available).length;
  const busy = busyLabel !== null || scan.loading;

  const quickScan = async () => {
    setStatus("Scanning all backends...");
    await scan.refresh();
    // `scan.data` is stale in this closure; read the fresh error/count instead.
    setStatus(null);
  };

  const applyAll = async (updates: NonNullable<typeof scanResult>["actionable"]) => {
    setStatus(`Creating a restore point before installing ${updates.length} updates...`);
    try {
      await api.createBackup(`Odysync - before bulk update`).catch(() => {
        setStatus("Restore point could not be created; continuing without one.");
      });
      setStatus(`Installing ${updates.length} updates...`);
      const result = await api.apply({
        updates,
        dry_run: false,
        restore_point: false,
      });
      setStatus(
        `Done: ${result.updated} updated, ${result.failed} failed, ${result.skipped} skipped.` +
          (result.reboot_required ? " A reboot is required." : ""),
      );
      await Promise.allSettled([
        scan.refresh(),
        history.refresh(),
        packages.refresh(),
        backup.refresh(),
      ]);
    } catch (e) {
      setStatus(`Install failed: ${errorText(e)}`);
    }
  };

  const installAll = async () => {
    if (!scanResult || scanResult.actionable.length === 0) return;
    setBusyLabel("install");
    await applyAll(scanResult.actionable);
    setBusyLabel(null);
  };

  const autoScanInstall = async () => {
    setBusyLabel("auto");
    setStatus("Auto mode: scanning...");
    try {
      const fresh = await api.scan();
      scan.set(fresh);
      if (fresh.actionable.length === 0) {
        setStatus("Auto mode: nothing to do, everything is up to date.");
      } else {
        await applyAll(fresh.actionable);
      }
    } catch (e) {
      setStatus(`Auto mode failed: ${errorText(e)}`);
    }
    setBusyLabel(null);
  };

  const stats = [
    {
      label: "Updates Available",
      value: scan.loading && scan.loadedAt === null ? "..." : String(updateCount),
      icon: <Package className="w-5 h-5" />,
      color: "text-accent",
      glow: "glow-cyan",
    },
    {
      label: "Backends Active",
      value: backends.loadedAt === null ? "..." : `${availableCount}/${backends.data.length}`,
      icon: <Zap className="w-5 h-5" />,
      color: "text-purple-neon",
      glow: "glow-purple",
    },
    {
      label: "System",
      value: sysInfo.data?.os ?? "Detecting...",
      icon: <Cpu className="w-5 h-5" />,
      color: "text-success",
      glow: "glow-green",
    },
    {
      label: "Elevation",
      value: sysInfo.data ? (sysInfo.data.elevated ? "Elevated" : "User") : "...",
      icon: <Shield className="w-5 h-5" />,
      color: sysInfo.data?.elevated ? "text-warning" : "text-cyber-text-dim",
      glow: "",
    },
  ];

  return (
    <div className="max-w-4xl mx-auto space-y-6">
      <PageHeader title="Dashboard" subtitle="System overview at a glance" />

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {stats.map((stat, i) => (
          <StatCard key={stat.label} {...stat} delay={i * 0.05} />
        ))}
      </div>

      <div className="flex flex-wrap gap-3">
        <motion.button
          type="button"
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={quickScan}
          disabled={busy}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-accent/10 border border-accent/30 text-accent font-medium text-sm hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
        >
          <RefreshCw className={`w-4 h-4 ${scan.loading ? "animate-spin" : ""}`} />
          {scan.loading ? "Scanning..." : "Quick Scan"}
        </motion.button>

        <motion.button
          type="button"
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={installAll}
          disabled={busy || updateCount === 0}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-success/10 border border-success/30 text-success font-medium text-sm hover:bg-success/20 disabled:opacity-50 transition-all"
        >
          <Download
            className={`w-4 h-4 ${busyLabel === "install" ? "animate-bounce" : ""}`}
          />
          {busyLabel === "install" ? "Installing..." : `Install All (${updateCount})`}
        </motion.button>

        <motion.button
          type="button"
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={autoScanInstall}
          disabled={busy}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-purple-neon/10 border border-purple-neon/30 text-purple-neon font-medium text-sm hover:bg-purple-neon/20 disabled:opacity-50 transition-all"
        >
          <Rocket className={`w-4 h-4 ${busyLabel === "auto" ? "animate-pulse" : ""}`} />
          {busyLabel === "auto" ? "Auto Mode..." : "Auto Scan + Install"}
        </motion.button>

        <motion.button
          type="button"
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={() => onNavigate("updates")}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-cyber-surface border border-cyber-border text-cyber-text font-medium text-sm hover:border-accent/30 transition-all"
        >
          <Package className="w-4 h-4" />
          View Updates
        </motion.button>
      </div>

      {(status || scan.error) && (
        <motion.div
          initial={{ opacity: 0, y: -5 }}
          animate={{ opacity: 1, y: 0 }}
          className="flex items-start gap-2 text-sm text-cyber-text-dim bg-cyber-surface border border-cyber-border rounded-lg px-4 py-3"
        >
          <PlayCircle className="w-4 h-4 text-accent flex-shrink-0 mt-0.5" />
          <span className="break-words">{status ?? `Scan error: ${scan.error}`}</span>
        </motion.div>
      )}

      <Card>
        <h3 className="text-sm font-bold mb-3 flex items-center gap-2">
          <Zap className="w-4 h-4 text-accent" />
          Backend Status
          {backends.loading && (
            <RefreshCw className="w-3 h-3 animate-spin text-cyber-text-faint" />
          )}
        </h3>
        <div className="grid grid-cols-2 md:grid-cols-3 gap-2">
          {backends.data.length === 0 && (
            <div className="text-xs text-cyber-text-faint col-span-full py-4 text-center">
              {backends.loading ? "Detecting backends..." : "No backends detected."}
            </div>
          )}
          {backends.data.map((b, i) => (
            <motion.div
              key={b.kind}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: Math.min(i * 0.02, 0.4) }}
              className="flex items-center justify-between gap-2 text-xs px-3 py-2 rounded-lg bg-cyber-bg/50 border border-cyber-border"
              title={b.available ? "Available" : "Not installed on this system"}
            >
              <span className="truncate text-cyber-text-dim">{b.name}</span>
              <span
                className={`w-2 h-2 rounded-full flex-shrink-0 ${
                  b.available ? "bg-success glow-green" : "bg-cyber-text-faint"
                }`}
              />
            </motion.div>
          ))}
        </div>
      </Card>
    </div>
  );
}
