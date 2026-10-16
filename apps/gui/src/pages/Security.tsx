import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Bug,
  CheckCircle2,
  Circle,
  Eye,
  Loader2,
  Network,
  RefreshCw,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Siren,
  Wrench,
} from "lucide-react";
import * as api from "../api";
import { safeListen } from "../events";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorBar,
  InfoBar,
  PageHeader,
  SuccessBar,
  WarningBar,
} from "../components/ui";
import { errorText, formatDate } from "../format";
import type { Remediation, SectionProgress, SecurityFinding, Severity } from "../types";

const SECTION_LABELS: Record<string, string> = {
  defender: "Microsoft Defender",
  persistence: "Persistence and autostart",
  integrity: "File and client integrity",
  network: "Network connections",
  posture: "System hardening",
};

const ALL_SECTIONS = ["defender", "persistence", "integrity", "network", "posture"];

/** Severities that describe something wrong, as opposed to something noted. */
const ACTIONABLE: Severity[] = ["critical", "high", "medium", "low"];
const SEVERITY_ORDER: Severity[] = [...ACTIONABLE, "info"];

const SEVERITY: Record<Severity, { tone: "danger" | "warning" | "info" | "neutral"; label: string }> = {
  critical: { tone: "danger", label: "Critical" },
  high: { tone: "danger", label: "High" },
  medium: { tone: "warning", label: "Medium" },
  low: { tone: "info", label: "Low" },
  // Deliberately not alarming: these are normal things worth being able to
  // see — your own apps, games, Spotify — not suspected malware.
  info: { tone: "neutral", label: "Watching" },
};

const CATEGORY_ICON: Record<string, React.ReactNode> = {
  malware: <Bug className="w-4 h-4" />,
  persistence: <Siren className="w-4 h-4" />,
  network: <Network className="w-4 h-4" />,
  posture: <ShieldCheck className="w-4 h-4" />,
  integrity: <Eye className="w-4 h-4" />,
  account: <Shield className="w-4 h-4" />,
};

