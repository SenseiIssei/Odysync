import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  ArrowRight,
  Brush,
  CheckCircle2,
  Circle,
  CircleAlert,
  HardDrive,
  Loader2,
  MemoryStick,
  Package,
  PartyPopper,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  TriangleAlert,
  Wand2,
} from "lucide-react";
import * as api from "../api";
import { safeListen } from "../events";
import { useStore, type Resource } from "../store";
import { Badge, Button, Card, Segmented, Switch } from "../components/ui";
import { CareRing, type RingMode, type RingTone } from "../components/CareRing";
import { assess, safeCleanupBytes, safeCleanupItems, type CareIssue } from "../care";
import { formatSize } from "../format";
import type { Tab } from "../nav";

/** The schedule automatic care runs under, separate from a plain update schedule. */
export const CARE_TASK = "OdysyncCare";
const CARE_FREQUENCY_KEY = "odysync.careFrequency";

interface Progress {
  phase: string;
  current: number;
  total: number;
  package?: string | null;
  percent?: number | null;
}

interface FixOutcome {
  updated: number;
  failed: number;
  cleared: number;
  reboot: boolean;
  error: string | null;
}

type StepState = "waiting" | "running" | "done";

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Whether a set of background resources is still loading. */
function stepState(resources: Resource<unknown>[]): StepState {
  if (resources.some((r) => r.loading)) return "running";
  return "done";
}

