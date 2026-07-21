import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ScrollText } from "lucide-react";
import { useStore } from "../store";
import {
  EmptyState,
  ErrorBar,
  PageHeader,
  RefreshButton,
  Toggle,
} from "../components/ui";

const AUTO_REFRESH_MS = 5000;

export default function Logs() {
  const { logs } = useStore();
  const [levelFilter, setLevelFilter] = useState("");
  const [search, setSearch] = useState("");
  const [follow, setFollow] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const viewportRef = useRef<HTMLDivElement>(null);

  const entries = logs.data;

  const levels = useMemo(
    () => [...new Set(entries.map((l) => l.level))].filter(Boolean).sort(),
    [entries],
  );

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return entries.filter(
      (l) =>
        (levelFilter === "" || l.level === levelFilter) &&
        (needle === "" || l.message.toLowerCase().includes(needle)),
    );
  }, [entries, levelFilter, search]);

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
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    setAutoScroll(atBottom);
  };

  const levelColor = (level: string) => {
    const l = level.toUpperCase();
    if (l.includes("ERROR")) return "text-danger";
    if (l.includes("WARN")) return "text-warning";
    if (l.includes("INFO")) return "text-accent";
    if (l.includes("DEBUG") || l.includes("TRACE")) return "text-cyber-text-faint";
    return "text-cyber-text-dim";
  };

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader
        title="Log Viewer"
        subtitle="The most recent entries from odysync.log"
        actions={<RefreshButton onClick={() => void logs.refresh()} spinning={logs.loading} />}
      />

      {logs.error && <ErrorBar message={logs.error} />}

      <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 space-y-4">
        <Toggle
          label="Follow log"
          description={`Re-read the log every ${AUTO_REFRESH_MS / 1000} seconds`}
          checked={follow}
          onChange={setFollow}
        />
        <Toggle
          label="Auto-scroll"
          description="Stay pinned to the newest line. Scroll up to pause, scroll back down to resume."
          checked={autoScroll}
          onChange={(v) => {
            setAutoScroll(v);
            if (v) {
              const el = viewportRef.current;
              if (el) el.scrollTop = el.scrollHeight;
            }
          }}
        />
      </div>

      {entries.length > 0 && (
        <>
          <input
            type="text"
            placeholder="Filter messages..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
          />

          {levels.length > 0 && (
            <div className="flex gap-2 flex-wrap">
              <button
                type="button"
                onClick={() => setLevelFilter("")}
                className={`px-3 py-1 rounded-full text-xs ${
                  levelFilter === ""
                    ? "bg-accent/20 text-accent"
                    : "bg-cyber-surface text-cyber-text-dim"
                }`}
              >
                All
              </button>
              {levels.map((l) => (
                <button
                  type="button"
                  key={l}
                  onClick={() => setLevelFilter(l)}
                  className={`px-3 py-1 rounded-full text-xs ${
                    levelFilter === l
                      ? "bg-accent/20 text-accent"
                      : "bg-cyber-surface text-cyber-text-dim"
                  }`}
                >
                  {l}
                </button>
              ))}
            </div>
          )}
        </>
      )}

      {filtered.length === 0 && (
        <EmptyState
          icon={<ScrollText className="w-12 h-12" />}
          title={
            logs.loading && logs.loadedAt === null
              ? "Reading log file..."
              : entries.length === 0
                ? "No log entries yet."
                : "No entries match the current filter."
          }
        />
      )}

      {filtered.length > 0 && (
        <div className="relative">
          <div
            ref={viewportRef}
            onScroll={onScroll}
            className="rounded-lg border border-cyber-border bg-cyber-surface p-4 max-h-[60vh] overflow-y-auto font-mono text-xs space-y-1 select-text"
          >
            {filtered.map((l, i) => (
              <div key={i} className="flex gap-3">
                {l.timestamp && (
                  <span className="text-cyber-text-faint flex-shrink-0">{l.timestamp}</span>
                )}
                <span className={`flex-shrink-0 ${levelColor(l.level)}`}>{l.level}</span>
                <span className="text-cyber-text-dim break-all">{l.message}</span>
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
              className="absolute bottom-3 right-3 flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-accent/20 border border-accent/40 text-accent text-xs backdrop-blur-sm hover:bg-accent/30 transition-all"
            >
              <ArrowDown className="w-3.5 h-3.5" />
              Jump to newest
            </button>
          )}
        </div>
      )}
    </div>
  );
}