export default function Security() {
  const { security, defender, sysInfo, logs } = useStore();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [severityFilter, setSeverityFilter] = useState<Severity | "all">("all");

  const elevated = sysInfo.data?.elevated ?? false;
  const platform = sysInfo.data?.platform ?? "windows";
  const report = security.data;

  // Live per-section progress, so a running audit shows which checks are in
  // flight and how long each took rather than one opaque spinner.
  const [sectionState, setSectionState] = useState<Record<string, SectionProgress>>({});
  useEffect(
    () =>
      safeListen<SectionProgress>("security-progress", (e) => {
        setSectionState((prev) => ({ ...prev, [e.payload.name]: e.payload }));
      }),
    [],
  );
  // Clear the per-section board when a fresh audit starts.
  useEffect(() => {
    if (security.loading) setSectionState({});
  }, [security.loading]);

  const sorted = useMemo(
    () =>
      [...(report?.findings ?? [])].sort(
        (a, b) => SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity),
      ),
    [report],
  );

  // Split rather than mix. Everything at Info severity is an inventory entry —
  // an autostart, a listener, an installed extension — and burying two real
  // findings among 120 of those is how you get someone to ignore all of them.
  const threats = useMemo(() => sorted.filter((f) => f.severity !== "info"), [sorted]);
  const watching = useMemo(() => sorted.filter((f) => f.severity === "info"), [sorted]);
  const findings = useMemo(
    () => (severityFilter === "all" ? threats : sorted.filter((f) => f.severity === severityFilter)),
    [sorted, threats, severityFilter],
  );

  const counts = useMemo(() => {
    const c: Record<Severity, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
    for (const f of report?.findings ?? []) c[f.severity]++;
    return c;
  }, [report]);

  const failedSections = (report?.sections ?? []).filter((s) => !s.ok);

  if (platform !== "windows") {
    return (
      <>
        <PageHeader title="Security" />
        <Card>
          <EmptyState
            icon={<ShieldCheck />}
            title="The security audit is built on Windows' own security features"
            hint={
              platform === "macos"
                ? "On macOS, Gatekeeper, XProtect and System Integrity Protection cover this ground; check them under System Settings → Privacy & Security."
                : "On Linux, keep the distribution's security updates current — the Updates page does that — and use your distribution's firewall and audit tools."
            }
          />
        </Card>
      </>
    );
  }

  const run = async (label: string, fn: () => Promise<unknown>, after?: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      const result = await fn();
      if (typeof result === "string" && result) setNotice(result);
      if (after) await after();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
      void logs.refresh();
    }
  };

  const applyFix = async (finding: SecurityFinding) => {
    if (!finding.remediation) return;
    if (confirming !== finding.id) {
      setConfirming(finding.id);
      return;
    }
    setConfirming(null);
    await run(
      `fix:${finding.id}`,
      () => api.applyRemediation(finding.remediation as Remediation),
      async () => {
        await security.refresh();
      },
    );
  };

  const d = defender.data;

  return (
    <>
      <PageHeader
        title="Security"
        subtitle="Persistence, tampering, network exposure and weak settings — plus Microsoft Defender, which does the actual malware scanning."
        actions={
          <Button
            onClick={() => void security.refresh()}
            busy={security.loading}
            disabled={busy !== null}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            {security.loading ? "Auditing…" : "Audit again"}
          </Button>
        }
      />

      {/* Honesty about what this is. Overstating it would be worse than useless
          for someone who has just been compromised. */}
      <InfoBar
        message={
          <>
            This is an audit, <strong className="font-medium">not an antivirus engine</strong>: a
            clean result does not prove a clean machine. For a suspected infection, also run a
            Microsoft Defender Offline scan.
          </>
        }
      />

      {!elevated && (
        <WarningBar message="Some checks — machine-wide persistence, service configuration, Defender control — need administrator rights and are skipped without them." />
      )}
      {error && <ErrorBar message={error} />}
      {notice && <SuccessBar message={notice} />}

      <Card
        title="Microsoft Defender"
        description={
          d
            ? `Definitions ${d.signature_version || "unknown"}${d.last_quick_scan ? ` · last quick scan ${formatDate(d.last_quick_scan)}` : ""}`
            : defender.loading
              ? "Asking Defender…"
              : "Defender's status is not available."
        }
        actions={
          <>
            <Button
              size="sm"
              busy={busy === "sigs"}
              disabled={busy !== null}
              onClick={() => run("sigs", api.updateDefenderSignatures, async () => void (await defender.refresh()))}
            >
              Update definitions
            </Button>
            <Button
              size="sm"
              busy={busy === "quick"}
              disabled={busy !== null}
              onClick={() =>
                run("quick", api.defenderQuickScan, async () => {
                  await Promise.allSettled([security.refresh(), defender.refresh()]);
                })
              }
            >
              Quick scan
            </Button>
            <Button
              size="sm"
              variant="ghost"
              busy={busy === "full"}
              disabled={busy !== null}
              title="Reads every file on every drive; can take hours and keeps running in the tray"
              onClick={() =>
                run("full", api.defenderFullScan, async () => {
                  await Promise.allSettled([security.refresh(), defender.refresh()]);
                })
              }
            >
              Full scan
            </Button>
          </>
        }
      >
        {d && (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
            <Status label="Real-time protection" ok={d.real_time_protection} />
            <Status label="Tamper protection" ok={d.tamper_protection} />
            <Status label="Antivirus" ok={d.antivirus_enabled} />
            <Status
              label={d.signature_age_days === 0 ? "Definitions current" : `Definitions ${d.signature_age_days} days old`}
              ok={d.signature_age_days <= 7}
            />
          </div>
        )}
      </Card>

      {security.loading && (
        <Card title="Audit in progress" padded={false}>
          <ul className="divide-y divide-line border-t border-line">
            {ALL_SECTIONS.map((name) => {
              const s = sectionState[name];
              const state = s?.state ?? "pending";
              return (
                <li key={name} className="flex items-center gap-3 px-5 py-2.5 text-sm">
                  {state === "done" ? (
                    <CheckCircle2 className="w-4 h-4 text-success" />
                  ) : state === "failed" ? (
                    <AlertTriangle className="w-4 h-4 text-danger" />
                  ) : state === "started" ? (
                    <Loader2 className="w-4 h-4 text-accent animate-spin" />
                  ) : (
                    <Circle className="w-4 h-4 text-line-strong" />
                  )}
                  <span className={`flex-1 ${state === "pending" ? "text-faint" : "text-fg"}`}>
                    {SECTION_LABELS[name] ?? name}
                  </span>
                  {s?.durationMs != null && (
                    <span className="text-faint tabular-nums">
                      {(s.durationMs / 1000).toFixed(1)} s
                      {s.findings != null && s.findings > 0 && ` · ${s.findings} found`}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        </Card>
      )}

      {failedSections.length > 0 && (
        <WarningBar
          message={
            <>
              {failedSections.length} check{failedSections.length !== 1 ? "s" : ""} could not run,
              so this audit is incomplete:{" "}
              <span className="font-mono text-xs">
                {failedSections.map((s) => `${s.name}: ${s.error ?? "unknown error"}`).join("; ")}
              </span>
            </>
          }
        />
      )}

      {report && (
        <Card
          padded={false}
          title="Findings"
          description={`Audited ${formatDate(report.scannedAt)}. Click a severity to filter.`}
        >
          <div className="flex flex-wrap gap-2 px-5 pb-4">
            {SEVERITY_ORDER.map((sev) => {
              const active = severityFilter === sev;
              return (
                <button
                  type="button"
                  key={sev}
                  onClick={() => setSeverityFilter(active ? "all" : sev)}
                  aria-pressed={active}
                  className={`inline-flex items-center gap-2 h-8 px-3 rounded-lg border text-sm transition-colors ${
                    active ? "border-accent bg-accent/10 text-fg" : "border-line text-muted hover:text-fg hover:bg-raised"
                  }`}
                >
                  <span className={`tabular-nums font-semibold ${counts[sev] > 0 && sev !== "info" ? "text-fg" : "text-faint"}`}>
                    {counts[sev]}
                  </span>
                  {SEVERITY[sev].label}
                </button>
              );
            })}
          </div>
          {findings.length === 0 ? (
            <div className="border-t border-line">
              <EmptyState
                icon={<CheckCircle2 className="text-success" />}
                title={
                  severityFilter === "all"
                    ? "Nothing needs your attention"
                    : `No ${SEVERITY[severityFilter as Severity].label.toLowerCase()} findings`
                }
                hint={
                  severityFilter === "all"
                    ? `${watching.length} item${watching.length !== 1 ? "s are" : " is"} listed under Watching below. No indicators were found — which is not the same as proof of a clean machine.`
                    : undefined
                }
              />
            </div>
          ) : (
            <ul className="divide-y divide-line border-t border-line">
              {findings.map((f) => (
                <Finding
                  key={f.id}
                  finding={f}
                  busy={busy === `fix:${f.id}`}
                  disabled={busy !== null}
                  confirming={confirming === f.id}
                  onFix={() => void applyFix(f)}
                />
              ))}
            </ul>
          )}
        </Card>
      )}

      {/* Inventory, collapsed. Your own apps, games and tools live here — this
          is "what is running on this machine", not "what is wrong with it". */}
      {severityFilter === "all" && watching.length > 0 && (
        <Card padded={false}>
          <details>
            <summary className="cursor-pointer px-5 py-4 text-sm text-muted hover:text-fg list-none flex items-center gap-2">
              <Eye className="w-4 h-4" />
              Watching — {watching.length} item{watching.length !== 1 ? "s" : ""} noted, none flagged
              as a problem
            </summary>
            <p className="px-5 pb-3 text-sm text-muted">
              Autostart entries, network listeners and browser extensions on this machine. Your own
              software appears here too; nothing in this list is being called malware.
            </p>
            <ul className="divide-y divide-line border-t border-line">
              {watching.map((f) => (
                <Finding
                  key={f.id}
                  finding={f}
                  busy={busy === `fix:${f.id}`}
                  disabled={busy !== null}
                  confirming={confirming === f.id}
                  onFix={() => void applyFix(f)}
                />
              ))}
            </ul>
          </details>
        </Card>
      )}
    </>
  );
}

function Status({ label, ok }: { label: string; ok: boolean }) {
  return (
    <div className="flex items-center gap-2 rounded-lg border border-line bg-canvas px-3 py-2.5 text-sm">
      {ok ? (
        <CheckCircle2 className="w-4 h-4 text-success flex-shrink-0" />
      ) : (
        <AlertTriangle className="w-4 h-4 text-danger flex-shrink-0" />
      )}
      <span className={`truncate ${ok ? "text-fg" : "text-danger"}`}>{label}</span>
    </div>
  );
}

function Finding({
  finding,
  busy,
  disabled,
  confirming,
  onFix,
}: {
  finding: SecurityFinding;
  busy: boolean;
  disabled: boolean;
  confirming: boolean;
  onFix: () => void;
}) {
  const sev = SEVERITY[finding.severity];
  const [open, setOpen] = useState(finding.severity === "critical");

  return (
    <li className="px-5 py-4">
      <div className="flex items-start gap-3">
        <span className="text-faint mt-0.5 flex-shrink-0">
          {CATEGORY_ICON[finding.category] ?? <ShieldAlert className="w-4 h-4" />}
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-fg">{finding.title}</span>
            <Badge tone={sev.tone}>{sev.label}</Badge>
            <Badge>{finding.category}</Badge>
          </div>
          <p className="text-sm text-muted mt-1">{finding.detail}</p>
          {finding.evidence.length > 0 && (
            <>
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                className="text-sm text-accent hover:underline mt-1.5"
              >
                {open ? "Hide" : "Show"} evidence ({finding.evidence.length})
              </button>
              {open && (
                <pre className="mt-2 text-xs whitespace-pre-wrap break-all font-mono bg-canvas border border-line rounded-lg p-3 max-h-48 overflow-y-auto selectable text-muted">
                  {finding.evidence.join("\n")}
                </pre>
              )}
            </>
          )}
          {finding.remediation?.kind === "manual" && (
            <p className="text-sm text-fg/90 mt-2">
              <span className="text-muted">What to do: </span>
              {finding.remediation.instructions}
            </p>
          )}
        </div>
        {finding.remediation && finding.remediation.kind !== "manual" && (
          <Button
            size="sm"
            variant={confirming ? "danger" : "secondary"}
            busy={busy}
            disabled={disabled}
            onClick={onFix}
            icon={<Wrench className="w-3.5 h-3.5" />}
          >
            {confirming ? "Confirm fix" : "Fix"}
          </Button>
        )}
      </div>
    </li>
  );
}
