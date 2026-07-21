import { useState } from "react";
import { motion } from "framer-motion";
import { History as HistoryIcon } from "lucide-react";
import * as api from "../api";
import { OutcomeIcon } from "./Updates";
import { useStore } from "../store";
import {
  EmptyState,
  ErrorBar,
  PageHeader,
  RefreshButton,
  errorText,
  formatDate,
} from "../components/ui";

export default function History() {
  const { history } = useStore();
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);

  const entries = history.data;
  const needle = search.trim().toLowerCase();
  const filtered = needle
    ? entries.filter(
        (e) =>
          e.package.toLowerCase().includes(needle) ||
          e.backend.toLowerCase().includes(needle),
      )
    : entries;

  const successCount = entries.filter((e) => e.status === "updated").length;
  const failCount = entries.filter(
    (e) => e.status !== "updated" && e.status !== "skipped",
  ).length;

  const clear = async () => {
    setClearing(true);
    setError(null);
    try {
      await api.clearUpdateHistory();
      await history.refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader
        title="Update History"
        subtitle="Every update Odysync has applied on this machine"
        actions={
          <>
            <RefreshButton onClick={() => void history.refresh()} spinning={history.loading} />
            {entries.length > 0 && (
              <button
                type="button"
                onClick={clear}
                disabled={clearing}
                className="px-3 py-1.5 rounded-lg border border-danger/30 text-danger text-xs hover:bg-danger/10 disabled:opacity-50 transition-all"
              >
                {clearing ? "Clearing..." : "Clear History"}
              </button>
            )}
          </>
        }
      />

      {error && <ErrorBar message={error} />}
      {history.error && <ErrorBar message={history.error} />}

      {entries.length > 0 && (
        <div className="flex gap-4 text-xs">
          <span className="text-success">{successCount} successful</span>
          <span className="text-danger">{failCount} failed</span>
          <span className="text-cyber-text-dim">{entries.length} total</span>
        </div>
      )}

      {entries.length > 0 && (
        <input
          type="text"
          placeholder="Search history..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
        />
      )}

      {entries.length === 0 && (
        <EmptyState
          icon={<HistoryIcon className="w-12 h-12" />}
          title={history.loading ? "Loading history..." : "No update history yet."}
          hint={history.loading ? undefined : "Applied updates are recorded here."}
        />
      )}

      {entries.length > 0 && filtered.length === 0 && (
        <EmptyState icon={<HistoryIcon className="w-12 h-12" />} title="No matches." />
      )}

      {filtered.length > 0 && (
        <div className="space-y-1">
          {filtered.map((e, i) => (
            <motion.div
              key={`${e.timestamp}-${e.package}-${i}`}
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: Math.min(i * 0.02, 0.4) }}
              className="flex items-center gap-3 p-3 rounded-lg bg-cyber-surface border border-cyber-border text-xs"
            >
              <OutcomeIcon status={e.status} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-cyber-text truncate">{e.package}</span>
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono flex-shrink-0">
                    {e.backend}
                  </span>
                </div>
                <div className="text-cyber-text-faint mt-0.5">
                  {e.from_version} -&gt; {e.to_version} · {formatDate(e.timestamp)}
                </div>
              </div>
              <span
                className="text-cyber-text-faint flex-shrink-0 max-w-[40%] truncate"
                title={e.detail}
              >
                {e.status === "updated" ? "updated" : e.detail || e.status}
              </span>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}
