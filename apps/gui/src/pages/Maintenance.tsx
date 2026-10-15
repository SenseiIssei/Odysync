import { useMemo, useState } from "react";
import {
  Brush,
  FileSearch,
  FolderOpen,
  HardDrive,
  RefreshCw,
  Stethoscope,
} from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  ErrorBar,
  PageHeader,
  SuccessBar,
  WarningBar,
} from "../components/ui";
import { errorText, formatSize } from "../format";
import type {
  CleanupItemDto,
  CleanupResultDto,
  LargeFileReport,
  VolumeOptimization,
} from "../types";

type Group = "safe" | "choice" | "permanent" | "report";

const GROUPS: { id: Group; title: string; hint: string }[] = [
  {
    id: "safe",
    title: "Safe to remove",
    hint: "Caches and leftovers that rebuild themselves. Selected by default.",
  },
  {
    id: "choice",
    title: "Your choice",
    hint: "Harmless, but with a small cost — read the line under each before ticking it.",
  },
  {
    id: "permanent",
    title: "Cannot be undone",
    hint: "These remove something you might want back. Odysync asks again before deleting them.",
  },
  {
    id: "report",
    title: "Measured only",
    hint: "Shown so you know where the space went. Remove these with the tool named in each note.",
  },
];

function groupOf(item: CleanupItemDto): Group {
  if (item.report_only) return "report";
  if (item.needs_confirmation) return "permanent";
  if (item.selected_by_default) return "safe";
  return "choice";
}

