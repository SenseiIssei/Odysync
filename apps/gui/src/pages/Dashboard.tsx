import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  ArrowRight,
  Brush,
  CheckCircle2,
  Circle,
  CircleAlert,
  CircleHelp,
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
import { useStore, type Resource } from "../store-context";
import { Badge, Button, Card, Segmented, Switch } from "../components/ui";
import { CareRing, type RingMode, type RingTone } from "../components/CareRing";
import { ScanProgressView } from "../components/Progress";
import {
  assess,
  safeCleanupBytes,
  safeCleanupItems,
  type CareIssue,
  type FailedCheck,
} from "../care";
import { errorText, formatSize } from "../format";
import type { Tab } from "../nav";
import type { ApplyEntryDto } from "../types";

/** The schedule automatic care runs under, separate from a plain update schedule. */
const CARE_TASK = "OdysyncCare";
const CARE_FREQUENCY_KEY = "odysync.careFrequency";

interface FixOutcome {
  updated: number;
  /** Updates that were tried and did not work, with the reason. */
  failures: ApplyEntryDto[];
  /** Updates that were not started, because the run was cancelled. */
  skipped: number;
  cancelled: boolean;
  cleared: number;
  reboot: boolean;
  error: string | null;
}

type StepState = "unchecked" | "running" | "partial" | "failed" | "done";

