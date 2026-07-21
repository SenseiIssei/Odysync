import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  Bug,
  CheckCircle,
  Eye,
  Network,
  RefreshCw,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Siren,
  Wrench,
} from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  SuccessBar,
  errorText,
  formatDate,
} from "../components/ui";
import type { Remediation, SecurityFinding, Severity } from "../types";

/** Severities that describe something wrong, as opposed to something noted. */
const ACTIONABLE: Severity[] = ["critical", "high", "medium", "low"];

const SEVERITY_ORDER: Severity[] = [...ACTIONABLE, "info"];

const SEVERITY_STYLE: Record<Severity, { text: string; border: string; bg: string; label: string }> =
  {
    critical: {
      text: "text-danger",
      border: "border-danger/40",
      bg: "bg-danger/5",
      label: "Critical",
    },
    high: { text: "text-danger", border: "border-danger/30", bg: "bg-danger/5", label: "High" },
    medium: {
      text: "text-warning",
      border: "border-warning/30",
      bg: "bg-warning/5",
      label: "Medium",
    },
    low: {
      text: "text-accent",
      border: "border-accent/30",
      bg: "bg-accent/5",
      label: "Low",
    },
    // Deliberately not "Info" and deliberately not alarming: these are normal
    // things worth being able to see — your own apps, games, Spotify — not
    // suspected malware. Presenting them alongside threats made the whole page
    // read as an infection report.
    info: {
      text: "text-cyber-text-dim",
      border: "border-cyber-border",
      bg: "bg-cyber-surface",
      label: "Watching",
    },
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
  const report = security.data;

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
  const threats = useMemo(
    () => sorted.filter((f) => f.severity !== "info"),
    [sorted],
  );
  const watching = useMemo(() => sorted.filter((f) => f.severity === "info"), [sorted]);

  const findings = useMemo(
    () =>
      severityFilter === "all"
        ? threats
        : sorted.filter((f) => f.severity === severityFilter),
    [sorted, threats, severityFilter],
  );

  const counts = useMemo(() => {
    const c: Record<Severity, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
    for (const f of report?.findings ?? []) c[f.severity]++;
    return c;
  }, [report]);

  const failedSections = (report?.sections ?? []).filter((s) => !s.ok);

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

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader
        title="Security"
        subtitle="Malware, persistence, unauthorised access and system hardening"
        actions={
          <button
            type="button"
            onClick={() => void security.refresh()}
            disabled={security.loading || busy !== null}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-cyber-border text-xs text-cyber-text-dim hover:text-accent hover:border-accent/30 disabled:opacity-50 transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${security.loading ? "animate-spin" : ""}`} />
            {security.loading ? "Auditing..." : "Re-audit"}
          </button>
        }
      />

      {/* Honesty about what this is. Overstating it would be worse than useless
          for someone who has just been compromised. */}
      <div className="flex items-start gap-2 text-xs text-cyber-text-dim bg-cyber-surface rounded-lg px-4 py-3 border border-cyber-border">
        <ShieldAlert className="w-4 h-4 flex-shrink-0 mt-px text-accent" />
        <span>
          This audits your system's security posture and known compromise indicators, and
          drives Microsoft Defender for the actual malware scanning. It is{" "}
          <strong className="text-cyber-text">not an antivirus engine of its own</strong>, and
          a clean result here does not prove the machine is clean. For a suspected active
          infection, also run a Microsoft Defender Offline scan.
        </span>
      </div>

      {!elevated && (
        <div className="flex items-start gap-2 text-xs text-warning bg-warning/5 rounded-lg px-4 py-3 border border-warning/30">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
          Several checks (machine-wide persistence, service configuration, Defender control)
          need administrator rights and are skipped without them. Use "Run as Admin".
        </div>
      )}

      {error && <ErrorBar message={error} />}
      {notice && <SuccessBar message={notice} />}

      {/* Defender status + actions */}
      <Card title="Microsoft Defender">
        {defender.data ? (
          <div className="space-y-3">
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
              <StatusPill
                label="Real-time protection"
                ok={defender.data.real_time_protection}
              />
              <StatusPill label="Tamper protection" ok={defender.data.tamper_protection} />
              <StatusPill label="Antivirus" ok={defender.data.antivirus_enabled} />
              <StatusPill
                label={`Signatures ${defender.data.signature_age_days}d old`}
                ok={defender.data.signature_age_days <= 7}
              />
            </div>
            <div className="text-xs text-cyber-text-faint space-y-0.5">
              <div>Signature version: {defender.data.signature_version || "unknown"}</div>
              {defender.data.last_quick_scan && (
                <div>Last quick scan: {defender.data.last_quick_scan}</div>
              )}
              {defender.data.last_full_scan && (
                <div>Last full scan: {defender.data.last_full_scan}</div>
              )}
            </div>
          </div>
        ) : (
          <p className="text-xs text-cyber-text-faint">
            {defender.loading ? "Querying Defender..." : "Defender status unavailable."}
          </p>
        )}

        <div className="flex flex-wrap gap-2 mt-4">
          <ActionButton
            label="Update signatures"
            busy={busy === "sigs"}
            disabled={busy !== null}
            onClick={() =>
              run("sigs", api.updateDefenderSignatures, async () => {
                await defender.refresh();
              })
            }
          />
          <ActionButton
            label="Quick scan"
            busy={busy === "quick"}
            disabled={busy !== null}
            onClick={() =>
              run("quick", api.defenderQuickScan, async () => {
                await Promise.allSettled([security.refresh(), defender.refresh()]);
              })
            }
          />
          <ActionButton
            label="Full scan (slow)"
            busy={busy === "full"}
            disabled={busy !== null}
            onClick={() =>
              run("full", api.defenderFullScan, async () => {
                await Promise.allSettled([security.refresh(), defender.refresh()]);
              })
            }
          />
        </div>
        <p className="text-xs text-cyber-text-faint mt-2">
          A full scan reads every file on every drive and can run for hours. It keeps going if
          you close this window to the tray.
        </p>
      </Card>

      {/* Summary */}
      {report && (
        <div className="grid grid-cols-3 md:grid-cols-5 gap-2">
          {SEVERITY_ORDER.map((sev) => {
            const style = SEVERITY_STYLE[sev];
            const active = severityFilter === sev;
            return (
              <button
                type="button"
                key={sev}
                onClick={() => setSeverityFilter(active ? "all" : sev)}
                className={`p-3 rounded-xl border text-left transition-all ${style.border} ${
                  active ? style.bg : "bg-cyber-surface"
                } hover:${style.bg}`}
              >
                <div className={`text-xl font-bold ${counts[sev] > 0 ? style.text : "text-cyber-text-faint"}`}>
                  {counts[sev]}
                </div>
                <div className="text-xs text-cyber-text-dim">{style.label}</div>
              </button>
            );
          })}
        </div>
      )}

      {failedSections.length > 0 && (
        <div className="rounded-lg border border-warning/30 bg-warning/5 px-4 py-3 text-xs text-warning space-y-1">
          <div className="font-medium">
            {failedSections.length} check{failedSections.length !== 1 ? "s" : ""} could not run —
            this audit is incomplete
          </div>
          {failedSections.map((s) => (
            <div key={s.name} className="font-mono break-words">
              {s.name}: {s.error ?? "unknown error"}
            </div>
          ))}
        </div>
      )}

      {security.loading && security.loadedAt === null && (
        <EmptyState
          icon={<RefreshCw className="w-12 h-12 animate-spin text-accent" />}
          title="Auditing this machine..."
          hint="Checking Defender, persistence, network listeners, file integrity and hardening."
        />
      )}

      {report && findings.length === 0 && (
        <EmptyState
          icon={<CheckCircle className="w-12 h-12 text-success" />}
          title={
            severityFilter === "all"
              ? "Nothing needs your attention."
              : `No ${SEVERITY_STYLE[severityFilter as Severity].label.toLowerCase()} findings.`
          }
          hint={
            severityFilter === "all"
              ? `${watching.length} item${watching.length !== 1 ? "s" : ""} are being watched below. No indicators were found by these checks — which is not the same as proof of a clean machine.`
              : undefined
          }
        />
      )}

      {findings.length > 0 && (
        <div className="space-y-2">
          {report && (
            <div className="text-xs text-cyber-text-faint">
              Audited {formatDate(report.scannedAt)}
            </div>
          )}
          {findings.map((f, i) => (
            <FindingCard
              key={f.id}
              finding={f}
              index={i}
              busy={busy === `fix:${f.id}`}
              disabled={busy !== null}
              confirming={confirming === f.id}
              onFix={() => void applyFix(f)}
            />
          ))}
        </div>
      )}

      {/* Inventory, collapsed. Your own apps, games and tools live here — this
          is "what is running on this machine", not "what is wrong with it". */}
      {severityFilter === "all" && watching.length > 0 && (
        <details className="rounded-lg border border-cyber-border bg-cyber-surface/50">
          <summary className="cursor-pointer px-4 py-3 text-sm text-cyber-text-dim hover:text-cyber-text">
            <Eye className="w-4 h-4 inline mr-2 -mt-0.5" />
            Watching — {watching.length} item{watching.length !== 1 ? "s" : ""} noted, none
            flagged as a problem
          </summary>
          <div className="px-3 pb-3 space-y-2">
            <p className="text-xs text-cyber-text-faint px-1">
              Autostart entries, network listeners and browser extensions found on this
              machine. Normal software — your own projects, games, Spotify — appears here.
              Nothing in this list is being called malware; it is listed so you can spot
              something you do not recognise.
            </p>
            {watching.map((f, i) => (
              <FindingCard
                key={f.id}
                finding={f}
                index={i}
                busy={busy === `fix:${f.id}`}
                disabled={busy !== null}
                confirming={confirming === f.id}
                onFix={() => void applyFix(f)}
              />
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

function StatusPill({ label, ok }: { label: string; ok: boolean }) {
  return (
    <div
      className={`flex items-center gap-1.5 px-2 py-1.5 rounded-lg border ${
        ok ? "border-success/30 bg-success/5" : "border-danger/30 bg-danger/5"
      }`}
    >
      {ok ? (
        <CheckCircle className="w-3.5 h-3.5 text-success flex-shrink-0" />
      ) : (
        <AlertTriangle className="w-3.5 h-3.5 text-danger flex-shrink-0" />
      )}
      <span className={`truncate ${ok ? "text-success" : "text-danger"}`}>{label}</span>
    </div>
  );
}

function ActionButton({
  label,
  onClick,
  busy,
  disabled,
}: {
  label: string;
  onClick: () => void;
  busy: boolean;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all"
    >
      {busy && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
      {busy ? "Running..." : label}
    </button>
  );
}

function FindingCard({
  finding,
  index,
  busy,
  disabled,
  confirming,
  onFix,
}: {
  finding: SecurityFinding;
  index: number;
  busy: boolean;
  disabled: boolean;
  confirming: boolean;
  onFix: () => void;
}) {
  const style = SEVERITY_STYLE[finding.severity];
  const [open, setOpen] = useState(finding.severity === "critical");

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: Math.min(index * 0.02, 0.3) }}
      className={`rounded-lg border ${style.border} ${style.bg} p-4`}
    >
      <div className="flex items-start gap-3">
        <div className={`${style.text} flex-shrink-0 mt-0.5`}>
          {CATEGORY_ICON[finding.category] ?? <ShieldAlert className="w-4 h-4" />}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className={`text-[10px] px-1.5 py-0.5 rounded font-bold uppercase ${style.text}`}>
              {style.label}
            </span>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono">
              {finding.category}
            </span>
            <span className="font-medium text-sm">{finding.title}</span>
          </div>
          <p className="text-xs text-cyber-text-dim mt-1">{finding.detail}</p>

          {finding.evidence.length > 0 && (
            <>
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                className="text-xs text-accent hover:underline mt-2"
              >
                {open ? "Hide" : "Show"} evidence ({finding.evidence.length})
              </button>
              {open && (
                <pre className="mt-2 text-[11px] whitespace-pre-wrap break-all font-mono bg-cyber-bg/60 border border-cyber-border rounded p-2 max-h-40 overflow-y-auto select-text text-cyber-text-dim">
                  {finding.evidence.join("\n")}
                </pre>
              )}
            </>
          )}
        </div>

        {finding.remediation && finding.remediation.kind !== "manual" && (
          <button
            type="button"
            onClick={onFix}
            disabled={disabled}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg border text-xs font-medium flex-shrink-0 disabled:opacity-50 transition-all ${
              confirming
                ? "bg-danger/10 border-danger/40 text-danger hover:bg-danger/20"
                : "bg-cyber-surface border-cyber-border text-cyber-text-dim hover:text-accent hover:border-accent/30"
            }`}
          >
            {busy ? (
              <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Wrench className="w-3.5 h-3.5" />
            )}
            {busy ? "Fixing..." : confirming ? "Confirm" : "Fix"}
          </button>
        )}
      </div>

      {finding.remediation?.kind === "manual" && (
        <p className="text-xs text-cyber-text-dim mt-3 pt-3 border-t border-cyber-border">
          <span className="text-accent font-medium">What to do: </span>
          {finding.remediation.instructions}
        </p>
      )}
    </motion.div>
  );
}
