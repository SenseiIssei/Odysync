import { useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowRight,
  Archive,
  CheckCircle2,
  Download,
  RefreshCw,
  RotateCcw,
  Wrench,
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
  InfoBar,
  PageHeader,
  SuccessBar,
  WarningBar,
} from "../components/ui";
import { errorText, formatDate, formatSize } from "../format";
import { GROUP_LABELS, hardwareGroupOf, isHardwareBackend } from "../hardware-backends";
import type { ApplyResultDto, ComponentStatus, UpdateDto } from "../types";
import { ApplyProgressView, ScanProgressView } from "../components/Progress";
import { FirmwareWarning } from "../components/FirmwareWarning";
import { OutcomeIcon } from "../components/OutcomeIcon";

export default function Drivers() {
  const { scan, apply, sysInfo, backends } = useStore();
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ApplyResultDto | null>(null);
  const [deselected, setDeselected] = useState<Set<string>>(new Set());
  // The selection the firmware question was asked about; a different one has
  // not been asked about, which closes the question.
  const [confirmFor, setConfirmFor] = useState<string | null>(null);

  const info = sysInfo.data;
  const elevated = info?.elevated ?? false;
  const windows = (info?.platform ?? "windows") === "windows";

  const updates = useMemo(
    () => (scan.data?.actionable ?? []).filter((u) => isHardwareBackend(u.backend)),
    [scan.data],
  );
  const waiting = useMemo(
    () => (scan.data?.skipped ?? []).filter((s) => isHardwareBackend(s.backend)),
    [scan.data],
  );
  const failures = useMemo(
    () => (scan.data?.failed_backends ?? []).filter((f) => isHardwareBackend(f.backend)),
    [scan.data],
  );
  const scanFailed = scan.health.state === "failed";
  // Failures of sources that carry no drivers do not matter on this page.
  const allChecked = !scanFailed && scan.health.state !== "unknown" && failures.length === 0;
  const selected = useMemo(
    () => updates.filter((u) => !deselected.has(key(u))),
    [updates, deselected],
  );
  const selectedHasFirmware = selected.some((u) => hardwareGroupOf(u.backend) === "firmware");
  const selection = selected.map(key).join("|");
  const confirming = confirmFor !== null && confirmFor === selection;

  const toggle = (u: UpdateDto) =>
    setDeselected((prev) => {
      const next = new Set(prev);
      const k = key(u);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const install = async () => {
    setConfirmFor(null);
    setError(null);
    setResult(null);
    try {
      const { result: r } = await apply.run({
        updates: selected,
        dry_run: false,
        // Hardware changes are the ones you most want to be able to roll back.
        restore_point: true,
      });
      setResult(r);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const start = () => {
    if (selected.length === 0) return;
    if (selectedHasFirmware) setConfirmFor(selection);
    else void install();
  };

  return (
    <>
      <PageHeader
        title="Drivers"
        subtitle="Driver updates from Windows Update and the hardware makers, the runtimes games and apps need, and devices that are not working."
        actions={
          <Button
            onClick={() => void scan.refresh()}
            busy={scan.loading}
            disabled={apply.running}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Rescan
          </Button>
        }
      />

      {windows && !elevated && (
        <InfoBar message="Checking for drivers works without administrator rights; installing them does not. Use Run as admin in the title bar when you are ready to install." />
      )}

      {scanFailed && !scan.loading && scan.health.state === "failed" && (
        <ErrorBar
          message={`Driver updates could not be checked: ${scan.health.error}`}
          action={
            <Button
              size="sm"
              onClick={() => void scan.refresh()}
              icon={<RefreshCw className="w-3.5 h-3.5" />}
            >
              Try again
            </Button>
          }
        />
      )}
      {failures.length > 0 && !scan.loading && (
        <WarningBar
          message={
            <>
              {failures.length} driver source{failures.length === 1 ? "" : "s"} could not be
              checked:{" "}
              <span className="text-xs">
                {failures
                  .map(
                    (f) =>
                      `${backends.data.find((b) => b.kind === f.backend)?.name ?? f.backend}: ${errorText(f.error)}`,
                  )
                  .join("; ")}
              </span>
            </>
          }
        />
      )}

      {error && <ErrorBar message={error} />}

      <ApplyProgressView />

      {result && (
        <Card
          title={
            result.failed > 0
              ? "Finished, but not everything was installed"
              : "Driver installation finished"
          }
          description={`${result.updated} updated, ${result.failed} failed${result.skipped > 0 ? `, ${result.skipped} skipped` : ""}.${result.reboot_required ? " Restart to finish." : ""}`}
          padded={false}
        >
          <ul className="divide-y divide-line border-t border-line">
            {result.entries.map((e, i) => (
              <li key={`${e.name}-${i}`} className="flex items-start gap-3 px-5 py-3 text-sm">
                <OutcomeIcon status={e.status} />
                <span className="flex-1">{e.name}</span>
                <span className="text-muted text-right max-w-[55%] break-words">{e.detail}</span>
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Card
        title="Driver updates"
        description="WHQL-certified drivers only, from Windows Update or straight from the maker. A restore point is taken first."
        padded={false}
        actions={
          updates.length > 0 ? (
            <Button
              variant="primary"
              onClick={start}
              busy={apply.running}
              disabled={scan.loading || scanFailed || confirming || selected.length === 0}
              icon={<Download className="w-4 h-4" />}
            >
              {apply.running ? "Installing…" : `Install ${selected.length}`}
            </Button>
          ) : undefined
        }
      >
        {confirming && (
          <div className="px-5 pb-4">
            <ConfirmBar
              message={<FirmwareWarning />}
              confirmLabel={`Install ${selected.length}, including firmware`}
              onConfirm={() => void install()}
              onCancel={() => setConfirmFor(null)}
            />
          </div>
        )}
        {scan.loading && (
          <div className="px-5 pb-4">
            <ScanProgressView />
          </div>
        )}
        {updates.length === 0 && waiting.length === 0 ? (
          <div className="border-t border-line">
            <EmptyState
              icon={
                scan.loading ? (
                  <RefreshCw className="animate-spin text-accent" />
                ) : allChecked ? (
                  <CheckCircle2 className="text-success" />
                ) : (
                  <AlertTriangle className="text-warning" />
                )
              }
              title={
                scan.loading
                  ? "Checking driver sources…"
                  : allChecked
                    ? "No driver updates"
                    : scan.health.state === "unknown"
                      ? "Not checked yet"
                      : "Not every driver source could be checked"
              }
              hint={
                scan.loading
                  ? undefined
                  : allChecked
                    ? "Windows Update and the hardware makers have nothing newer for this machine."
                    : "There may be driver updates nobody saw. Scan again to check."
              }
            />
          </div>
        ) : (
          <ul className="divide-y divide-line border-t border-line">
            {updates.map((u) => {
              const group = hardwareGroupOf(u.backend);
              const checked = !deselected.has(key(u));
              return (
                <li key={key(u)} className="flex items-center gap-3 px-5 py-3 text-sm">
                  <Checkbox label={`Include ${u.name}`} checked={checked} onChange={() => toggle(u)} />
                  <span className="flex-1 min-w-0 truncate font-medium" title={u.name}>
                    {u.name}
                  </span>
                  {group && (
                    <Badge tone={group === "firmware" ? "warning" : "neutral"}>
                      {GROUP_LABELS[group]}
                    </Badge>
                  )}
                  <VersionJump from={u.installed} to={u.available} />
                </li>
              );
            })}
            {waiting.map((s, i) => (
              <li key={`${s.backend}:${s.id}:${i}`} className="flex items-center gap-3 px-5 py-3 text-sm">
                <span className="w-4" />
                <span className="flex-1 min-w-0 truncate text-muted" title={s.name}>
                  {s.name}
                </span>
                <span className="text-sm text-faint">{s.reason}</span>
                {s.installed && s.available && s.available !== "1" && (
                  <VersionJump from={s.installed} to={s.available} />
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      <DeviceProblems />
      {windows && <Runtimes elevated={elevated} />}
      {windows && <DriverBackups elevated={elevated} />}
    </>
  );
}

function key(u: UpdateDto) {
  return `${u.backend}:${u.id}`;
}

function VersionJump({ from, to }: { from: string; to: string }) {
  return (
    <span className="font-mono text-[13px] whitespace-nowrap selectable">
      <span className="text-muted">{from || "-"}</span>
      <span className="sr-only"> to </span>
      <ArrowRight aria-hidden className="inline w-3.5 h-3.5 mx-1.5 text-faint -mt-0.5" />
      <span className="text-fg">{to}</span>
    </span>
  );
}

function DeviceProblems() {
  const { devices } = useStore();
  return (
    <Card
      title="Devices"
      description="What Device Manager knows but hides behind a number, in plain words."
      padded={false}
      actions={
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void devices.refresh()}
          busy={devices.loading}
          icon={<RefreshCw className="w-3.5 h-3.5" />}
        >
          Check again
        </Button>
      }
    >
      {devices.error && (
        <div className="px-5 pb-4">
          <ErrorBar message={`Devices could not be checked: ${devices.error}`} />
        </div>
      )}
      {devices.data.length === 0 ? (
        devices.error === null && (
          <div className="border-t border-line px-5 py-4 text-sm text-muted flex items-center gap-2">
            {devices.loadedAt === null ? (
              "Checking devices…"
            ) : (
              <>
                <CheckCircle2 aria-hidden className="w-4 h-4 text-success" />
                Every device reports that it is working.
              </>
            )}
          </div>
        )
      ) : (
        <ul className="divide-y divide-line border-t border-line">
          {devices.data.map((d) => (
            <li key={d.deviceId} className="px-5 py-4">
              <div className="flex items-center gap-2">
                <AlertTriangle aria-hidden className="w-4 h-4 text-warning flex-shrink-0" />
                <span className="text-sm font-medium text-fg">{d.name}</span>
                <Badge>{d.class || "Device"}</Badge>
                <span className="text-xs text-faint ml-auto">code {d.code}</span>
              </div>
              <p className="text-sm text-muted mt-1.5 ml-6">{d.meaning}</p>
              <p className="text-sm text-fg mt-0.5 ml-6">{d.suggestion}</p>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

const STATE_BADGE: Record<ComponentStatus["state"], { tone: "success" | "warning" | "danger" | "neutral"; label: string }> = {
  installed: { tone: "success", label: "Installed" },
  outdated: { tone: "warning", label: "Update available" },
  missing: { tone: "neutral", label: "Not installed" },
  "out-of-support": { tone: "danger", label: "Out of support" },
};

function Runtimes({ elevated }: { elevated: boolean }) {
  const { components } = useStore();
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const install = async (c: ComponentStatus) => {
    setBusy(c.id);
    setMessage(null);
    try {
      const text = await api.componentInstall(c.id);
      setMessage({ ok: true, text });
      await components.reload();
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title="Game & app runtimes"
      description="Visual C++, DirectX and .NET, installed or repaired with Microsoft's own installers, never with loose DLLs from a download site."
      padded={false}
    >
      {message && (
        <div className="px-5 pb-4">
          {message.ok ? <SuccessBar message={message.text} /> : <ErrorBar message={message.text} />}
        </div>
      )}
      {components.error && (
        <div className="px-5 pb-4">
          <ErrorBar
            message={`Runtimes could not be checked: ${components.error}`}
            action={
              <Button
                size="sm"
                busy={components.loading}
                onClick={() => void components.refresh()}
                icon={<RefreshCw className="w-3.5 h-3.5" />}
              >
                Try again
              </Button>
            }
          />
        </div>
      )}
      <ul className="divide-y divide-line border-t border-line">
        {components.loadedAt === null && components.error === null && (
          <li className="px-5 py-4 text-sm text-muted">Checking runtimes…</li>
        )}
        {components.data.map((c) => {
          const badge = STATE_BADGE[c.state];
          const action =
            c.state === "outdated" ? "Update" : c.state === "missing" ? "Install" : "Repair";
          return (
            <li key={c.id} className="flex items-center gap-4 px-5 py-3.5">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-fg">{c.name}</span>
                  <Badge tone={badge.tone}>{badge.label}</Badge>
                </div>
                <p className="text-sm text-muted mt-0.5">{c.note}</p>
              </div>
              <span className="font-mono text-[13px] text-muted whitespace-nowrap selectable">
                {c.state === "outdated" && c.installed && c.latest ? (
                  <VersionJump from={c.installed} to={c.latest} />
                ) : (
                  c.installed ?? ""
                )}
              </span>
              {c.installable && (
                <Button
                  size="sm"
                  variant={c.state === "installed" ? "ghost" : "secondary"}
                  busy={busy === c.id}
                  disabled={!elevated || (busy !== null && busy !== c.id)}
                  title={elevated ? undefined : "Needs administrator rights"}
                  icon={c.state === "installed" ? <Wrench className="w-3.5 h-3.5" /> : <Download className="w-3.5 h-3.5" />}
                  onClick={() => void install(c)}
                >
                  {action}
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </Card>
  );
}

function DriverBackups({ elevated }: { elevated: boolean }) {
  const { driverBackups } = useStore();
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<string | null>(null);

  const run = async (label: string, work: () => Promise<{ ok: boolean; text: string }>) => {
    setBusy(label);
    setMessage(null);
    try {
      setMessage(await work());
      await driverBackups.reload();
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const backupNow = () =>
    run("backup", async () => {
      const b = await api.driverBackupCreate("taken from the Odysync app");
      return {
        // A backup with holes in it is not a success.
        ok: b.complete,
        text: `Saved ${b.package_count} driver packages (${formatSize(b.size_bytes)}).${b.complete ? "" : " Some packages could not be exported, so this backup is incomplete."}`,
      };
    });

  const restore = (id: string) => {
    setConfirmRestore(null);
    void run(`restore:${id}`, async () => {
      const r = await api.driverBackupRestore(id);
      const parts = [
        `${r.restored.length} restored`,
        `${r.already_current.length} already current`,
        r.failed.length > 0 ? `${r.failed.length} failed` : null,
      ].filter(Boolean);
      return {
        ok: r.failed.length === 0,
        text: `${parts.join(", ")}.${r.reboot_required ? " Restart to finish." : ""}${r.failed.length > 0 ? ` Not restored: ${r.failed.join(", ")}.` : ""}`,
      };
    });
  };

  return (
    <Card
      title="Driver backups"
      description="A copy of every third-party driver package, so a driver update can be undone. Windows keeps only the single previous driver on its own."
      padded={false}
      actions={
        <Button
          size="sm"
          onClick={() => void backupNow()}
          busy={busy === "backup"}
          disabled={busy !== null && busy !== "backup"}
          icon={<Archive className="w-3.5 h-3.5" />}
        >
          Back up now
        </Button>
      }
    >
      {message && (
        <div className="px-5 pb-4">
          {message.ok ? <SuccessBar message={message.text} /> : <ErrorBar message={message.text} />}
        </div>
      )}
      {driverBackups.error && (
        <div className="px-5 pb-4">
          <ErrorBar message={`The backups could not be read: ${driverBackups.error}`} />
        </div>
      )}
      {driverBackups.data.length === 0 ? (
        driverBackups.error === null && (
          <div className="border-t border-line px-5 py-4 text-sm text-muted">
            {driverBackups.loadedAt === null
              ? "Reading the backups…"
              : "No backups yet. A full backup is usually 3 to 6 GB, so Odysync takes one only when you ask."}
          </div>
        )
      ) : (
        <ul className="divide-y divide-line border-t border-line">
          {driverBackups.data.map((b) => (
            <li key={b.id} className="px-5 py-3.5">
              <div className="flex items-center gap-4">
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium text-fg">{formatDate(b.created_at)}</div>
                  <div className="text-sm text-muted">
                    {b.package_count} packages · {formatSize(b.size_bytes)} · {b.reason}
                    {!b.complete && " · incomplete"}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  busy={busy === `restore:${b.id}`}
                  disabled={!elevated || busy !== null || confirmRestore === b.id}
                  title={elevated ? undefined : "Needs administrator rights"}
                  aria-label={`Restore the backup of ${formatDate(b.created_at)}`}
                  icon={<RotateCcw className="w-3.5 h-3.5" />}
                  onClick={() => setConfirmRestore(b.id)}
                >
                  Restore
                </Button>
              </div>
              {confirmRestore === b.id && (
                <ConfirmBar
                  className="mt-3"
                  message="Restoring adds the saved packages again and lets Windows install them where they fit. It does not force a downgrade: if a newer driver is still installed, Windows may keep preferring it."
                  confirmLabel="Restore these drivers"
                  onConfirm={() => restore(b.id)}
                  onCancel={() => setConfirmRestore(null)}
                />
              )}
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