/** Where a set of background checks stands. A failed one is never "done". */
function stepState(resources: Resource<unknown>[]): StepState {
  if (resources.some((r) => r.loading)) return "running";
  if (resources.some((r) => r.error !== null)) return "failed";
  if (resources.some((r) => r.loadedAt === null)) return "unchecked";
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
    apply,
  } = store;
  const windows = sysInfo.data?.platform === "windows";

  // ── What the machine looks like ──────────────────────────────────────────
  const attendedKinds = useMemo(
    () => new Set(backends.data.filter((b) => b.attended).map((b) => b.kind)),
    [backends.data],
  );
  const developerKinds = useMemo(
    () => new Set(backends.data.filter((b) => b.developer).map((b) => b.kind)),
    [backends.data],
  );
  // After a failed scan the list in hand is an old one. It cannot be installed
  // from (the command layer has nothing to match it against) and says nothing
  // about today.
  const scanUsable = scan.health.state !== "failed";
  const actionable = useMemo(
    () => (scanUsable ? (scan.data?.actionable ?? []) : []),
    [scan.data, scanUsable],
  );
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

  const failedChecks = useMemo(() => {
    const checks: (FailedCheck | null)[] = [
      failedCheck("devices", "Devices", devices, "drivers"),
      failedCheck("runtimes", "Runtimes", components, "drivers"),
      failedCheck("cleanup", "Junk files", cleanup, "maintenance"),
      failedCheck("health", "Disk and battery health", health, "hardware"),
      // Elsewhere these two have nothing to look at.
      windows ? failedCheck("defender", "Microsoft Defender", defender, "security") : null,
      windows ? failedCheck("security", "Security", security, "security") : null,
    ];
    return checks.filter((c) => c !== null);
  }, [devices, components, cleanup, health, defender, security, windows]);

  // Outside Windows there is no audit that could be incomplete.
  const incompleteSecurity = windows
    ? (security.data?.sections ?? []).filter((s) => !s.ok).length
    : 0;
  const assessment = useMemo(
    () =>
      assess({
        scan: scan.health,
        sourceName: (kind) => backends.data.find((b) => b.kind === kind)?.name ?? kind,
        failedChecks,
        incompleteSecurity,
        updates,
        attendedUpdates,
        developerUpdates,
        needsAdmin: scanUsable
          ? (scan.data?.skipped ?? []).filter((s) =>
              s.reason.toLowerCase().includes("administrator"),
            ).length
          : 0,
        devices: devices.data.length,
        outdatedRuntimes: components.data.filter((c) => c.state === "outdated").length,
        seriousFindings: findings.filter((f) => f.severity === "critical" || f.severity === "high")
          .length,
        minorFindings: findings.filter((f) => f.severity === "medium" || f.severity === "low")
          .length,
        defender: windows ? defender.data : null,
        safeCleanup,
        health: health.data,
      }),
    [
      scan.health,
      scanUsable,
      backends.data,
      failedChecks,
      incompleteSecurity,
      windows,
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
  const states = steps.map((s): StepState => {
    const state = stepState(s.resources);
    if (state !== "done") return state;
    if (s.id === "security") {
      const unread = windows && defender.data?.real_time_protection === null;
      return unread || incompleteSecurity > 0 ? "partial" : state;
    }
    if (s.id !== "updates") return state;
    return scan.health.state === "partial" || scan.health.state === "failed"
      ? scan.health.state
      : state;
  });
  const checking = states.some((s) => s === "running");
  const checkedFraction = states.filter((s) => s !== "running").length / steps.length;
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

  const fixEverything = useCallback(async () => {
    const toInstall = updates;
    const toClean = safeCleanup.map((c) => c.id);
    setOutcome(null);
    setCelebrating(false);
    const done: FixOutcome = {
      updated: 0,
      failures: [],
      skipped: 0,
      cancelled: false,
      cleared: 0,
      reboot: false,
      error: null,
    };
    try {
      if (toInstall.length > 0) {
        setFixing({ step: "Installing updates", progress: 5 });
        const { result, cancelled } = await apply.run({
          updates: toInstall,
          dry_run: false,
          restore_point: config.data?.restore_point ?? true,
        });
        done.updated = result.updated;
        done.failures = result.entries.filter(
          (e) => e.status !== "updated" && e.status !== "skipped",
        );
        done.skipped = result.skipped;
        done.cancelled = cancelled;
        done.reboot = result.reboot_required;
      }
      // Cancel means stop, not "skip to the next step".
      if (toClean.length > 0 && !done.cancelled) {
        setFixing({ step: "Clearing junk files", progress: 70 });
        const results = await api.cleanupRun(toClean);
        done.cleared = results.reduce((sum, r) => sum + r.removed_bytes, 0);
      }
      setFixing({ step: "Checking again", progress: 90 });
      await Promise.allSettled([
        // The install already started a fresh scan; this waits for it.
        toInstall.length > 0 ? scan.refresh() : null,
        cleanup.reload(),
      ]);
    } catch (e) {
      done.error = errorText(e);
    } finally {
      setFixing(null);
      setOutcome(done);
      setCelebrating(done.error === null && done.failures.length === 0 && !done.cancelled);
    }
  }, [updates, safeCleanup, config.data, apply, scan, cleanup]);

  // ── The ring ─────────────────────────────────────────────────────────────
  const booting = store.booting && scan.loadedAt === null;
  const { score, scoreKnown } = assessment;
  // An install started on another page shows here too.
  const installing = apply.running && !apply.dryRun;
  const installed = apply.progress && apply.progress.total > 0
    ? apply.progress.current / apply.progress.total
    : 0;
  let ringMode: RingMode = "idle";
  let ringValue: number | null = scoreKnown ? score : null;
  let ringTone: RingTone = scoreKnown ? assessment.tone : "warning";
  let ringLabel = scoreKnown ? assessment.grade : "Not checked";
  let ringCaption: string | undefined = scoreKnown ? "Care score" : "No score";
  if (fixing) {
    ringMode = "busy";
    ringValue = installing ? 5 + Math.round(installed * 60) : fixing.progress;
    ringTone = "accent";
    ringLabel = "Fixing";
    ringCaption =
      installing && apply.progress?.package
        ? `Installing ${apply.progress.package}`
        : fixing.step;
  } else if (installing) {
    ringMode = "busy";
    ringValue = Math.round(installed * 100);
    ringTone = "accent";
    ringLabel = "Installing";
    ringCaption = apply.progress?.package || undefined;
  } else if (booting || checking) {
    ringMode = "busy";
    ringValue = Math.round((booting ? store.bootProgress : checkedFraction) * 100);
    ringTone = "accent";
    ringLabel = "Checking";
    ringCaption = runningStep?.label;
  } else if (celebrating) {
    ringMode = "done";
  }

  const busy = !!fixing || installing || booting || checking;
  const canFix =
    !busy &&
    !apply.running &&
    assessment.fixable > 0 &&
    backends.loadedAt !== null &&
    !(updates.length > 0 && scan.fromCache);

  const headline = fixing
    ? "Taking care of it…"
    : installing
      ? "Installing updates…"
      : busy
        ? "Checking your PC…"
        : scan.health.state === "failed"
          ? "Updates could not be checked"
          : !scoreKnown
            ? "Nothing has been checked yet"
            : assessment.issues.length === 0
              ? "Everything is taken care of"
              : assessment.fixable > 0
                ? `${assessment.fixable === 1 ? "One thing" : `${assessment.fixable} things`} can be fixed in one click`
                : `${assessment.issues.length === 1 ? "One thing needs" : `${assessment.issues.length} things need`} a look`;

  const info = sysInfo.data;
  const lastChecked = scan.loadedAt
    ? new Date(scan.loadedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })
    : null;
  const showingLastTime = scan.fromCache && lastChecked !== null;
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
              ) : ringValue === null ? (
                <CircleHelp className="w-16 h-16 text-warning" strokeWidth={1.5} />
              ) : undefined
            }
          />

          <div className="flex-1 min-w-[300px]">
            <div className="text-xs font-semibold uppercase tracking-[0.14em] text-accent">
              Smart care
            </div>
            <h1 tabIndex={-1} className="mt-2 text-[30px] leading-tight font-bold text-fg">
              {headline.replace("PC", platformName)}
            </h1>
            <p className="mt-2 text-[15px] text-muted max-w-[52ch]">
              {fixing
                ? "Installing safe updates, then clearing junk files. Drivers and firmware are left alone."
                : installing
                  ? "Started from another page. A scan follows when it is done."
                  : busy && showingLastTime
                    ? `Showing what Odysync found on ${lastChecked} while it checks again.`
                    : busy
                      ? "Updates, drivers, junk files, security and hardware health, all at once."
                      : scan.health.state === "failed"
                        ? `${scan.health.error} Without it there is no score; what the other checks found is listed below.`
                        : assessment.fixable > 0
                          ? "Safe updates and junk files, in one go. Drivers and anything risky always wait for you."
                          : assessment.issues.length > 0
                            ? "What is left needs a decision from you. Each item below says where to make it."
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
                  {fixing || installing
                    ? "Working…"
                    : busy
                      ? "Checking…"
                      : scan.health.state === "failed"
                        ? "Try again"
                        : "Scan now"}
                </Button>
              )}
              {installing && <CancelInstall />}
            </div>

            {scan.loading && !apply.running && (
              <div className="mt-5 max-w-[560px]">
                <ScanProgressView />
              </div>
            )}

            <ul className="mt-7 grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-x-6 gap-y-2.5">
              {steps.map((s, i) => (
                <StepRow key={s.id} label={s.label} icon={s.icon} state={booting && states[i] === "unchecked" ? "running" : states[i]} />
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
                  {defender.data?.real_time_protection && security.data
                    ? "Up to date, tidy, protected and healthy."
                    : windows && !security.data
                      ? "Up to date, tidy and healthy. The security audit has not run."
                      : "Up to date, tidy and healthy."}{" "}
                  Turn on automatic care to keep it that way.
                </div>
              </div>
            </div>
          ) : (
            <ul className="divide-y divide-line -my-2 stagger">
              {assessment.issues.map((issue) => (
                <IssueRow
                  key={issue.id}
                  issue={issue}
                  busy={busy}
                  onOpen={() => onNavigate(issue.tab)}
                  onRetry={scanEverything}
                />
              ))}
            </ul>
          )}
        </Card>

        <div className="space-y-6">
          <AutomaticCare />
          <Glance
            onNavigate={onNavigate}
            reclaimable={safeCleanupBytes(safeCleanup)}
            windows={windows}
          />
        </div>
      </div>
    </div>
  );
}