export default function Dashboard({ onNavigate }: { onNavigate: (tab: Tab) => void }) {
  const store = useStore();
  const {
    sysInfo,
    backends,
    scan,
    cleanup,
    devices,
    components,
    security,
    defender,
    health,
    config,
    history,
    packages,
  } = store;

  // ── What the machine looks like ──────────────────────────────────────────
  const attendedKinds = useMemo(
    () => new Set(backends.data.filter((b) => b.attended).map((b) => b.kind)),
    [backends.data],
  );
  const developerKinds = useMemo(
    () => new Set(backends.data.filter((b) => b.developer).map((b) => b.kind)),
    [backends.data],
  );
  const actionable = useMemo(() => scan.data?.actionable ?? [], [scan.data]);
  // Only what unattended care may install: no drivers, no developer packages.
  const updates = useMemo(
    () =>
      actionable.filter((u) => !attendedKinds.has(u.backend) && !developerKinds.has(u.backend)),
    [actionable, attendedKinds, developerKinds],
  );
  const attendedUpdates = useMemo(
    () => actionable.filter((u) => attendedKinds.has(u.backend)),
    [actionable, attendedKinds],
  );
  const developerUpdates = useMemo(
    () => actionable.filter((u) => developerKinds.has(u.backend)),
    [actionable, developerKinds],
  );
  const safeCleanup = useMemo(() => safeCleanupItems(cleanup.data), [cleanup.data]);
  const findings = useMemo(
    () => (security.data?.findings ?? []).filter((f) => f.severity !== "info"),
    [security.data],
  );

  const assessment = useMemo(
    () =>
      assess({
        updates,
        attendedUpdates,
        developerUpdates,
        needsAdmin: (scan.data?.skipped ?? []).filter((s) =>
          s.reason.toLowerCase().includes("administrator"),
        ).length,
        devices: devices.data.length,
        outdatedRuntimes: components.data.filter((c) => c.state === "outdated").length,
        seriousFindings: findings.filter((f) => f.severity === "critical" || f.severity === "high")
          .length,
        minorFindings: findings.filter((f) => f.severity === "medium" || f.severity === "low")
          .length,
        defender: defender.data,
        safeCleanup,
        health: health.data,
      }),
    [
      updates,
      attendedUpdates,
      developerUpdates,
      scan.data,
      devices.data,
      components.data,
      findings,
      defender.data,
      safeCleanup,
      health.data,
    ],
  );

  // ── Checking ─────────────────────────────────────────────────────────────
  const steps: { id: string; label: string; icon: ReactNode; resources: Resource<unknown>[] }[] = [
    { id: "updates", label: "Updates", icon: <Package />, resources: [scan as Resource<unknown>] },
    {
      id: "drivers",
      label: "Drivers & devices",
      icon: <MemoryStick />,
      resources: [devices as Resource<unknown>, components as Resource<unknown>],
    },
    { id: "cleanup", label: "Junk files", icon: <Brush />, resources: [cleanup as Resource<unknown>] },
    {
      id: "security",
      label: "Security",
      icon: <ShieldCheck />,
      resources: [defender as Resource<unknown>, security as Resource<unknown>],
    },
    { id: "health", label: "Disk & battery", icon: <HardDrive />, resources: [health as Resource<unknown>] },
  ];
  const states = steps.map((s) => stepState(s.resources));
  const checking = states.some((s) => s === "running");
  const checkedFraction = states.filter((s) => s === "done").length / steps.length;
  const runningStep = steps.find((_, i) => states[i] === "running");

  const scanEverything = useCallback(() => {
    void scan.refresh();
    void devices.refresh();
    void components.refresh();
    void cleanup.refresh();
    void defender.refresh();
    void security.refresh();
    void health.refresh();
  }, [scan, devices, components, cleanup, defender, security, health]);

  // ── Fixing ───────────────────────────────────────────────────────────────
  const [fixing, setFixing] = useState<{ step: string; progress: number } | null>(null);
  const [outcome, setOutcome] = useState<FixOutcome | null>(null);
  // The tick in the ring after a fix, before it goes back to the score.
  const [celebrating, setCelebrating] = useState(false);
  useEffect(() => {
    if (!celebrating) return;
    const t = setTimeout(() => setCelebrating(false), 2600);
    return () => clearTimeout(t);
  }, [celebrating]);

  useEffect(
    () =>
      safeListen<Progress>("apply-progress", (e) => {
        const p = e.payload;
        if (p.total > 0) {
          setFixing((f) =>
            f && f.step.startsWith("Installing")
              ? {
                  step: p.package ? `Installing ${p.package}` : "Installing updates",
                  progress: 5 + Math.round((p.current / p.total) * 60),
                }
              : f,
          );
        }
      }),
    [],
  );

  const fixEverything = useCallback(async () => {
    const toInstall = updates;
    const toClean = safeCleanup.map((c) => c.id);
    setOutcome(null);
    let updated = 0;
    let failed = 0;
    let reboot = false;
    let cleared = 0;
    let error: string | null = null;
    try {
      if (toInstall.length > 0) {
        setFixing({ step: "Installing updates", progress: 5 });
        const result = await api.apply({
          updates: toInstall,
          dry_run: false,
          restore_point: config.data?.restore_point ?? true,
        });
        updated = result.updated;
        failed = result.failed;
        reboot = result.reboot_required;
      }
      if (toClean.length > 0) {
        setFixing({ step: "Clearing junk files", progress: 70 });
        const results = await api.cleanupRun(toClean);
        cleared = results.reduce((sum, r) => sum + r.removed_bytes, 0);
      }
      setFixing({ step: "Checking again", progress: 90 });
      await Promise.allSettled([
        scan.refresh(),
        cleanup.refresh(),
        history.refresh(),
        packages.refresh(),
      ]);
    } catch (e) {
      error = errorText(e);
    } finally {
      setFixing(null);
      setOutcome({ updated, failed, cleared, reboot, error });
      setCelebrating(error === null);
    }
  }, [updates, safeCleanup, config.data, scan, cleanup, history, packages]);

  // ── The ring ─────────────────────────────────────────────────────────────
  const booting = store.booting && scan.loadedAt === null;
  let ringMode: RingMode = "idle";
  let ringValue = assessment.score;
  let ringTone: RingTone = assessment.tone;
  let ringLabel = assessment.grade;
  let ringCaption: string | undefined = "Care score";
  if (fixing) {
    ringMode = "busy";
    ringValue = fixing.progress;
    ringTone = "accent";
    ringLabel = "Fixing";
    ringCaption = fixing.step;
  } else if (booting || checking) {
    ringMode = "busy";
    ringValue = Math.round((booting ? store.bootProgress : checkedFraction) * 100);
    ringTone = "accent";
    ringLabel = "Checking";
    ringCaption = runningStep?.label;
  } else if (celebrating) {
    ringMode = "done";
  }

  const busy = !!fixing || booting || checking;
  const canFix = !busy && assessment.fixable > 0 && backends.loadedAt !== null;

  const headline = fixing
    ? "Taking care of it…"
    : busy
      ? "Checking your PC…"
      : assessment.issues.length === 0
        ? "Everything is taken care of"
        : assessment.fixable > 0
          ? `${assessment.fixable === 1 ? "One thing" : `${assessment.fixable} things`} can be fixed in one click`
          : `${assessment.issues.length === 1 ? "One thing needs" : `${assessment.issues.length} things need`} a look`;

  const info = sysInfo.data;
  const lastChecked = scan.loadedAt ? new Date(scan.loadedAt).toLocaleTimeString() : null;
  const platformName =
    info?.platform === "macos" ? "Mac" : info?.platform === "linux" ? "computer" : "PC";

  return (
    <div className="space-y-6">
      {/* Hero: the ring and the one button. */}
      <section className="relative overflow-hidden rounded-3xl border border-line glass px-8 py-8">
        <div className="flex flex-wrap items-center gap-x-12 gap-y-8">
          <CareRing
            mode={ringMode}
            value={ringValue}
            tone={ringTone}
            label={ringLabel}
            caption={ringCaption}
            center={
              ringMode === "done" ? (
                <CheckCircle2 className="w-16 h-16 text-success animate-pop" strokeWidth={1.5} />
              ) : undefined
            }
          />

          <div className="flex-1 min-w-[300px]">
            <div className="text-xs font-semibold uppercase tracking-[0.14em] text-accent">
              Smart care
            </div>
            <h1 className="mt-2 text-[30px] leading-tight font-bold text-fg">
              {headline.replace("PC", platformName)}
            </h1>
            <p className="mt-2 text-[15px] text-muted max-w-[52ch]">
              {fixing
                ? "Installing safe updates, then clearing junk files. Drivers and firmware are left alone."
                : busy
                  ? "Updates, drivers, junk files, security and hardware health — all at once."
                  : assessment.fixable > 0
                    ? "Safe updates and junk files, in one go. Drivers and anything risky always wait for you."
                    : assessment.issues.length > 0
                      ? "What is left needs a decision from you — each item below says where to make it."
                      : lastChecked
                        ? `Last checked at ${lastChecked}.`
                        : "Odysync checks updates, drivers, junk files, security and hardware health."}
            </p>

            <div className="mt-6 flex flex-wrap items-center gap-3">
              {canFix ? (
                <>
                  <Button
                    size="lg"
                    variant="primary"
                    icon={<Wand2 className="w-5 h-5" />}
                    onClick={() => void fixEverything()}
                  >
                    Fix everything
                  </Button>
                  <Button size="lg" variant="secondary" onClick={scanEverything} icon={<RefreshCw className="w-4 h-4" />}>
                    Scan again
                  </Button>
                </>
              ) : (
                <Button
                  size="lg"
                  variant={busy ? "secondary" : "primary"}
                  busy={busy}
                  icon={<Sparkles className="w-5 h-5" />}
                  onClick={scanEverything}
                >
                  {fixing ? "Working…" : busy ? "Checking…" : "Scan now"}
                </Button>
              )}
            </div>

            <ul className="mt-7 grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-x-6 gap-y-2.5">
              {steps.map((s, i) => (
                <StepRow key={s.id} label={s.label} icon={s.icon} state={booting ? (states[i] === "done" ? "done" : "running") : states[i]} />
              ))}
            </ul>
          </div>
        </div>
      </section>

      {outcome && <OutcomeBanner outcome={outcome} onHistory={() => onNavigate("history")} />}

      <div className="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_320px] gap-6 items-start">
        <Card
          title="What Odysync found"
          description={
            assessment.issues.length === 0
              ? undefined
              : "Every point off the score is listed here, with the reason."
          }
        >
          {busy && assessment.issues.length === 0 ? (
            <div className="space-y-3">
              {[0, 1, 2].map((i) => (
                <div key={i} className="flex items-center gap-3">
                  <span className="skeleton w-9 h-9 rounded-xl" />
                  <span className="skeleton h-4 flex-1" />
                </div>
              ))}
            </div>
          ) : assessment.issues.length === 0 ? (
            <div className="flex items-center gap-4 py-2">
              <div className="w-11 h-11 rounded-2xl bg-success/12 text-success flex items-center justify-center animate-pop">
                <PartyPopper className="w-5 h-5" />
              </div>
              <div>
                <div className="text-[15px] font-semibold">Nothing to do</div>
                <div className="text-sm text-muted">
                  Up to date, tidy, protected and healthy. Turn on automatic care to keep it that way.
                </div>
              </div>
            </div>
          ) : (
            <ul className="divide-y divide-line -my-2 stagger">
              {assessment.issues.map((issue) => (
                <IssueRow key={issue.id} issue={issue} onOpen={() => onNavigate(issue.tab)} />
              ))}
            </ul>
          )}
        </Card>

        <div className="space-y-6">
          <AutomaticCare />
          <Glance
            onNavigate={onNavigate}
            reclaimable={safeCleanupBytes(safeCleanup)}
            updates={actionable.length}
            protectedNow={defender.data ? defender.data.real_time_protection : null}
            disksOk={health.data ? health.data.disks.every((d) => d.state === "good" || d.state === "unknown") : null}
          />
        </div>
      </div>
    </div>
  );
}

