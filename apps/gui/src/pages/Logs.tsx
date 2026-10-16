import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, FolderOpen, RefreshCw, ScrollText } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  SearchInput,
  Segmented,
  Switch,
} from "../components/ui";

const AUTO_REFRESH_MS = 5000;

export default function Logs() {
  const { logs } = useStore();
  const [level, setLevel] = useState("all");
  const [search, setSearch] = useState("");
  const [follow, setFollow] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const viewportRef = useRef<HTMLDivElement>(null);

  const entries = logs.data;

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return entries.filter((l) => {
      const lv = l.level.toUpperCase();
      const levelOk =
        level === "all" ||
        (level === "problems" && (lv.includes("WARN") || lv.includes("ERROR")));
      return levelOk && (needle === "" || l.message.toLowerCase().includes(needle));
    });
  }, [entries, level, search]);

  // Poll while following so the viewer behaves like `tail -f`.
  const refresh = logs.refresh;
  useEffect(() => {
    if (!follow) return;
    const id = setInterval(() => void refresh(), AUTO_REFRESH_MS);
    return () => clearInterval(id);
  }, [follow, refresh]);

  // Pin to the bottom as new lines arrive. Scrolling the viewport itself rather
  // than calling scrollIntoView keeps the page around it still.
  useEffect(() => {
    if (!autoScroll) return;
    const el = viewportRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [autoScroll, filtered.length]);

  // Scrolling up to read something releases the pin; returning to the bottom
  // re-engages it. Nothing is more irritating than a log that yanks you away
  // mid-read.
  const onScroll = () => {
    const el = viewportRef.current;
    if (!el) return;
    setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
  };

  const levelColor = (lv: string) => {
    const l = lv.toUpperCase();
    if (l.includes("ERROR")) return "text-danger";
    if (l.includes("WARN")) return "text-warning";
    if (l.includes("INFO")) return "text-info";
    return "text-faint";
  };

  return (
    <>
      <PageHeader
        title="Logs"
        subtitle="What Odysync did and why, from odysync.log."
        actions={
          <>
            <Button
              variant="ghost"
              icon={<FolderOpen className="w-4 h-4" />}
              onClick={() =>
                void api
                  .getLogFolder()
                  .then((dir) => api.revealPath(dir))
                  .catch(() => {})
              }
            >
              Open folder
            </Button>
            <Button
              onClick={() => void logs.refresh()}
              busy={logs.loading}
              icon={<RefreshCw className="w-4 h-4" />}
            >
              Refresh
            </Button>
          </>
        }
      />

      {logs.error && <ErrorBar message={logs.error} />}

      <Card padded={false}>
        <div className="flex flex-wrap items-center gap-3 px-5 py-3 border-b border-line">
          <SearchInput value={search} onChange={setSearch} placeholder="Filter messages" />
          <Segmented
            label="Level"
            value={level}
            onChange={setLevel}
            options={[
              { value: "all", label: "All" },
              { value: "problems", label: "Warnings & errors" },
            ]}
          />
          <label className="flex items-center gap-2 text-sm text-muted">
            <Switch label="Follow the log" checked={follow} onChange={setFollow} />
            Follow
          </label>
        </div>

        {filtered.length === 0 ? (
          <EmptyState
            icon={<ScrollText />}
            title={
              logs.loading && logs.loadedAt === null
                ? "Reading the log…"
                : entries.length === 0
                  ? "The log is empty"
                  : "No entry matches the filter"
            }
          />
        ) : (
          <div className="relative">
            <div
              ref={viewportRef}
              onScroll={onScroll}
              className="px-5 py-4 max-h-[62vh] overflow-y-auto font-mono text-[12.5px] leading-relaxed space-y-0.5 selectable"
            >
              {filtered.map((l, i) => (
                <div key={i} className="flex gap-3">
                  {l.timestamp && (
                    <span className="text-faint flex-shrink-0 tabular-nums">{l.timestamp}</span>
                  )}
                  <span className={`flex-shrink-0 w-12 ${levelColor(l.level)}`}>{l.level}</span>
                  <span className="text-muted break-all">{l.message}</span>
                </div>
              ))}
            </div>
            {!autoScroll && (
              <button
                type="button"
                onClick={() => {
                  const el = viewportRef.current;
                  if (el) el.scrollTop = el.scrollHeight;
                  setAutoScroll(true);
                }}
                className="absolute bottom-3 right-4 flex items-center gap-1.5 h-8 px-3 rounded-full bg-surface border border-line-strong text-sm text-fg shadow-sm hover:bg-raised"
              >
                <ArrowDown className="w-3.5 h-3.5" />
                Newest
              </button>
            )}
          </div>
        )}
      </Card>
    </>
  );
}
