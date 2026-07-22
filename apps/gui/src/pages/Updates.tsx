import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  CheckCircle,
  Clock,
  Download,
  HardDrive,
  Package,
  RefreshCw,
  XCircle,
} from "lucide-react";
import * as api from "../api";
import { safeListen } from "../events";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  errorText,
  formatSize,
} from "../components/ui";
import type { ApplyResultDto, ApplyStatus, SkippedDto, UpdateDto } from "../types";

interface Progress {
  package: string;
  current: number;
  total: number;
  /** "installing" while working, "done" at the end. */
  phase: string;
  /** 0–100, computed from completed count. */
  percent: number | null;
}

export default function Updates() {
  const { scan, sysInfo, history, packages } = useStore();
  const [applying, setApplying] = useState(false);
  const [applyResult, setApplyResult] = useState<ApplyResultDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dryRun, setDryRun] = useState(false);
  const [restorePoint, setRestorePoint] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);

  // Deselected ids rather than selected ones, so updates that appear in a later
  // scan are selected by default without needing to re-sync a selection set.
  const [deselected, setDeselected] = useState<Set<string>>(new Set());

  const scanResult = scan.data;
  const actionable = useMemo(() => scanResult?.actionable ?? [], [scanResult]);
  const selected = useMemo(
    () => actionable.filter((u) => !deselected.has(u.id)),
    [actionable, deselected],
  );

  useEffect(
    () => safeListen<Progress>("apply-progress", (e) => setProgress(e.payload)),
    [],
  );

  const doScan = useCallback(async () => {
    setError(null);
    setApplyResult(null);
    setProgress(null);
    await scan.refresh();
  }, [scan]);

  const doApply = useCallback(async () => {
    if (selected.length === 0) return;
    setApplying(true);
    setError(null);
    setProgress(null);
    setApplyResult(null);
    try {
      const result = await api.apply({
        updates: selected,
        dry_run: dryRun,
        restore_point: restorePoint,
      });
      setApplyResult(result);
      if (!dryRun) {
        await Promise.allSettled([scan.refresh(), history.refresh(), packages.refresh()]);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setProgress(null);
      setApplying(false);
    }
  }, [selected, dryRun, restorePoint, scan, history, packages]);

  const toggleSelect = (id: string) =>
    setDeselected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const toggleAll = () => {
    if (selected.length === actionable.length) {
      setDeselected(new Set(actionable.map((u) => u.id)));
    } else {
      setDeselected(new Set());
    }
  };

  const info = sysInfo.data;
  const firstScanRunning = scan.loading && scan.loadedAt === null;

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader title="Updates" subtitle="Packages with a newer version available" />

      {info && (
        <div className="flex items-center gap-4 text-xs text-cyber-text-dim bg-cyber-surface rounded-lg border border-cyber-border px-4 py-3">
          <span className="text-accent">{info.os}</span>
          <span>v{info.version}</span>
          <span className={info.elevated ? "text-warning" : "text-cyber-text-faint"}>
            {info.elevated ? "Elevated" : "Unelevated"}
          </span>
        </div>
      )}

      <div className="flex items-center gap-3 flex-wrap">
        <motion.button
          type="button"
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          onClick={doScan}
          disabled={scan.loading || applying}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-accent/10 border border-accent/30 text-accent font-medium text-sm hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
        >
          <RefreshCw className={`w-4 h-4 ${scan.loading ? "animate-spin" : ""}`} />
          {scan.loading ? "Scanning..." : "Scan for Updates"}
        </motion.button>

        {actionable.length > 0 && (
          <>
            <motion.button
              type="button"
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={doApply}
              disabled={applying || scan.loading || selected.length === 0}
              className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-success/10 border border-success/30 text-success font-medium text-sm hover:bg-success/20 disabled:opacity-50 transition-all glow-green"
            >
              <Download className={`w-4 h-4 ${applying ? "animate-pulse" : ""}`} />
              {applying
                ? "Applying..."
                : `Apply ${selected.length} Update${selected.length !== 1 ? "s" : ""}`}
            </motion.button>

            <label className="flex items-center gap-2 text-xs text-cyber-text-dim cursor-pointer">
              <input
                type="checkbox"
                checked={dryRun}
                onChange={(e) => setDryRun(e.target.checked)}
                className="accent-accent"
              />
              Dry run
            </label>

            <label className="flex items-center gap-2 text-xs text-cyber-text-dim cursor-pointer">
              <input
                type="checkbox"
                checked={restorePoint}
                onChange={(e) => setRestorePoint(e.target.checked)}
                className="accent-accent"
              />
              Restore point
            </label>
          </>
        )}
      </div>

      {applying && progress && progress.total > 0 && (
        <motion.div
          initial={{ opacity: 0, y: -5 }}
          animate={{ opacity: 1, y: 0 }}
          className="rounded-lg border border-accent/30 bg-accent/5 p-4 scan-overlay"
        >
          <div className="flex items-center justify-between text-xs mb-2">
            <span className="text-accent truncate">
              {progress.phase === "done"
                ? "Finishing up…"
                : progress.package
                  ? `Installing ${progress.package}`
                  : "Working…"}
            </span>
            <span className="text-cyber-text-dim flex-shrink-0 font-mono">
              {progress.current}/{progress.total}
              {progress.percent != null && ` · ${progress.percent}%`}
            </span>
          </div>
          <div className="h-1.5 bg-cyber-bg rounded-full overflow-hidden">
            <motion.div
              className="h-full bg-accent glow-cyan"
              animate={{
                width: `${progress.percent ?? (progress.current / progress.total) * 100}%`,
              }}
              transition={{ duration: 0.3 }}
            />
          </div>
        </motion.div>
      )}

      {error && <ErrorBar message={error} />}
      {scan.error && <ErrorBar message={`Scan failed: ${scan.error}`} />}

      {/* A backend that errors out is not the same as having no updates. */}
      {scanResult && scanResult.failed_backends.length > 0 && (
        <details className="rounded-lg border border-warning/30 bg-warning/5 px-4 py-3 text-xs text-warning">
          <summary className="cursor-pointer">
            {scanResult.failed_backends.length} backend
            {scanResult.failed_backends.length !== 1 ? "s" : ""} could not be scanned — these
            results may be incomplete
          </summary>
          <ul className="mt-2 space-y-1">
            {scanResult.failed_backends.map((f) => (
              <li key={f.backend} className="font-mono break-words">
                {f.backend}: {f.error}
              </li>
            ))}
          </ul>
        </details>
      )}

      {applyResult && (
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
        >
          <Card title={dryRun ? "Dry Run Results" : "Apply Results"}>
            <div className="flex gap-6 text-xs flex-wrap">
              <span className="text-success">{applyResult.updated} updated</span>
              <span className="text-danger">{applyResult.failed} failed</span>
              <span className="text-cyber-text-dim">{applyResult.skipped} skipped</span>
              {applyResult.reboot_required && (
                <span className="text-warning font-medium">Reboot required</span>
              )}
            </div>
            {applyResult.entries.length > 0 && (
              <div className="space-y-1 mt-3">
                {applyResult.entries.map((e, i) => (
                  <div
                    key={`${e.name}-${i}`}
                    className="flex items-center gap-2 text-xs py-1 border-t border-cyber-border"
                  >
                    <OutcomeIcon status={e.status} />
                    <span className="flex-1 text-cyber-text truncate">{e.name}</span>
                    <span
                      className="text-cyber-text-faint flex-shrink-0 max-w-[50%] truncate"
                      title={e.detail}
                    >
                      {e.detail}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </motion.div>
      )}

      {firstScanRunning && (
        <EmptyState
          icon={<RefreshCw className="w-12 h-12 animate-spin text-accent" />}
          title="Scanning every detected backend..."
          hint="This runs automatically at startup and can take a minute."
        />
      )}

      {scanResult && (
        <div className="space-y-2">
          {actionable.length > 0 && (
            <>
              <div className="flex items-center justify-between">
                <h3 className="font-bold text-sm">
                  {actionable.length} update{actionable.length !== 1 ? "s" : ""} available
                </h3>
                <button
                  type="button"
                  onClick={toggleAll}
                  className="text-xs text-accent hover:underline"
                >
                  {selected.length === actionable.length ? "Deselect all" : "Select all"}
                </button>
              </div>
              {actionable.map((u, i) => (
                <UpdateCard
                  key={`${u.backend}:${u.id}`}
                  update={u}
                  checked={!deselected.has(u.id)}
                  onToggle={() => toggleSelect(u.id)}
                  index={i}
                />
              ))}
            </>
          )}

          {scanResult.skipped.length > 0 && (
            <>
              <h3 className="font-bold text-sm pt-4 text-cyber-text-dim">
                {scanResult.skipped.length} skipped by policy
              </h3>
              {scanResult.skipped.map((s, i) => (
                <SkippedCard key={`${s.backend}:${s.id}:${i}`} skipped={s} />
              ))}
            </>
          )}

          {scanResult.total === 0 && !scan.loading && (
            <EmptyState
              icon={<CheckCircle className="w-12 h-12 text-success glow-green rounded-full" />}
              title="Everything is up to date."
            />
          )}
        </div>
      )}

      {!scanResult && !scan.loading && !scan.error && (
        <EmptyState
          icon={<Package className="w-12 h-12" />}
          title='No scan results yet.'
          hint='Click "Scan for Updates" to check for available updates.'
        />
      )}
    </div>
  );
}

export function OutcomeIcon({ status }: { status: ApplyStatus }) {
  switch (status) {
    case "updated":
      return <CheckCircle className="w-4 h-4 text-success flex-shrink-0" />;
    case "skipped":
      return <Clock className="w-4 h-4 text-cyber-text-faint flex-shrink-0" />;
    default:
      return <XCircle className="w-4 h-4 text-danger flex-shrink-0" />;
  }
}

function UpdateCard({
  update,
  checked,
  onToggle,
  index,
}: {
  update: UpdateDto;
  checked: boolean;
  onToggle: () => void;
  index: number;
}) {
  return (
    <motion.label
      initial={{ opacity: 0, x: -10 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ delay: Math.min(index * 0.03, 0.4) }}
      className={`flex items-start gap-3 p-4 rounded-lg border cursor-pointer transition-all ${
        checked
          ? "bg-accent/5 border-accent/30 glow-cyan"
          : "bg-cyber-surface border-cyber-border hover:border-cyber-border-bright"
      }`}
    >
      <input
        type="checkbox"
        checked={checked}
        onChange={onToggle}
        className="mt-1 accent-accent"
      />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-sm truncate">{update.name}</span>
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono flex-shrink-0">
            {update.backend}
          </span>
        </div>
        <div className="flex items-center gap-3 mt-1 text-xs text-cyber-text-dim">
          <span className="font-mono">
            {update.installed} <span className="text-cyber-text-faint">-&gt;</span>{" "}
            <span className="text-accent font-medium">{update.available}</span>
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

function SkippedCard({ skipped }: { skipped: SkippedDto }) {
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