function StepRow({ label, icon, state }: { label: string; icon: ReactNode; state: StepState }) {
  return (
    <li className="flex items-center gap-2.5 text-sm">
      <span
        className={`w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 transition-colors [&>svg]:w-3.5 [&>svg]:h-3.5 ${
          state === "done" ? "bg-success/12 text-success" : state === "running" ? "bg-accent/12 text-accent" : "bg-raised text-faint"
        }`}
      >
        {state === "running" ? <Loader2 className="animate-spin" /> : state === "done" ? <CheckCircle2 key="done" className="animate-pop" /> : icon}
      </span>
      <span className={state === "waiting" ? "text-faint" : "text-fg"}>{label}</span>
    </li>
  );
}

function IssueRow({ issue, onOpen }: { issue: CareIssue; onOpen: () => void }) {
  const tone = {
    danger: { box: "bg-danger/12 text-danger", icon: <ShieldAlert className="w-4.5 h-4.5" /> },
    warning: { box: "bg-warning/12 text-warning", icon: <TriangleAlert className="w-4.5 h-4.5" /> },
    info: { box: "bg-info/12 text-info", icon: <CircleAlert className="w-4.5 h-4.5" /> },
  }[issue.tone];
  return (
    <li className="flex items-center gap-4 py-3">
      <span className={`w-9 h-9 rounded-xl flex items-center justify-center flex-shrink-0 ${tone.box}`}>{tone.icon}</span>
      <div className="min-w-0 flex-1">
        <div className="text-[14.5px] font-medium text-fg truncate">{issue.title}</div>
        <div className="text-sm text-muted truncate" title={issue.detail}>
          {issue.detail}
        </div>
      </div>
      <span className="text-xs font-semibold tabular-nums text-faint w-9 text-right" title="Points off the care score">
        −{issue.points}
      </span>
      {issue.fix ? (
        <Badge tone="accent">
          <Wand2 className="w-3 h-3" /> In the fix
        </Badge>
      ) : (
        <Button size="sm" variant="ghost" onClick={onOpen} icon={<ArrowRight className="w-3.5 h-3.5" />}>
          Review
        </Button>
      )}
    </li>
  );
}

