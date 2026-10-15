import { ShieldCheck, ShieldAlert, Cpu, ArrowUpCircle } from "lucide-react";
import { Badge, Card } from "./ui";
import type { ScanResult } from "../types";

/**
 * Backend ids that come from the Windows Update Agent.
 *
 * These match `BackendKind::id()` in `odysync-core`. Kept as one object so the
 * order below is the order they render in, security first.
 */
const WINDOWS_UPDATE_BACKENDS = [
  {
    id: "windows-update",
    label: "Security & quality",
    icon: ShieldCheck,
    tone: "text-muted",
  },
  {
    id: "windows-defender-update",
    label: "Defender definitions",
    icon: ShieldAlert,
    tone: "text-muted",
  },
  { id: "windows-drivers", label: "Drivers", icon: Cpu, tone: "text-muted" },
  {
    id: "windows-feature-update",
    label: "Feature upgrades",
    icon: ArrowUpCircle,
    tone: "text-warning",
  },
] as const;

interface Row {
  label: string;
  icon: (typeof WINDOWS_UPDATE_BACKENDS)[number]["icon"];
  tone: string;
  actionable: number;
  blocked: number;
  reasons: string[];
}

/**
 * A summary of what Windows itself is offering.
 *
 * The generic update list is correct but flattens a real distinction: on an
 * unelevated run every Windows update lands in `skipped`, mixed in with
 * everything else the policy engine declined. On the machine this was built
 * against that meant 13 pending security updates sitting inside a list of 49
 * skipped entries, which reads as "nothing to do".
 *
 * Returns `null` when Windows Update contributed nothing to the scan, so the
 * card simply does not exist on macOS and Linux.
 */
export default function WindowsUpdateCard({
  scan,
  elevated,
}: {
  scan: ScanResult;
  elevated: boolean;
}) {
  const rows: Row[] = WINDOWS_UPDATE_BACKENDS.map(({ id, label, icon, tone }) => {
    const actionable = scan.actionable.filter((u) => u.backend === id);
    const blocked = scan.skipped.filter((s) => s.backend === id);
    return {
      label,
      icon,
      tone,
      actionable: actionable.length,
      blocked: blocked.length,
      // Deduplicated: fifteen updates blocked for the same reason should say
      // that reason once.
      reasons: [...new Set(blocked.map((s) => s.reason))],
    };
  }).filter((r) => r.actionable > 0 || r.blocked > 0);

  if (rows.length === 0) return null;

  const total = rows.reduce((n, r) => n + r.actionable + r.blocked, 0);
  const blockedByElevation = rows.some((r) =>
    r.reasons.some((reason) => reason.toLowerCase().includes("administrator")),
  );

  return (
    <Card
      title="Windows Update"
      description={`${total} pending from Microsoft, installed by Windows' own update agent.`}
      padded={false}
    >
      <ul className="divide-y divide-line border-t border-line">
        {rows.map((row) => {
          const Icon = row.icon;
          return (
            <li key={row.label} className="flex items-center gap-3 px-5 py-3 text-sm">
              <Icon className={`w-4 h-4 flex-shrink-0 ${row.tone}`} />
              <span className="flex-1 text-fg">{row.label}</span>
              <div className="flex items-center gap-2">
                {row.actionable > 0 && <Badge tone="success">{row.actionable} ready</Badge>}
                {row.blocked > 0 && (
                  // `title` alone would become the accessible name and replace
                  // the visible "N blocked" entirely, so a screen reader would
                  // hear the reason and never the count. aria-label carries
                  // both.
                  <span
                    title={row.reasons.join("; ")}
                    aria-label={`${row.blocked} waiting: ${row.reasons.join("; ")}`}
                  >
                    <Badge tone="neutral">{row.blocked} waiting</Badge>
                  </span>
                )}
              </div>
            </li>
          );
        })}
      </ul>

      {/* The single most common reason a Windows update cannot be applied, and
          one the user can act on immediately. Shown only when it is actually
          the cause, so it does not become background noise. */}
      {blockedByElevation && !elevated && (
        <p className="px-5 py-3 border-t border-line text-sm text-muted">
          Installing Windows updates needs administrator rights; checking for them does not.
          Use <span className="text-fg font-medium">Run as admin</span> in the title bar to
          install them.
        </p>
      )}

      {/* A feature-upgrade row can only appear when the backend is enabled —
          it is off by default, and a disabled backend contributes nothing to a
          scan. So the note explains why they are showing, not how to show
          them. */}
      {rows.some((r) => r.label === "Feature upgrades") && (
        <p className="px-5 py-3 border-t border-line text-sm text-warning">
          Feature upgrades replace the Windows version and take far longer than a normal
          update. They are listed because you turned them on in Settings.
        </p>
      )}
    </Card>
  );
}