export default function Maintenance() {
  const { cleanup, sysInfo } = useStore();
  const windows = (sysInfo.data?.platform ?? "windows") === "windows";
  const elevated = sysInfo.data?.elevated ?? false;

  // Choices the user made, layered over each category's default.
  const [choices, setChoices] = useState<Record<string, boolean>>({});
  const [confirming, setConfirming] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [results, setResults] = useState<CleanupResultDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const items = cleanup.data;
  // The component store's size is only known after DISM analyses it, which
  // takes minutes; it stays selectable at "size unknown" rather than never.
  const selectable = (i: CleanupItemDto) =>
    !i.report_only && !i.error && (i.bytes > 0 || i.id === "component-store");
  const isSelected = (i: CleanupItemDto) =>
    selectable(i) && (choices[i.id] ?? i.selected_by_default);
  const selected = useMemo(
    () => items.filter((i) => selectable(i) && (choices[i.id] ?? i.selected_by_default)),
    [items, choices],
  );
  const selectedBytes = selected.reduce((n, i) => n + i.bytes, 0);
  const permanent = selected.filter((i) => i.needs_confirmation);

  const clean = async () => {
    if (selected.length === 0) return;
    if (permanent.length > 0 && !confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    setCleaning(true);
    setError(null);
    setResults(null);
    try {
      setResults(await api.cleanupRun(selected.map((i) => i.id)));
      setChoices({});
      await cleanup.refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setCleaning(false);
    }
  };

  const removed = results?.reduce((n, r) => n + r.removed_bytes, 0) ?? 0;
  const skippedCount = results?.reduce((n, r) => n + r.skipped.length, 0) ?? 0;

  return (
    <>
      <PageHeader
        title="Cleanup"
        subtitle="Measure first, then remove only what you tick. Nothing outside the folders listed here is ever touched."
        actions={
          <Button
            onClick={() => void cleanup.refresh()}
            busy={cleanup.loading}
            disabled={cleaning}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Measure again
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {results && (
        <SuccessBar
          message={`Reclaimed ${formatSize(removed)}.${
            skippedCount > 0
              ? ` ${skippedCount} item${skippedCount === 1 ? " was" : "s were"} in use or protected and left alone.`
              : ""
          }`}
        />
      )}

      <Card padded={false}>
        <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-4">
          <div>
            <div className="text-sm text-muted">Selected</div>
            <div className="text-2xl font-semibold tracking-tight">
              {cleanup.loadedAt === null ? "Measuring…" : formatSize(selectedBytes)}
            </div>
          </div>
          <Button
            variant={confirming ? "danger" : "primary"}
            onClick={() => void clean()}
            busy={cleaning}
            disabled={selected.length === 0 || cleanup.loading}
            icon={<Brush className="w-4 h-4" />}
          >
            {confirming ? "Yes, delete permanently" : `Clean ${selected.length} categor${selected.length === 1 ? "y" : "ies"}`}
          </Button>
        </div>
        {confirming && (
          <div className="px-5 pb-4">
            <WarningBar
              message={
                <>
                  You selected {permanent.length === 1 ? "something" : "things"} that cannot be
                  undone:{" "}
                  {permanent.map((p) => (
                    <span key={p.id} className="block mt-1">
                      <strong className="font-medium">{p.label}</strong> — {p.consequence}
                    </span>
                  ))}
                </>
              }
            />
          </div>
        )}
      </Card>

      {GROUPS.map((group) => {
        const rows = items.filter((i) => groupOf(i) === group.id);
        if (rows.length === 0) return null;
        return (
          <Card key={group.id} title={group.title} description={group.hint} padded={false}>
            <ul className="divide-y divide-line border-t border-line">
              {rows.map((item) => (
                <li key={item.id} className="flex items-start gap-3 px-5 py-3.5">
                  <div className="pt-0.5 w-4">
                    {group.id !== "report" && (
                      <Checkbox
                        label={`Clean ${item.label}`}
                        checked={isSelected(item)}
                        disabled={!selectable(item) || cleaning}
                        onChange={(v) => {
                          setConfirming(false);
                          setChoices((c) => ({ ...c, [item.id]: v }));
                        }}
                      />
                    )}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-fg">{item.label}</div>
                    <p className="text-sm text-muted mt-0.5">{item.consequence}</p>
                    {item.note && <p className="text-sm text-faint mt-1">{item.note}</p>}
                  </div>
                  <div className="text-right flex-shrink-0">
                    {item.error ? (
                      <Badge tone="warning">could not measure</Badge>
                    ) : (
                      <>
                        <div className="text-sm font-medium tabular-nums text-fg">
                          {item.bytes > 0
                            ? formatSize(item.bytes)
                            : item.id === "component-store"
                              ? "size unknown"
                              : "—"}
                        </div>
                        {item.file_count > 0 && (
                          <div className="text-xs text-faint tabular-nums">
                            {item.file_count.toLocaleString()} files
                          </div>
                        )}
                      </>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          </Card>
        );
      })}

      <LargeFiles />
      {windows && <DiskTools elevated={elevated} />}
    </>
  );
}

function LargeFiles() {
  const [report, setReport] = useState<LargeFileReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const find = async () => {
    setBusy(true);
    setError(null);
    try {
      setReport(await api.largeFiles(25, 100));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card
      title="Large files"
      description="The biggest files in your home folder. Odysync only lists them — what is worth keeping is your call."
      padded={false}
      actions={
        <Button size="sm" onClick={() => void find()} busy={busy} icon={<FileSearch className="w-3.5 h-3.5" />}>
          {report ? "Search again" : "Find large files"}
        </Button>
      }
    >
      {error && (
        <div className="px-5 pb-4">
          <ErrorBar message={error} />
        </div>
      )}
      {report && (
        <>
          <ul className="divide-y divide-line border-t border-line">
            {report.files.length === 0 && (
              <li className="px-5 py-4 text-sm text-muted">No file is larger than 100 MB.</li>
            )}
            {report.files.map((f) => (
              <li key={f.path} className="flex items-center gap-4 px-5 py-2.5">
                <span className="w-20 text-sm font-medium tabular-nums text-fg flex-shrink-0">
                  {formatSize(f.bytes)}
                </span>
                <span className="flex-1 min-w-0 truncate text-sm text-muted font-mono selectable" title={f.path}>
                  {f.path}
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<FolderOpen className="w-3.5 h-3.5" />}
                  onClick={() => void api.revealPath(f.path).catch(() => {})}
                >
                  Show
                </Button>
              </li>
            ))}
          </ul>
          <p className="px-5 py-3 border-t border-line text-xs text-faint">
            {report.scanned.toLocaleString()} files looked at
            {report.truncated ? " — stopped at the time limit, so larger files may exist." : "."}
          </p>
        </>
      )}
    </Card>
  );
}

function DiskTools({ elevated }: { elevated: boolean }) {
  const [plan, setPlan] = useState<VolumeOptimization[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [output, setOutput] = useState<{ title: string; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (label: string, work: () => Promise<string>) => {
    setBusy(label);
    setError(null);
    try {
      setOutput({ title: label, text: await work() });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title="Disk and system repair"
      description="Windows' own tools, run the right way: TRIM for SSDs and defragmentation only for hard disks; DISM and SFC to repair system files."
      padded={false}
    >
      <ul className="divide-y divide-line border-t border-line">
        <li className="flex items-center gap-4 px-5 py-3.5">
          <HardDrive className="w-4 h-4 text-faint flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium">Optimise drives</div>
            <p className="text-sm text-muted">
              {plan
                ? plan.map((v) => `${v.drive} ${v.media_type} → ${v.operation === "Retrim" ? "TRIM" : "defragment"}`).join(" · ")
                : "Odysync picks the operation from the drive type; defragmenting an SSD only wears it out."}
            </p>
          </div>
          {plan ? (
            <Button
              size="sm"
              disabled={!elevated || busy !== null}
              busy={busy === "Optimise drives"}
              title={elevated ? undefined : "Needs administrator rights"}
              onClick={() =>
                void run("Optimise drives", async () =>
                  (await api.diskOptimizationRun()).join("\n"),
                )
              }
            >
              Optimise
            </Button>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              busy={busy === "plan"}
              onClick={() =>
                void (async () => {
                  setBusy("plan");
                  try {
                    setPlan(await api.diskOptimizationPlan());
                  } catch (e) {
                    setError(errorText(e));
                  } finally {
                    setBusy(null);
                  }
                })()
              }
            >
              Check drives
            </Button>
          )}
        </li>
        <li className="flex items-center gap-4 px-5 py-3.5">
          <Stethoscope className="w-4 h-4 text-faint flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium">Repair system files</div>
            <p className="text-sm text-muted">
              Runs DISM and SFC to find and fix damaged Windows files. Takes 10–30 minutes.
            </p>
          </div>
          <Button
            size="sm"
            disabled={!elevated || busy !== null}
            busy={busy === "Repair system files"}
            title={elevated ? undefined : "Needs administrator rights"}
            onClick={() => void run("Repair system files", () => api.runMaintenance("system_health"))}
          >
            Run
          </Button>
        </li>
      </ul>
      {error && (
        <div className="px-5 py-4 border-t border-line">
          <ErrorBar message={error} />
        </div>
      )}
      {output && (
        <div className="px-5 py-4 border-t border-line">
          <div className="text-sm font-medium mb-2">{output.title}</div>
          <pre className="text-xs whitespace-pre-wrap break-words text-muted font-mono max-h-64 overflow-y-auto selectable">
            {output.text || "(no output)"}
          </pre>
        </div>
      )}
    </Card>
  );
}
