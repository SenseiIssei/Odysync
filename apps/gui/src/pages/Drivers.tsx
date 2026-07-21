import { useCallback, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  Cpu,
  Download,
  MemoryStick,
  RefreshCw,
  ShieldCheck,
  Zap,
} from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  errorText,
  formatSize,
} from "../components/ui";
import {
  GROUP_HINTS,
  GROUP_LABELS,
  hardwareGroupOf,
  isHardwareBackend,
  type HardwareGroup,
} from "../hardware-backends";
import type { ApplyResultDto, UpdateDto } from "../types";
import { OutcomeIcon } from "./Updates";

const GROUP_ORDER: HardwareGroup[] = ["driver", "firmware", "oem"];

export default function Drivers() {
  const { scan, hardware, sysInfo, backends, history, backup } = useStore();
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ApplyResultDto | null>(null);
  const [deselected, setDeselected] = useState<Set<string>>(new Set());
  const [confirmFirmware, setConfirmFirmware] = useState(false);

  const elevated = sysInfo.data?.elevated ?? false;

  const hardwareUpdates = useMemo(
    () => (scan.data?.actionable ?? []).filter((u) => isHardwareBackend(u.backend)),
    [scan.data],
  );

  const grouped = useMemo(() => {
    const map: Record<HardwareGroup, UpdateDto[]> = { driver: [], firmware: [], oem: [] };
    for (const u of hardwareUpdates) {
      const g = hardwareGroupOf(u.backend);
      if (g) map[g].push(u);
    }
    return map;
  }, [hardwareUpdates]);

  const selected = useMemo(
    () => hardwareUpdates.filter((u) => !deselected.has(u.id)),
    [hardwareUpdates, deselected],
  );

  const selectedHasFirmware = selected.some(
    (u) => hardwareGroupOf(u.backend) === "firmware",
  );

  // Hardware backends that reported an error are worth showing prominently:
  // a driver backend that cannot enumerate looks identical to "no updates".
  const hardwareScanFailures = useMemo(
    () => (scan.data?.failed_backends ?? []).filter((f) => isHardwareBackend(f.backend)),
    [scan.data],
  );

  const hardwareBackendsPresent = useMemo(
    () => backends.data.filter((b) => isHardwareBackend(b.kind) && b.available),
    [backends.data],
  );

  const toggle = (id: string) =>
    setDeselected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const applySelected = useCallback(async () => {
    if (selected.length === 0) return;
    if (selectedHasFirmware && !confirmFirmware) {
      setConfirmFirmware(true);
      return;
    }
    setConfirmFirmware(false);
    setApplying(true);
    setError(null);
    setResult(null);
    try {
      const r = await api.apply({
        updates: selected,
        dry_run: false,
        // Hardware changes are the ones you most want to be able to roll back.
        restore_point: true,
      });
      setResult(r);
      await Promise.allSettled([scan.refresh(), history.refresh(), backup.refresh()]);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setApplying(false);
    }
  }, [selected, selectedHasFirmware, confirmFirmware, scan, history, backup]);

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader
        title="Hardware Updates"
        subtitle="Drivers, firmware and vendor update tools"
        actions={
          <button
            type="button"
            onClick={() => void scan.refresh()}
            disabled={scan.loading || applying}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-cyber-border text-xs text-cyber-text-dim hover:text-accent hover:border-accent/30 disabled:opacity-50 transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${scan.loading ? "animate-spin" : ""}`} />
            {scan.loading ? "Scanning..." : "Rescan"}
          </button>
        }
      />

      {!elevated && (
        <div className="flex items-start gap-2 text-xs text-warning bg-warning/5 rounded-lg px-4 py-3 border border-warning/30">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
          <span>
            Driver and firmware updates require administrator rights. Without them Windows
            refuses the driver search entirely, so this page will look empty even when
            updates exist. Use "Run as Admin" in the title bar.
          </span>
        </div>
      )}

      {/* Detected hardware, so the page is useful even with nothing to install. */}
      {hardware.data && (
        <Card title="Detected hardware">
          <div className="space-y-1.5 text-xs">
            <div className="flex items-center gap-2">
              <Cpu className="w-3.5 h-3.5 text-accent flex-shrink-0" />
              <span className="text-cyber-text-dim truncate">{hardware.data.cpu}</span>
            </div>
            {hardware.data.gpu.map((g, i) => (
              <div key={`${g.name}-${i}`} className="flex items-center gap-2">
                <Zap className="w-3.5 h-3.5 text-purple-neon flex-shrink-0" />
                <span className="text-cyber-text-dim truncate flex-1">{g.name}</span>
                <span className="text-cyber-text-faint font-mono flex-shrink-0">
                  driver {g.driver_version || "unknown"}
                </span>
              </div>
            ))}
          </div>
        </Card>
      )}

      {hardwareScanFailures.length > 0 && (
        <div className="rounded-lg border border-warning/30 bg-warning/5 px-4 py-3 text-xs text-warning space-y-1">
          <div className="font-medium">
            {hardwareScanFailures.length} hardware source could not be checked
          </div>
          {hardwareScanFailures.map((f) => (
            <div key={f.backend} className="font-mono break-words">
              {f.backend}: {f.error}
            </div>
          ))}
        </div>
      )}

      {error && <ErrorBar message={error} />}

      {result && (
        <Card title="Results">
          <div className="flex gap-6 text-xs flex-wrap">
            <span className="text-success">{result.updated} updated</span>
            <span className="text-danger">{result.failed} failed</span>
            <span className="text-cyber-text-dim">{result.skipped} skipped</span>
            {result.reboot_required && (
              <span className="text-warning font-medium">Reboot required</span>
            )}
          </div>
          <div className="space-y-1 mt-3">
            {result.entries.map((e, i) => (
              <div
                key={`${e.name}-${i}`}
                className="flex items-center gap-2 text-xs py-1 border-t border-cyber-border"
              >
                <OutcomeIcon status={e.status} />
                <span className="flex-1 text-cyber-text truncate">{e.name}</span>
                <span className="text-cyber-text-faint truncate max-w-[50%]" title={e.detail}>
                  {e.detail}
                </span>
              </div>
            ))}
          </div>
        </Card>
      )}

      {hardwareUpdates.length > 0 && (
        <>
          <div className="flex items-center gap-3 flex-wrap">
            <motion.button
              type="button"
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={applySelected}
              disabled={applying || scan.loading || selected.length === 0}
              className={`flex items-center gap-2 px-5 py-2.5 rounded-lg border font-medium text-sm disabled:opacity-50 transition-all ${
                confirmFirmware
                  ? "bg-danger/10 border-danger/30 text-danger hover:bg-danger/20"
                  : "bg-success/10 border-success/30 text-success hover:bg-success/20 glow-green"
              }`}
            >
              <Download className={`w-4 h-4 ${applying ? "animate-pulse" : ""}`} />
              {applying
                ? "Installing..."
                : confirmFirmware
                  ? "Confirm — this includes firmware"
                  : `Install ${selected.length} hardware update${selected.length !== 1 ? "s" : ""}`}
            </motion.button>
            <span className="flex items-center gap-1.5 text-xs text-cyber-text-dim">
              <ShieldCheck className="w-3.5 h-3.5 text-accent" />
              A restore point is always taken before hardware updates
            </span>
          </div>

          {confirmFirmware && (
            <div className="flex items-start gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30">
              <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
              <span>
                Your selection includes firmware/BIOS updates. Do not shut down or unplug the
                machine while these run — an interrupted firmware flash can leave a device
                unbootable. Run on mains power.
              </span>
            </div>
          )}
        </>
      )}

      {GROUP_ORDER.map((group) => {
        const items = grouped[group];
        if (items.length === 0) return null;
        return (
          <div key={group} className="space-y-2">
            <div>
              <h3 className="font-bold text-sm">
                {GROUP_LABELS[group]}{" "}
                <span className="text-cyber-text-faint font-normal">({items.length})</span>
              </h3>
              <p className="text-xs text-cyber-text-dim">{GROUP_HINTS[group]}</p>
            </div>
            {items.map((u, i) => (
              <motion.label
                key={`${u.backend}:${u.id}`}
                initial={{ opacity: 0, x: -10 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: Math.min(i * 0.03, 0.3) }}
                className={`flex items-start gap-3 p-4 rounded-lg border cursor-pointer transition-all ${
                  !deselected.has(u.id)
                    ? group === "firmware"
                      ? "bg-warning/5 border-warning/30"
                      : "bg-accent/5 border-accent/30 glow-cyan"
                    : "bg-cyber-surface border-cyber-border hover:border-cyber-border-bright"
                }`}
              >
                <input
                  type="checkbox"
                  checked={!deselected.has(u.id)}
                  onChange={() => toggle(u.id)}
                  className="mt-1 accent-accent"
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-sm truncate">{u.name}</span>
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono flex-shrink-0">
                      {u.backend}
                    </span>
                  </div>
                  <div className="flex items-center gap-3 mt-1 text-xs text-cyber-text-dim">
                    <span className="font-mono">
                      {u.installed} <span className="text-cyber-text-faint">-&gt;</span>{" "}
                      <span className="text-accent font-medium">{u.available}</span>
                    </span>
                    {u.size_bytes != null && u.size_bytes > 0 && (
                      <span>{formatSize(u.size_bytes)}</span>
                    )}
                  </div>
                </div>
              </motion.label>
            ))}
          </div>
        );
      })}

      {hardwareUpdates.length === 0 && !scan.loading && (
        <EmptyState
          icon={<MemoryStick className="w-12 h-12" />}
          title={
            scan.loadedAt === null
              ? "No scan results yet."
              : "No hardware updates available."
          }
          hint={
            hardwareBackendsPresent.length === 0
              ? "No hardware update sources were detected on this machine."
              : `Checked ${hardwareBackendsPresent.length} hardware source${hardwareBackendsPresent.length !== 1 ? "s" : ""}.`
          }
        />
      )}

      {scan.loading && scan.loadedAt === null && (
        <EmptyState
          icon={<RefreshCw className="w-12 h-12 animate-spin text-accent" />}
          title="Checking hardware sources..."
        />
      )}
    </div>
  );
}