function OutcomeBanner({ outcome, onHistory }: { outcome: FixOutcome; onHistory: () => void }) {
  if (outcome.error) {
    return (
      <div role="alert" className="rounded-2xl border border-danger/30 bg-danger/10 px-5 py-4 text-sm text-fg animate-rise">
        The fix stopped: {outcome.error}
      </div>
    );
  }
  const parts: string[] = [];
  if (outcome.updated > 0) parts.push(`${outcome.updated} update${outcome.updated === 1 ? "" : "s"} installed`);
  if (outcome.cleared > 0) parts.push(`${formatSize(outcome.cleared)} cleared`);
  return (
    <div role="status" className="rounded-2xl border border-success/30 bg-success/10 px-5 py-4 flex flex-wrap items-center gap-x-4 gap-y-2 animate-rise">
      <CheckCircle2 className="w-5 h-5 text-success flex-shrink-0" />
      <div className="flex-1 min-w-[240px] text-sm">
        <span className="font-semibold text-fg">Done{parts.length ? ` — ${parts.join(", ")}` : ""}.</span>
        {outcome.failed > 0 && <span className="text-muted"> {outcome.failed} could not be installed; History says why.</span>}
        {outcome.reboot && <span className="text-muted"> Restart when convenient to finish.</span>}
      </div>
      {outcome.failed > 0 && (
        <Button size="sm" variant="secondary" onClick={onHistory}>
          Open History
        </Button>
      )}
    </div>
  );
}