/** The entry for a check whose last attempt failed, or null when it did not. */
function failedCheck<T>(
  id: string,
  label: string,
  resource: Resource<T>,
  tab: Tab,
): FailedCheck | null {
  return resource.error === null ? null : { id, label, error: resource.error, tab };
}

function CancelInstall() {
  const { apply } = useStore();
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <Button
        size="lg"
        busy={apply.cancelling}
        onClick={() => {
          setError(null);
          apply.cancel().catch((e: unknown) => setError(errorText(e)));
        }}
      >
        {apply.cancelling ? "Stopping after this package…" : "Cancel"}
      </Button>
      {error && <p className="text-sm text-danger break-words">Could not cancel: {error}</p>}
    </>
  );
}

const STEP: Record<StepState, { box: string; text: string }> = {
  done: { box: "bg-success/12 text-success", text: "checked" },
  running: { box: "bg-accent/12 text-accent", text: "being checked" },
  partial: { box: "bg-warning/12 text-warning", text: "only partly checked" },
  failed: { box: "bg-danger/12 text-danger", text: "could not be checked" },
  unchecked: { box: "bg-raised text-faint", text: "not checked" },
};

function StepRow({ label, icon, state }: { label: string; icon: ReactNode; state: StepState }) {
  return (
    <li className="flex items-center gap-2.5 text-sm">
      <span
        aria-hidden
        className={`w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 transition-colors [&>svg]:w-3.5 [&>svg]:h-3.5 ${STEP[state].box}`}
      >
        {state === "running" ? (
          <Loader2 className="animate-spin" />
        ) : state === "done" ? (
          <CheckCircle2 key="done" className="animate-pop" />
        ) : state === "partial" ? (
          <TriangleAlert />
        ) : state === "failed" ? (
          <CircleAlert />
        ) : (
          icon
        )}
      </span>
      <span className={state === "unchecked" ? "text-faint" : "text-fg"}>
        {label}
        {state === "done" || state === "running" ? (
          <span className="sr-only">: {STEP[state].text}</span>
        ) : (
          <span className="block text-xs text-muted">{STEP[state].text}</span>
        )}
      </span>
    </li>
  );
}

