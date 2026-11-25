/** The running scan and the running install, shown the same on every page. */

import { useState } from "react";
import { X } from "lucide-react";
import { errorText } from "../format";
import { nameList } from "../scan-health";
import { useStore } from "../store-context";
import { Button, Card, ErrorBar, Meter } from "./ui";

/** A cancel button that says so when the cancel itself fails. */
function CancelButton({
  label,
  cancelling,
  onCancel,
}: {
  label: string;
  cancelling: boolean;
  onCancel: () => Promise<void>;
}) {
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button
        size="sm"
        aria-label={label}
        busy={cancelling}
        icon={<X className="w-3.5 h-3.5" />}
        onClick={() => {
          setError(null);
          onCancel().catch((e: unknown) => setError(errorText(e)));
        }}
      >
        {cancelling ? "Cancelling…" : "Cancel"}
      </Button>
      {error && <ErrorBar message={`Could not cancel: ${error}`} />}
    </div>
  );
}

/** "Checked 12 of 21 sources, still waiting for Windows Update". */
export function ScanProgressView() {
  const { scan } = useStore();
  if (!scan.loading) return null;
  const p = scan.progress;
  const text =
    p === null
      ? "Starting the check…"
      : `Checked ${p.done} of ${p.total} sources${
          p.waiting.length > 0 ? `, still waiting for ${nameList(p.waiting)}` : ""
        }`;
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-sm text-muted">{text}</span>
        <CancelButton
          label="Cancel the check"
          cancelling={scan.cancelling}
          onCancel={scan.cancel}
        />
      </div>
      <Meter
        label="Update sources checked"
        value={p && p.total > 0 ? p.done / p.total : 0}
      />
    </div>
  );
}

/** What is being installed, with a way to stop after it. */
export function ApplyProgressView() {
  const { apply } = useStore();
  if (!apply.running) return null;
  const p = apply.progress;
  const text = apply.cancelling
    ? p?.package
      ? `Finishing ${p.package}, then stopping`
      : "Stopping after the current package"
    : p?.phase === "done"
      ? "Finishing up…"
      : p?.package
        ? `${apply.dryRun ? "Simulating" : "Installing"} ${p.package}`
        : apply.dryRun
          ? "Simulating…"
          : "Preparing the installation…";
  return (
    <Card>
      <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
        <span className="text-sm text-fg min-w-0 truncate">{text}</span>
        <span className="flex items-center gap-3">
          {p && p.total > 0 && (
            <span className="text-sm text-muted flex-shrink-0 tabular-nums">
              {p.current} of {p.total}
            </span>
          )}
          <CancelButton
            label="Cancel the installation"
            cancelling={apply.cancelling}
            onCancel={apply.cancel}
          />
        </span>
      </div>
      <Meter
        label="Installation progress"
        value={p && p.total > 0 ? (p.percent ?? (p.current / p.total) * 100) / 100 : 0}
      />
    </Card>
  );
}

/**
 * Speaks the progress of a scan or an install. Mounted once, outside the
 * pages, so it is heard on whichever page is open.
 *
 * A scan reports dozens of steps; only every quarter is worth interrupting
 * somebody for.
 */
export function ProgressAnnouncer() {
  const { scan, apply } = useStore();
  let text = "";
  if (apply.running) {
    text = apply.cancelling
      ? "Stopping after the current package"
      : apply.progress?.package
        ? `Installing ${apply.progress.package}, ${apply.progress.current} of ${apply.progress.total}`
        : "Installing updates";
  } else if (scan.loading) {
    const p = scan.progress;
    const quarter = p && p.total > 0 ? Math.floor((p.done / p.total) * 4) * 25 : 0;
    text = quarter > 0 ? `Checking for updates, ${quarter} percent` : "Checking for updates";
  } else if (scan.health.state === "failed") {
    text = "The check for updates failed";
  } else if (scan.data && !scan.fromCache) {
    const n = scan.data.actionable.length;
    text =
      `Check finished, ${n} update${n === 1 ? "" : "s"} ready` +
      (scan.health.state === "partial" ? ", some sources could not be checked" : "");
  }
  return (
    <div role="status" aria-live="polite" className="sr-only">
      {text}
    </div>
  );
}