/**
 * The switch that makes Odysync look after the machine on its own: a
 * scheduled `odysync care` run.
 */
function AutomaticCare() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [frequency, setFrequency] = useState<"daily" | "weekly">(() => {
    try {
      return localStorage.getItem(CARE_FREQUENCY_KEY) === "weekly" ? "weekly" : "daily";
    } catch {
      return "daily";
    }
  });
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .checkSchedule(CARE_TASK)
      .then((v) => live && setEnabled(v))
      .catch(() => live && setEnabled(false));
    return () => {
      live = false;
    };
  }, []);

  const apply = async (on: boolean, freq: "daily" | "weekly") => {
    setWorking(true);
    setError(null);
    try {
      if (on) {
        await api.createSchedule({ frequency: freq, time: "12:00", task_name: CARE_TASK, care: true });
      } else {
        await api.removeSchedule(CARE_TASK);
      }
      setEnabled(on);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setWorking(false);
    }
  };

  return (
    <Card>
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-[16px] font-semibold flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-accent" /> Automatic care
          </h2>
          <p className="text-sm text-muted mt-1">
            {enabled
              ? `${frequency === "daily" ? "Every day" : "Every week"} at noon, Odysync installs safe updates and clears junk files on its own.`
              : "Let Odysync install safe updates and clear junk files on its own. Drivers always wait for you."}
          </p>
        </div>
        <Switch
          label="Automatic care"
          checked={!!enabled}
          disabled={enabled === null || working}
          onChange={(v) => void apply(v, frequency)}
        />
      </div>
      {enabled && (
        <div className="mt-4">
          <Segmented
            label="How often"
            value={frequency}
            options={[
              { value: "daily", label: "Daily" },
              { value: "weekly", label: "Weekly" },
            ]}
            onChange={(v) => {
              setFrequency(v);
              try {
                localStorage.setItem(CARE_FREQUENCY_KEY, v);
              } catch {
                /* a remembered choice is a convenience */
              }
              void apply(true, v);
            }}
          />
        </div>
      )}
      {error && <p className="mt-3 text-sm text-danger break-words">{error}</p>}
    </Card>
  );
}

function Glance({
  onNavigate,
  reclaimable,
  updates,
  protectedNow,
  disksOk,
}: {
  onNavigate: (tab: Tab) => void;
  reclaimable: number;
  updates: number;
  protectedNow: boolean | null;
  disksOk: boolean | null;
}) {
  const rows: { tab: Tab; icon: ReactNode; label: string; value: string; ok: boolean | null }[] = [
    { tab: "updates", icon: <Package />, label: "Updates", value: updates === 0 ? "Up to date" : `${updates} ready`, ok: updates === 0 },
    { tab: "maintenance", icon: <Brush />, label: "Junk files", value: reclaimable > 0 ? formatSize(reclaimable) : "Tidy", ok: reclaimable < 200 * 1024 * 1024 },
    {
      tab: "security",
      icon: <ShieldCheck />,
      label: "Protection",
      value: protectedNow === null ? "—" : protectedNow ? "On" : "Off",
      ok: protectedNow,
    },
    { tab: "hardware", icon: <HardDrive />, label: "Disks", value: disksOk === null ? "—" : disksOk ? "Healthy" : "Check", ok: disksOk },
  ];
  return (
    <Card title="At a glance" padded>
      <ul className="-my-1.5">
        {rows.map((r) => (
          <li key={r.tab}>
            <button
              type="button"
              onClick={() => onNavigate(r.tab)}
              className="group w-full flex items-center gap-3 py-2 text-sm rounded-lg hover:bg-raised/50 -mx-2 px-2 transition-colors"
            >
              <span className="text-faint [&>svg]:w-4 [&>svg]:h-4">{r.icon}</span>
              <span className="text-muted">{r.label}</span>
              <span className="ml-auto flex items-center gap-2 font-medium text-fg">
                {r.ok === null ? (
                  <Circle className="w-2 h-2 text-faint" />
                ) : (
                  <span className={`w-2 h-2 rounded-full ${r.ok ? "bg-success" : "bg-warning"}`} />
                )}
                {r.value}
              </span>
              <ArrowRight className="w-3.5 h-3.5 text-faint opacity-0 group-hover:opacity-100 transition-opacity" />
            </button>
          </li>
        ))}
      </ul>
    </Card>
  );
}
