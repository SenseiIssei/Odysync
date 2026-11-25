import { useCallback, useMemo, useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  Download,
  Lock,
  LockOpen,
  Package,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import * as api from "../api";
import { useStore } from "../store-context";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  ConfirmBar,
  EmptyState,
  ErrorBar,
  PageHeader,
  WarningBar,
} from "../components/ui";
import { ApplyProgressView, ScanProgressView } from "../components/Progress";
import { FirmwareWarning } from "../components/FirmwareWarning";
import { OutcomeIcon } from "../components/OutcomeIcon";
import { errorText, formatSize } from "../format";
import { hardwareGroupOf, isHardwareBackend } from "../hardware-backends";
import { isCancelled, nameList } from "../scan-health";
import WindowsUpdateCard from "../components/WindowsUpdateCard";
import type { ApplyResultDto, SkippedDto, UpdateDto } from "../types";

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
  const { scan, apply, sysInfo, config, backends } = useStore();
  const [finished, setFinished] = useState<{
    result: ApplyResultDto;
    dryRun: boolean;
    cancelled: boolean;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dryRun, setDryRun] = useState(false);
  // The choice for this one run; until it is made, the setting decides.
  const [restoreChoice, setRestoreChoice] = useState<boolean | null>(null);
  const restorePoint = restoreChoice ?? config.data?.restore_point ?? true;
  // The selection the firmware question was asked about. Any other selection
  // has not been asked about, which closes the question.
  const [confirmFor, setConfirmFor] = useState<string | null>(null);

  // Choices the user made, layered over each row's default, so updates that
  // appear in a later scan need no syncing of a selection set.
  const [choices, setChoices] = useState<Record<string, boolean>>({});

  const scanResult = scan.data;
  const actionable = useMemo(() => scanResult?.actionable ?? [], [scanResult]);
  /**
   * What is ticked without being asked: only what one-click care would install
   * too. Drivers, firmware, OS upgrades and developer packages are for a person
   * to choose, and so is anything whose source is not known (yet).
   */
  const preselected = useCallback(
    (u: UpdateDto) => {
      const source = backends.data.find((b) => b.kind === u.backend);
      return (
        source !== undefined &&
        !source.attended &&
        !source.developer &&
        hardwareGroupOf(u.backend) !== "firmware"
      );
    },
    [backends.data],
  );
  const isSelected = useCallback(
    (u: UpdateDto) => choices[key(u)] ?? preselected(u),
    [choices, preselected],
  );
  const selected = useMemo(() => actionable.filter(isSelected), [actionable, isSelected]);
  const selection = selected.map(key).join("|");
  const firmware = selected.some((u) => hardwareGroupOf(u.backend) === "firmware");
  const confirming = confirmFor !== null && confirmFor === selection;
  const sourceName = useCallback(
    (backend: string) =>
      SOURCE_LABELS[backend] ?? backends.data.find((b) => b.kind === backend)?.name ?? backend,
    [backends.data],
  );

  const doScan = useCallback(async () => {
    setError(null);
    setFinished(null);
    await scan.refresh();
  }, [scan]);

  const install = async () => {
    setConfirmFor(null);
    setError(null);
    setFinished(null);
    try {
      const { result, cancelled } = await apply.run({
        updates: selected,
        dry_run: dryRun,
        restore_point: restorePoint,
      });
      setFinished({ result, dryRun, cancelled });
      if (!dryRun) setRestoreChoice(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const start = () => {
    if (selected.length === 0) return;
    if (firmware && !dryRun) setConfirmFor(selection);
    else void install();
  };

  const toggle = (u: UpdateDto) =>
    setChoices((prev) => ({ ...prev, [key(u)]: !isSelected(u) }));

  const allSelected = actionable.length > 0 && selected.length === actionable.length;
  const toggleAll = () =>
    setChoices((prev) => ({
      ...prev,
      ...Object.fromEntries(actionable.map((u) => [key(u), !allSelected])),
    }));

  /** Hold or release a package, and show it in the list without a new scan. */
  const changeHold = async (u: UpdateDto | SkippedDto, held: boolean) => {
    setError(null);
    try {
      const request = { backend: u.backend, id: nativeId(u) };
      await (held ? api.hold(request) : api.unhold(request));
      await config.reload();
    } catch (e) {
      setError(errorText(e));
      return;
    }
    const now = scan.data;
    if (!now) return;
    const { backend, id, name, installed, available } = u;
    const row: SkippedDto = {
      backend,
      id,
      name,
      installed,
      available,
      reason: held ? HELD : "released, offered again after the next scan",
    };
    const isRow = (x: { backend: string; id: string }) => x.backend === backend && x.id === id;
    scan.set({
      ...now,
      actionable: now.actionable.filter((x) => !isRow(x)),
      skipped: [...now.skipped.filter((x) => !isRow(x)), row],
    });
  };

  const info = sysInfo.data;
  const firstScanRunning = scan.loading && scan.loadedAt === null;
  const health = scan.health;
  const unanswered = health.state === "partial" || health.state === "failed" ? health.failed : [];
  const unansweredNames = unanswered.map((f) => sourceName(f.backend));
  const retry = (
    <Button
      size="sm"
      onClick={() => void doScan()}
      busy={scan.loading}
      disabled={apply.running}
      icon={<RefreshCw className="w-3.5 h-3.5" />}
    >
      Try again
    </Button>
  );
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
            onClick={() => void doScan()}
            busy={scan.loading}
            disabled={apply.running}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            {scan.loading ? "Scanning…" : "Scan again"}
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {health.state === "failed" && !scan.loading && (
        <ErrorBar
          action={retry}
          message={
            <>
              Updates could not be checked: {health.error}
              {scan.error !== null && scanResult !== null && scan.loadedAt !== null && (
                <> What is listed below was found on {new Date(scan.loadedAt).toLocaleString()}.</>
              )}
            </>
          }
        />
      )}

      {/* A source that errors out is not the same as one without updates. */}
      {unanswered.length > 0 && !scan.loading && (
        <WarningBar
          action={health.state === "partial" ? retry : undefined}
          message={
            <details>
              <summary className="cursor-pointer">
                {unanswered.every(isCancelled)
                  ? `The check was cancelled before ${nameList(unansweredNames)} answered, so the list may be incomplete`
                  : `${nameList(unansweredNames)} could not be checked, so the list may be incomplete`}
              </summary>
              <ul className="mt-2 space-y-1 text-muted">
                {unanswered.map((f) => (
                  <li key={f.backend} className="text-xs break-words selectable">
                    {sourceName(f.backend)}: {errorText(f.error)}
                  </li>
                ))}
              </ul>
            </details>
          }
        />
      )}

      {scan.loading && (
        <Card>
          {firstScanRunning && (
            <EmptyState
              icon={<RefreshCw className="animate-spin text-accent" />}
              title="Checking every update source…"
              hint="Installed programs, drivers and Windows Update. Windows Update alone can take a minute or two to answer."
            />
          )}
          <ScanProgressView />
        </Card>
      )}

      <ApplyProgressView />

      {finished && <ApplyResults {...finished} />}

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
                    onChange={setRestoreChoice}
                  />
                  Restore point
                </label>
              )}
              <Button
                variant="primary"
                onClick={start}
                busy={apply.running}
                disabled={
                  scan.loading || confirming || health.state === "failed" || selected.length === 0
                }
                icon={<Download className="w-4 h-4" />}
              >
                {apply.running
                  ? "Installing…"
                  : `${dryRun ? "Simulate" : "Install"} ${selected.length}`}
              </Button>
            </div>
          </div>
          {confirming && (
            <div className="px-5 py-4 border-b border-line">
              <ConfirmBar
                message={<FirmwareWarning />}
                confirmLabel={`Install ${selected.length}, including firmware`}
                onConfirm={() => void install()}
                onCancel={() => setConfirmFor(null)}
              />
            </div>
          )}

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
                <th className="py-2.5 pr-4 font-medium text-right">Size</th>
                <th className="py-2.5 pr-5">
                  <span className="sr-only">Hold</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {actionable.map((u) => {
                const checked = isSelected(u);
                return (
                  <tr
                    key={key(u)}
                    onClick={() => toggle(u)}
                    className="border-t border-line cursor-pointer transition-colors hover:bg-raised/50"
                  >
                    <td className="py-3 pl-5" onClick={(e) => e.stopPropagation()}>
                      <Checkbox
                        label={`Include ${u.name}`}
                        checked={checked}
                        onChange={() => toggle(u)}
                      />
                    </td>
                    <td
                      className={`py-3 pr-4 max-w-[360px] truncate ${checked ? "font-medium text-fg" : "text-muted"}`}
                      title={u.name}
                    >
                      {u.name}
                    </td>
                    <td className="py-3 pr-4">
                      <span className="inline-flex flex-wrap gap-1.5">
                        <Badge>{sourceName(u.backend)}</Badge>
                        <KindBadge
                          backend={u.backend}
                          attended={backends.data.find((b) => b.kind === u.backend)?.attended}
                          developer={backends.data.find((b) => b.kind === u.backend)?.developer}
                        />
                      </span>
                    </td>
                    <td className="py-3 pr-4 font-mono text-[13px] whitespace-nowrap selectable">
                      <span className="text-muted">{u.installed}</span>
                      <ArrowRight aria-hidden className="inline w-3.5 h-3.5 mx-1.5 text-faint -mt-0.5" />
                      <span className="sr-only"> to </span>
                      <span className={checked ? "text-fg" : "text-muted"}>{u.available}</span>
                    </td>
                    <td className="py-3 pr-4 text-right text-muted tabular-nums whitespace-nowrap">
                      {u.size_bytes ? formatSize(u.size_bytes) : "-"}
                    </td>
                    <td className="py-3 pr-5 text-right" onClick={(e) => e.stopPropagation()}>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={apply.running || scan.loading}
                        aria-label={`Hold ${u.name}`}
                        title="Do not offer this package again until it is released"
                        icon={<Lock className="w-3.5 h-3.5" />}
                        onClick={() => void changeHold(u, true)}
                      >
                        Hold
                      </Button>
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
          description="Found, but not installed, each for the reason shown."
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
                    {s.installed}{" "}
                    <ArrowRight aria-hidden className="inline w-3.5 h-3.5 mx-1 text-faint -mt-0.5" />
                    <span className="sr-only"> to </span> {s.available}
                  </span>
                )}
                <span className="w-[260px] text-sm text-muted text-right">{s.reason}</span>
                {s.reason.startsWith(HELD) ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={apply.running || scan.loading}
                    aria-label={`Release ${s.name}`}
                    icon={<LockOpen className="w-3.5 h-3.5" />}
                    onClick={() => void changeHold(s, false)}
                  >
                    Release
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={apply.running || scan.loading}
                    aria-label={`Hold ${s.name}`}
                    title="Do not offer this package again until it is released"
                    icon={<Lock className="w-3.5 h-3.5" />}
                    onClick={() => void changeHold(s, true)}
                  >
                    Hold
                  </Button>
                )}
              </li>
            ))}
          </ul>
        </Card>
      )}

      {scanResult && scanResult.total === 0 && !scan.loading && health.state === "complete" && (
        <Card>
          <EmptyState
            icon={<CheckCircle2 className="text-success" />}
            title="Everything is up to date"
            hint={
              config.data && config.data.scan_interval_hours > 0
                ? `Odysync checks again every ${config.data.scan_interval_hours} hours, as set in Settings.`
                : "Odysync checks again when you ask, or on a schedule you set."
            }
          />
        </Card>
      )}

      {/* Nothing found by the sources that answered is not "up to date". */}
      {scanResult && scanResult.total === 0 && !scan.loading && health.state === "partial" && (
        <Card>
          <EmptyState
            icon={<TriangleAlert className="text-warning" />}
            title="No updates from the sources that answered"
            hint={`${nameList(unansweredNames)} ${unansweredNames.length === 1 ? "was" : "were"} not checked, so there may be updates nobody saw.`}
            action={retry}
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

/** How the command layer words a hold; a note may follow. */
const HELD = "held by policy";

function key(u: UpdateDto): string {
  return `${u.backend}:${u.id}`;
}

/**
 * The id a hold is stored under. A scan reports the full `backend:id`, the
 * hold commands take the two apart.
 */
function nativeId(u: { backend: string; id: string }): string {
  const prefix = `${u.backend}:`;
  return u.id.startsWith(prefix) ? u.id.slice(prefix.length) : u.id;
}

/** Why a row is not ticked by itself. */
function KindBadge({
  backend,
  attended,
  developer,
}: {
  backend: string;
  attended?: boolean;
  developer?: boolean;
}) {
  if (hardwareGroupOf(backend) === "firmware") return <Badge tone="warning">Firmware</Badge>;
  if (attended) return <Badge>{isHardwareBackend(backend) ? "Driver" : "System upgrade"}</Badge>;
  if (developer) return <Badge>Developer</Badge>;
  return null;
}

function ApplyResults({
  result,
  dryRun,
  cancelled,
}: {
  result: ApplyResultDto;
  dryRun: boolean;
  cancelled: boolean;
}) {
  const title = cancelled
    ? "Installation cancelled"
    : dryRun
      ? "Dry run finished"
      : result.failed > 0
        ? "Finished, but not everything was installed"
        : "Installation finished";
  return (
    <Card
      title={title}
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
