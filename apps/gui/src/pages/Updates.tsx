import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  Clock,
  Download,
  Package,
  RefreshCw,
  XCircle,
} from "lucide-react";
import * as api from "../api";
import { safeListen } from "../events";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  EmptyState,
  ErrorBar,
  Meter,
  PageHeader,
  WarningBar,
  errorText,
  formatSize,
} from "../components/ui";
import WindowsUpdateCard from "../components/WindowsUpdateCard";
import type { ApplyResultDto, ApplyStatus, UpdateDto } from "../types";

interface Progress {
  package: string;
  current: number;
  total: number;
  /** "installing" while working, "done" at the end. */
  phase: string;
  /** 0–100, computed from completed count. */
  percent: number | null;
}

/** Short, friendly names for where an update comes from. */
const SOURCE_LABELS: Record<string, string> = {
  direct: "Publisher",
  "nvidia-gpu": "NVIDIA",
  "windows-update": "Windows Update",
  "windows-drivers": "Windows Update",
  "windows-defender-update": "Windows Update",
  "windows-feature-update": "Windows Update",
  winget: "winget",
  msstore: "Store",
};

export default function Updates() {
  const { scan, sysInfo, history, packages, backends } = useStore();
  const [applying, setApplying] = useState(false);
  const [applyResult, setApplyResult] = useState<ApplyResultDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dryRun, setDryRun] = useState(false);
  const [restorePoint, setRestorePoint] = useState(true);
  const [progress, setProgress] = useState<Progress | null>(null);

  // Deselected ids rather than selected ones, so updates that appear in a later
  // scan are selected by default without needing to re-sync a selection set.
  const [deselected, setDeselected] = useState<Set<string>>(new Set());

  const scanResult = scan.data;
  const actionable = useMemo(() => scanResult?.actionable ?? [], [scanResult]);
  const selected = useMemo(
    () => actionable.filter((u) => !deselected.has(key(u))),
    [actionable, deselected],
  );
  const sourceName = useCallback(
    (backend: string) =>
      SOURCE_LABELS[backend] ?? backends.data.find((b) => b.kind === backend)?.name ?? backend,
    [backends.data],
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

  const toggle = (u: UpdateDto) =>
    setDeselected((prev) => {
      const next = new Set(prev);
      const k = key(u);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const allSelected = selected.length === actionable.length;
  const toggleAll = () =>
    setDeselected(allSelected ? new Set(actionable.map(key)) : new Set());

  const info = sysInfo.data;
  const firstScanRunning = scan.loading && scan.loadedAt === null;
  // Windows Update entries have their own card; the generic skipped list is
  // what remains.
  const otherSkipped = (scanResult?.skipped ?? []).filter(
    (s) => !s.backend.startsWith("windows-"),
  );

  return (
    <>
      <PageHeader
        title="Updates"
        subtitle="Newer versions of what is installed, checked against each publisher's own source."
        actions={
          <Button
            onClick={doScan}
            busy={scan.loading}
            disabled={applying}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            {scan.loading ? "Scanning…" : "Scan again"}
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {scan.error && <ErrorBar message={`Scan failed: ${scan.error}`} />}

      {/* A backend that errors out is not the same as having no updates. */}
      {scanResult && scanResult.failed_backends.length > 0 && (
        <WarningBar
          message={
            <details>
              <summary className="cursor-pointer">
                {scanResult.failed_backends.length} source
                {scanResult.failed_backends.length !== 1 ? "s" : ""} could not be checked — the
                list may be incomplete
              </summary>
              <ul className="mt-2 space-y-1 text-muted">
                {scanResult.failed_backends.map((f) => (
                  <li key={f.backend} className="font-mono text-xs break-words">
                    {f.backend}: {f.error}
                  </li>
                ))}
              </ul>
            </details>
          }
        />
      )}

      {firstScanRunning && (
        <Card>
          <EmptyState
            icon={<RefreshCw className="animate-spin text-accent" />}
            title="Checking every update source…"
            hint="Installed programs, drivers and Windows Update. This usually takes a few seconds."
          />
        </Card>
      )}

      {applying && progress && progress.total > 0 && (
        <Card>
          <div className="flex items-center justify-between text-sm mb-3">
            <span className="text-fg truncate">
              {progress.phase === "done"
                ? "Finishing up…"
                : progress.package
                  ? `Installing ${progress.package}`
                  : "Working…"}
            </span>
            <span className="text-muted flex-shrink-0 tabular-nums">
              {progress.current} of {progress.total}
            </span>
          </div>
          <Meter value={(progress.percent ?? (progress.current / progress.total) * 100) / 100} />
        </Card>
      )}

      {applyResult && <ApplyResults result={applyResult} dryRun={dryRun} />}

      {scanResult && actionable.length > 0 && (
        <Card padded={false}>
          <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-4 border-b border-line">
            <div>
              <h2 className="text-[15px] font-semibold">
                {actionable.length} update{actionable.length !== 1 ? "s" : ""} ready
              </h2>
              <p className="text-sm text-muted">
                Each installer is verified before it runs. Nothing is reinstalled or downgraded.
              </p>
            </div>
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-sm text-muted cursor-pointer">
                <Checkbox label="Dry run" checked={dryRun} onChange={setDryRun} />
                Dry run
              </label>
              {info?.platform === "windows" && (
                <label
                  className="flex items-center gap-2 text-sm text-muted cursor-pointer"
                  title="Needs administrator rights; skipped otherwise"
                >
                  <Checkbox
                    label="Create a restore point first"
                    checked={restorePoint}
                    onChange={setRestorePoint}
                  />
                  Restore point
                </label>
              )}
              <Button
                variant="primary"
                onClick={doApply}
                busy={applying}
                disabled={scan.loading || selected.length === 0}
                icon={<Download className="w-4 h-4" />}
              >
                {applying
                  ? "Installing…"
                  : `${dryRun ? "Simulate" : "Install"} ${selected.length}`}
              </Button>
            </div>
          </div>

          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-faint">
                <th className="w-12 py-2.5 pl-5">
                  <Checkbox
                    label={allSelected ? "Deselect all" : "Select all"}
                    checked={allSelected}
                    onChange={toggleAll}
                  />
                </th>
                <th className="py-2.5 font-medium">Name</th>
                <th className="py-2.5 font-medium">Source</th>
                <th className="py-2.5 font-medium">Version</th>
                <th className="py-2.5 pr-5 font-medium text-right">Size</th>
              </tr>
            </thead>
            <tbody>
              {actionable.map((u) => {
                const checked = !deselected.has(key(u));
                return (
                  <tr
                    key={key(u)}
                    onClick={() => toggle(u)}
                    className={`border-t border-line cursor-pointer transition-colors ${
                      checked ? "hover:bg-raised/50" : "opacity-60 hover:opacity-80"
                    }`}
                  >
                    <td className="py-3 pl-5" onClick={(e) => e.stopPropagation()}>
                      <Checkbox
                        label={`Include ${u.name}`}
                        checked={checked}
                        onChange={() => toggle(u)}
                      />
                    </td>
                    <td className="py-3 pr-4 font-medium text-fg max-w-[360px] truncate" title={u.name}>
                      {u.name}
                    </td>
                    <td className="py-3 pr-4">
                      <Badge>{sourceName(u.backend)}</Badge>
                    </td>
                    <td className="py-3 pr-4 font-mono text-[13px] whitespace-nowrap selectable">
                      <span className="text-muted">{u.installed}</span>
                      <ArrowRight className="inline w-3.5 h-3.5 mx-1.5 text-faint -mt-0.5" />
                      <span className="text-fg">{u.available}</span>
                    </td>
                    <td className="py-3 pr-5 text-right text-muted tabular-nums whitespace-nowrap">
                      {u.size_bytes ? formatSize(u.size_bytes) : "—"}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </Card>
      )}

      {/* Above the generic skipped list, because on an unelevated run every
          Windows update is "skipped" and would disappear into it. */}
      {scanResult && (
        <WindowsUpdateCard scan={scanResult} elevated={info?.elevated ?? false} />
      )}

      {scanResult && otherSkipped.length > 0 && (
        <Card
          title={`${otherSkipped.length} held back`}
          description="Found, but not installed — each for the reason shown."
          padded={false}
        >
          <ul className="divide-y divide-line border-t border-line">
            {otherSkipped.map((s, i) => (
              <li
                key={`${s.backend}:${s.id}:${i}`}
                className="flex items-center justify-between gap-4 px-5 py-3"
              >
                <span className="flex-1 min-w-0 text-sm text-fg truncate">{s.name}</span>
                {s.installed && s.available && s.available !== "1" && (
                  <span className="font-mono text-[13px] text-muted whitespace-nowrap selectable">
                    {s.installed} <ArrowRight className="inline w-3.5 h-3.5 mx-1 text-faint -mt-0.5" />{" "}
                    {s.available}
                  </span>
                )}
                <span className="w-[260px] text-sm text-muted text-right">{s.reason}</span>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {scanResult && scanResult.total === 0 && !scan.loading && (
        <Card>
          <EmptyState
            icon={<CheckCircle2 className="text-success" />}
            title="Everything is up to date"
            hint="Odysync checks again automatically on the schedule set in Settings."
          />
        </Card>
      )}

      {!scanResult && !scan.loading && !scan.error && (
        <Card>
          <EmptyState
            icon={<Package />}
            title="No scan yet"
            hint="Scan to see which installed programs, drivers and system components have updates."
          />
        </Card>
      )}
    </>
  );
}

function key(u: UpdateDto): string {
  return `${u.backend}:${u.id}`;
}

function ApplyResults({ result, dryRun }: { result: ApplyResultDto; dryRun: boolean }) {
  return (
    <Card
      title={dryRun ? "Dry run finished" : "Installation finished"}
      description={
        <span className="flex flex-wrap gap-x-4 gap-y-1">
          <span className="text-success">{result.updated} updated</span>
          {result.failed > 0 && <span className="text-danger">{result.failed} failed</span>}
          {result.skipped > 0 && <span>{result.skipped} skipped</span>}
          {result.reboot_required && (
            <span className="text-warning">Restart to finish</span>
          )}
        </span>
      }
      padded={false}
    >
      {result.entries.length > 0 && (
        <ul className="divide-y divide-line border-t border-line">
          {result.entries.map((e, i) => (
            <li key={`${e.name}-${i}`} className="flex items-start gap-3 px-5 py-3 text-sm">
              <OutcomeIcon status={e.status} />
              <span className="flex-1 text-fg">{e.name}</span>
              {e.detail && (
                <span className="text-muted max-w-[55%] text-right break-words selectable">
                  {e.detail}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

export function OutcomeIcon({ status }: { status: ApplyStatus }) {
  switch (status) {
    case "updated":
      return <CheckCircle2 className="w-4 h-4 text-success flex-shrink-0 mt-0.5" />;
    case "skipped":
      return <Clock className="w-4 h-4 text-faint flex-shrink-0 mt-0.5" />;
    default:
      return <XCircle className="w-4 h-4 text-danger flex-shrink-0 mt-0.5" />;
  }
}