function IssueRow({
  issue,
  busy,
  onOpen,
  onRetry,
}: {
  issue: CareIssue;
  busy: boolean;
  onOpen: () => void;
  onRetry: () => void;
}) {
  const tone = {
    danger: { box: "bg-danger/12 text-danger", word: "Urgent", icon: <ShieldAlert className="w-4.5 h-4.5" /> },
    warning: { box: "bg-warning/12 text-warning", word: "Attention", icon: <TriangleAlert className="w-4.5 h-4.5" /> },
    info: { box: "bg-info/12 text-info", word: "Note", icon: <CircleAlert className="w-4.5 h-4.5" /> },
  }[issue.tone];
  return (
    <li className="flex items-center gap-4 py-3">
      <span aria-hidden className={`w-9 h-9 rounded-xl flex items-center justify-center flex-shrink-0 ${tone.box}`}>
        {tone.icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[14.5px] font-medium text-fg truncate">
          <span className="sr-only">{tone.word}: </span>
          {issue.title}
        </div>
        <div className="text-sm text-muted line-clamp-2 break-words" title={issue.detail}>
          {issue.detail}
        </div>
      </div>
      {issue.points > 0 && (
        <span className="text-xs font-semibold tabular-nums text-faint w-9 text-right" title="Points off the care score">
          <span aria-hidden>−{issue.points}</span>
          <span className="sr-only">{issue.points} points off the care score</span>
        </span>
      )}
      {issue.fix ? (
        <Badge tone="accent">
          <Wand2 className="w-3 h-3" /> In the fix
        </Badge>
      ) : issue.retry ? (
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={onRetry}
          aria-label={`Check again: ${issue.title}`}
          icon={<RefreshCw className="w-3.5 h-3.5" />}
        >
          Check again
        </Button>
      ) : (
        <Button
          size="sm"
          variant="ghost"
          onClick={onOpen}
          aria-label={`Review: ${issue.title}`}
          icon={<ArrowRight className="w-3.5 h-3.5" />}
        >
          Review
        </Button>
      )}
    </li>
  );
}

/** How many failures to spell out before pointing at History. */
const FAILURES_SHOWN = 3;

function OutcomeBanner({ outcome, onHistory }: { outcome: FixOutcome; onHistory: () => void }) {
  const { updated, failures, skipped, cancelled, cleared, reboot, error } = outcome;
  const parts: string[] = [];
  if (updated > 0) parts.push(`${updated} update${updated === 1 ? "" : "s"} installed`);
  if (cleared > 0) parts.push(`${formatSize(cleared)} cleared`);
  const did = parts.join(", ");

  const tone = error !== null ? "danger" : failures.length > 0 || cancelled ? "warning" : "success";
  const box = {
    danger: "border-danger/30 bg-danger/10",
    warning: "border-warning/30 bg-warning/10",
    success: "border-success/30 bg-success/10",
  }[tone];
  const title =
    error !== null
      ? "The fix stopped"
      : cancelled
        ? "Cancelled"
        : failures.length > 0
          ? "Finished, but not everything worked"
          : "Done";

  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={`rounded-2xl border px-5 py-4 flex flex-wrap items-start gap-x-4 gap-y-2 animate-rise ${box}`}
    >
      {tone === "success" ? (
        <CheckCircle2 aria-hidden className="w-5 h-5 text-success flex-shrink-0" />
      ) : (
        <TriangleAlert
          aria-hidden
          className={`w-5 h-5 flex-shrink-0 ${tone === "danger" ? "text-danger" : "text-warning"}`}
        />
      )}
      <div className="flex-1 min-w-[240px] text-sm text-fg">
        <p>
          <span className="font-semibold">
            {title}
            {did ? `: ${did}` : ""}.
          </span>
          {cancelled && skipped > 0 && ` ${skipped} update${skipped === 1 ? " was" : "s were"} left for later.`}
          {reboot && " Restart when convenient to finish."}
        </p>
        {error !== null && <p className="mt-1 break-words">{error}</p>}
        {failures.length > 0 && (
          <ul className="mt-1.5 space-y-1">
            {failures.slice(0, FAILURES_SHOWN).map((f, i) => (
              <li key={`${f.name}-${i}`} className="break-words">
                <span className="font-medium">{f.name}</span> was not installed
                {f.detail ? `: ${f.detail}` : "."}
              </li>
            ))}
            {failures.length > FAILURES_SHOWN && (
              <li>
                {failures.length - FAILURES_SHOWN} more could not be installed. History lists them
                with the reason.
              </li>
            )}
          </ul>
        )}
      </div>
      {failures.length > 0 && (
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
      .catch((e: unknown) => {
        if (!live) return;
        // The switch stays usable: turning it on writes the schedule anew.
        setEnabled(false);
        setError(`Whether automatic care is on could not be read: ${errorText(e)}`);
      });
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
      {error && <p role="alert" className="mt-3 text-sm text-danger break-words">{error}</p>}
    </Card>
  );
}

function Glance({
  onNavigate,
  reclaimable,
  windows,
}: {
  onNavigate: (tab: Tab) => void;
  reclaimable: number;
  windows: boolean;
}) {
  const { scan, cleanup, defender, health } = useStore();

  const ready = scan.data?.actionable.length ?? 0;
  const updates: { value: string; ok: boolean | null } =
    scan.health.state === "failed"
      ? { value: "Not checked", ok: null }
      : scan.health.state === "unknown"
        ? { value: "-", ok: null }
        : ready > 0
          ? { value: `${ready} ready`, ok: false }
          : scan.health.state === "partial"
            ? { value: "Not fully checked", ok: null }
            : { value: "Up to date", ok: true };

  const junk: { value: string; ok: boolean | null } = cleanup.error
    ? { value: "Not measured", ok: null }
    : cleanup.loadedAt === null
      ? { value: "-", ok: null }
      : {
          value: reclaimable > 0 ? formatSize(reclaimable) : "Tidy",
          ok: reclaimable < 200 * 1024 * 1024,
        };

  const realTime = defender.data?.real_time_protection ?? null;
  const protection: { value: string; ok: boolean | null } =
    defender.error || (defender.data && realTime === null)
      ? { value: "Could not be read", ok: null }
      : realTime === null
        ? { value: "-", ok: null }
        : { value: realTime ? "On" : "Off", ok: realTime };

  const disks = health.data?.disks ?? [];
  const reported = disks.filter((d) => d.state !== "unknown");
  const disk: { value: string; ok: boolean | null } = health.error
    ? { value: "Could not be read", ok: null }
    : health.data === null
      ? { value: "-", ok: null }
      : reported.length === 0
        ? { value: "Not reported", ok: null }
        : reported.every((d) => d.state === "good")
          ? { value: "Healthy", ok: true }
          : { value: "Check", ok: false };

  const rows: { tab: Tab; icon: ReactNode; label: string; value: string; ok: boolean | null }[] = [
    { tab: "updates", icon: <Package />, label: "Updates", ...updates },
    { tab: "maintenance", icon: <Brush />, label: "Junk files", ...junk },
    ...(windows
      ? [{ tab: "security" as const, icon: <ShieldCheck />, label: "Protection", ...protection }]
      : []),
    { tab: "hardware", icon: <HardDrive />, label: "Disks", ...disk },
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
                  <Circle aria-hidden className="w-2 h-2 text-faint" />
                ) : (
                  <span aria-hidden className={`w-2 h-2 rounded-full ${r.ok ? "bg-success" : "bg-warning"}`} />
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
