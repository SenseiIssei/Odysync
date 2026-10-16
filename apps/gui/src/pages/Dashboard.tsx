import type { ReactNode } from "react";
import {
  ArrowRight,
  Brush,
  CheckCircle2,
  Loader2,
  MemoryStick,
  Package,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  PageHeader,
} from "../components/ui";
import { formatSize } from "../format";
import type { Tab } from "../nav";

export default function Dashboard({ onNavigate }: { onNavigate: (tab: Tab) => void }) {
  const { sysInfo, backends, scan, cleanup, devices, components, security, defender } =
    useStore();

  const result = scan.data;
  const ready = result?.actionable ?? [];
  const needsAdmin = (result?.skipped ?? []).filter((s) =>
    s.reason.toLowerCase().includes("administrator"),
  );
  const scanned = scan.loadedAt !== null;
  const reclaimable = cleanup.data
    .filter((c) => c.selected_by_default && !c.error)
    .reduce((sum, c) => sum + c.bytes, 0);
  const outdatedRuntimes = components.data.filter((c) => c.state === "outdated").length;
  // "info" entries are an inventory (autostarts, listeners, extensions), not
  // problems; the Security page lists them separately and so does this tile.
  const findings = (security.data?.findings ?? []).filter((f) => f.severity !== "info");
  const serious = findings.filter((f) => f.severity === "critical" || f.severity === "high");
  const sources = backends.data.filter((b) => b.available && b.enabled);

  const lastScan = scan.loadedAt ? new Date(scan.loadedAt).toLocaleTimeString() : null;

  return (
    <>
      <PageHeader
        title="Overview"
        subtitle={
          sysInfo.data
            ? `${sysInfo.data.os} · ${sysInfo.data.elevated ? "running as administrator" : "running as a standard user"}`
            : "Looking at this machine…"
        }
        actions={
          <Button
            variant="secondary"
            onClick={() => void scan.refresh()}
            busy={scan.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            {scan.loading ? "Scanning…" : "Scan now"}
          </Button>
        }
      />

      {/* The one thing that matters most, stated plainly. */}
      <section className="rounded-xl border border-line bg-surface p-6 flex flex-wrap items-center justify-between gap-6">
        <div className="flex items-center gap-4 min-w-0">
          <div
            className={`w-12 h-12 rounded-xl flex items-center justify-center flex-shrink-0 ${
              !scanned
                ? "bg-raised text-muted"
                : ready.length > 0
                  ? "bg-accent/12 text-accent"
                  : "bg-success/12 text-success"
            }`}
          >
            {!scanned ? (
              <Loader2 className="w-6 h-6 animate-spin" />
            ) : ready.length > 0 ? (
              <Package className="w-6 h-6" />
            ) : (
              <CheckCircle2 className="w-6 h-6" />
            )}
          </div>
          <div className="min-w-0">
            <div className="text-lg font-semibold tracking-tight">
              {!scanned
                ? "Checking for updates…"
                : ready.length === 0
                  ? "Everything is up to date"
                  : `${ready.length} update${ready.length === 1 ? "" : "s"} ready to install`}
            </div>
            <div className="text-sm text-muted mt-0.5">
              {scanned && needsAdmin.length > 0
                ? `${needsAdmin.length} more need administrator rights. `
                : ""}
              {lastScan ? `Last checked at ${lastScan}.` : "This takes a few seconds."}
            </div>
          </div>
        </div>
        {ready.length > 0 && (
          <Button
            variant="primary"
            onClick={() => onNavigate("updates")}
            icon={<ArrowRight className="w-4 h-4" />}
          >
            Review updates
          </Button>
        )}
      </section>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <Tile
          icon={<MemoryStick className="w-4 h-4" />}
          title="Drivers & runtimes"
          onClick={() => onNavigate("drivers")}
          value={
            devices.data.length > 0
              ? devices.data.length === 1 ? "1 device needs attention" : `${devices.data.length} devices need attention`
              : "All devices working"
          }
          detail={
            outdatedRuntimes > 0
              ? `${outdatedRuntimes} runtime update${outdatedRuntimes === 1 ? "" : "s"} available`
              : "Runtimes are current"
          }
          tone={devices.data.length > 0 ? "warning" : "success"}
        />
        <Tile
          icon={<Brush className="w-4 h-4" />}
          title="Cleanup"
          onClick={() => onNavigate("maintenance")}
          value={
            cleanup.loadedAt === null
              ? "Measuring…"
              : `${formatSize(reclaimable)} safe to reclaim`
          }
          detail="Caches and leftovers only — nothing is deleted without asking"
          tone="neutral"
        />
        <Tile
          icon={<ShieldCheck className="w-4 h-4" />}
          title="Security"
          onClick={() => onNavigate("security")}
          value={
            security.loadedAt === null
              ? defender.data?.real_time_protection
                ? "Real-time protection on"
                : "Audit pending"
              : serious.length > 0
                ? `${serious.length} serious finding${serious.length === 1 ? "" : "s"}`
                : findings.length > 0
                  ? `${findings.length} minor finding${findings.length === 1 ? "" : "s"}`
                  : "No findings"
          }
          detail={
            defender.data
              ? `Defender definitions ${defender.data.signature_age_days === 0 ? "are current" : `are ${defender.data.signature_age_days} days old`}`
              : "Posture, persistence and network checks"
          }
          tone={serious.length > 0 ? "danger" : findings.length > 0 ? "warning" : "success"}
        />
      </div>

      <Card
        title="Where updates come from"
        description="Every source checks what it installs. Odysync names who did the checking instead of showing a generic tick."
      >
        {sources.length === 0 ? (
          <p className="text-sm text-muted">
            {backends.loading ? "Detecting update sources…" : "No update source is available."}
          </p>
        ) : (
          <ul className="divide-y divide-line -my-1">
            {sources.map((b) => (
              <li key={b.kind} className="flex items-center justify-between gap-4 py-2.5">
                <span className="text-sm text-fg truncate">{b.name}</span>
                <Badge tone={b.verification.includes("Odysync") ? "accent" : "neutral"}>
                  {b.verification}
                </Badge>
              </li>
            ))}
          </ul>
        )}
        {backends.data.some((b) => b.opt_in && !b.enabled && b.available) && (
          <p className="text-sm text-muted mt-4">
            winget and the Microsoft Store are optional. Turn them on in{" "}
            <button
              type="button"
              className="text-accent hover:underline"
              onClick={() => onNavigate("settings")}
            >
              Settings
            </button>{" "}
            if you want them as additional sources.
          </p>
        )}
      </Card>
    </>
  );
}

function Tile({
  icon,
  title,
  value,
  detail,
  tone,
  onClick,
}: {
  icon: ReactNode;
  title: string;
  value: string;
  detail: string;
  tone: "neutral" | "success" | "warning" | "danger";
  onClick: () => void;
}) {
  const dot = {
    neutral: "bg-faint",
    success: "bg-success",
    warning: "bg-warning",
    danger: "bg-danger",
  }[tone];
  return (
    <button
      type="button"
      onClick={onClick}
      className="group text-left rounded-xl border border-line bg-surface p-5 hover:border-line-strong hover:bg-raised/40 transition-colors"
    >
      <div className="flex items-center justify-between text-sm text-muted">
        <span className="flex items-center gap-2">
          <span className="text-faint">{icon}</span>
          {title}
        </span>
        <ArrowRight className="w-4 h-4 text-faint opacity-0 group-hover:opacity-100 transition-opacity" />
      </div>
      <div className="mt-3 flex items-center gap-2">
        <span className={`w-2 h-2 rounded-full flex-shrink-0 ${dot}`} />
        <span className="text-[15px] font-semibold text-fg">{value}</span>
      </div>
      <div className="text-sm text-muted mt-1">{detail}</div>
    </button>
  );
}
