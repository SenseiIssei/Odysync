import { useCallback, useMemo, useState } from "react";
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
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  EmptyState,
  ErrorBar,
  InfoBar,
  PageHeader,
  SuccessBar,
  WarningBar,
  errorText,
  formatDate,
  formatSize,
} from "../components/ui";
import { GROUP_LABELS, hardwareGroupOf, isHardwareBackend } from "../hardware-backends";
import type { ApplyResultDto, ComponentStatus, UpdateDto } from "../types";
import { OutcomeIcon } from "./Updates";

export default function Drivers() {
  const { scan, sysInfo, history, backup } = useStore();
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ApplyResultDto | null>(null);
  const [deselected, setDeselected] = useState<Set<string>>(new Set());
  const [confirmFirmware, setConfirmFirmware] = useState(false);

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
  const selected = useMemo(
    () => updates.filter((u) => !deselected.has(key(u))),
    [updates, deselected],
  );
  const selectedHasFirmware = selected.some((u) => hardwareGroupOf(u.backend) === "firmware");

  const toggle = (u: UpdateDto) =>
    setDeselected((prev) => {
      const next = new Set(prev);
      const k = key(u);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const install = useCallback(async () => {
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
    <>
      <PageHeader
        title="Drivers"
        subtitle="Driver updates from Windows Update and the hardware makers, the runtimes games and apps need, and devices that are not working."
        actions={
          <Button
            onClick={() => void scan.refresh()}
            busy={scan.loading}
            disabled={applying}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Rescan
          </Button>
        }
      />

      {windows && !elevated && (
        <InfoBar message="Checking for drivers works without administrator rights; installing them does not. Use Run as admin in the title bar when you are ready to install." />
      )}

      {failures.length > 0 && (
        <WarningBar
          message={
            <>
              {failures.length} driver source{failures.length === 1 ? "" : "s"} could not be
              checked:{" "}
              <span className="font-mono text-xs">
                {failures.map((f) => `${f.backend}: ${f.error}`).join("; ")}
              </span>
            </>
          }
        />
      )}

      {error && <ErrorBar message={error} />}

      {result && (
        <Card
          title="Driver installation finished"
          description={`${result.updated} updated, ${result.failed} failed${result.reboot_required ? " — restart to finish" : ""}.`}
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
              variant={confirmFirmware ? "danger" : "primary"}
              onClick={install}
              busy={applying}
              disabled={scan.loading || selected.length === 0}
              icon={<Download className="w-4 h-4" />}
            >
              {confirmFirmware ? "Confirm: includes firmware" : `Install ${selected.length}`}
            </Button>
          ) : undefined
        }
      >
        {confirmFirmware && (
          <div className="px-5 pb-4">
            <WarningBar message="Your selection includes firmware or BIOS updates. Keep the machine on mains power and do not shut it down while they run — an interrupted firmware update can leave a device unbootable." />
          </div>
        )}
        {updates.length === 0 && waiting.length === 0 ? (
          <div className="border-t border-line">
            <EmptyState
              icon={
                scan.loading ? (
                  <RefreshCw className="animate-spin text-accent" />
                ) : (
                  <CheckCircle2 className="text-success" />
                )
              }
              title={scan.loading ? "Checking driver sources…" : "No driver updates"}
              hint={
                scan.loading
                  ? undefined
                  : "Windows Update and the hardware makers have nothing newer for this machine."
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
      <span className="text-muted">{from || "—"}</span>
      <ArrowRight className="inline w-3.5 h-3.5 mx-1.5 text-faint -mt-0.5" />
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
      {devices.data.length === 0 ? (
        <div className="border-t border-line px-5 py-4 text-sm text-muted flex items-center gap-2">
          <CheckCircle2 className="w-4 h-4 text-success" />
          {devices.loadedAt === null ? "Checking devices…" : "Every device reports that it is working."}
        </div>
      ) : (
        <ul className="divide-y divide-line border-t border-line">
          {devices.data.map((d) => (
            <li key={d.deviceId} className="px-5 py-4">
              <div className="flex items-center gap-2">
                <AlertTriangle className="w-4 h-4 text-warning flex-shrink-0" />
                <span className="text-sm font-medium text-fg">{d.name}</span>
                <Badge>{d.class || "Device"}</Badge>
                <span className="text-xs text-faint ml-auto">code {d.code}</span>
              </div>
              <p className="text-sm text-muted mt-1.5 ml-6">{d.meaning}</p>
              <p className="text-sm text-fg/90 mt-0.5 ml-6">{d.suggestion}</p>
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
      await components.refresh();
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title="Game & app runtimes"
      description="Visual C++, DirectX and .NET, installed or repaired with Microsoft's own installers — never loose DLLs from a download site."
      padded={false}
    >
      {message && (
        <div className="px-5 pb-4">
          {message.ok ? <SuccessBar message={message.text} /> : <ErrorBar message={message.text} />}
        </div>
      )}
      <ul className="divide-y divide-line border-t border-line">
        {components.loadedAt === null && (
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

  const run = async (label: string, work: () => Promise<string>) => {
    setBusy(label);
    setMessage(null);
    try {
      setMessage({ ok: true, text: await work() });
      await driverBackups.refresh();
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const backupNow = () =>
    run("backup", async () => {
      const b = await api.driverBackupCreate("taken from the Odysync app");
      return `Saved ${b.package_count} driver packages (${formatSize(b.size_bytes)}).${b.complete ? "" : " Some packages could not be exported."}`;
    });

  const restore = (id: string) => {
    if (confirmRestore !== id) {
      setConfirmRestore(id);
      return;
    }
    setConfirmRestore(null);
    void run(`restore:${id}`, async () => {
      const r = await api.driverBackupRestore(id);
      const parts = [
        `${r.restored.length} restored`,
        `${r.already_current.length} already current`,
        r.failed.length > 0 ? `${r.failed.length} failed` : null,
      ].filter(Boolean);
      return `${parts.join(", ")}.${r.reboot_required ? " Restart to finish." : ""}`;
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
      {driverBackups.data.length === 0 ? (
        <div className="border-t border-line px-5 py-4 text-sm text-muted">
          No backups yet. A full backup is usually 3–6 GB, so Odysync does not take one
          automatically unless you turn that on in Settings.
        </div>
      ) : (
        <ul className="divide-y divide-line border-t border-line">
          {driverBackups.data.map((b) => (
            <li key={b.id} className="flex items-center gap-4 px-5 py-3.5">
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium text-fg">{formatDate(b.created_at)}</div>
                <div className="text-sm text-muted">
                  {b.package_count} packages · {formatSize(b.size_bytes)} · {b.reason}
                  {!b.complete && " · incomplete"}
                </div>
              </div>
              <Button
                size="sm"
                variant={confirmRestore === b.id ? "danger" : "ghost"}
                busy={busy === `restore:${b.id}`}
                disabled={!elevated || (busy !== null && busy !== `restore:${b.id}`)}
                title={elevated ? undefined : "Needs administrator rights"}
                icon={<RotateCcw className="w-3.5 h-3.5" />}
                onClick={() => restore(b.id)}
              >
                {confirmRestore === b.id ? "Confirm restore" : "Restore"}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {confirmRestore && (
        <p className="px-5 py-3 border-t border-line text-sm text-muted">
          Restoring re-adds the saved packages and lets Windows install them where they fit.
          It does not force a downgrade: if a newer driver is still installed, Windows may
          keep preferring it.
        </p>
      )}
    </Card>
  );
}
