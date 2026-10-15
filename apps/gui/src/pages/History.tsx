import { useState } from "react";
import { ArrowRight, History as HistoryIcon, RefreshCw, Trash2 } from "lucide-react";
import * as api from "../api";
import { OutcomeIcon } from "./Updates";
import { useStore } from "../store";
import {
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  SearchInput,
} from "../components/ui";
import { errorText, formatDate } from "../format";

export default function History() {
  const { history } = useStore();
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  const entries = history.data;
  const needle = search.trim().toLowerCase();
  const filtered = needle
    ? entries.filter(
        (e) => e.package.toLowerCase().includes(needle) || e.backend.toLowerCase().includes(needle),
      )
    : entries;

  const ok = entries.filter((e) => e.status === "updated").length;
  const failed = entries.filter((e) => e.status !== "updated" && e.status !== "skipped").length;

  const clear = async () => {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
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
    <>
      <PageHeader
        title="History"
        subtitle={
          entries.length > 0
            ? `${ok} updated, ${failed} failed, ${entries.length} in total.`
            : "Every update Odysync installs is recorded here."
        }
        actions={
          <>
            <Button
              onClick={() => void history.refresh()}
              busy={history.loading}
              icon={<RefreshCw className="w-4 h-4" />}
            >
              Refresh
            </Button>
            {entries.length > 0 && (
              <Button
                variant={confirmClear ? "danger" : "ghost"}
                onClick={() => void clear()}
                busy={clearing}
                icon={<Trash2 className="w-4 h-4" />}
              >
                {confirmClear ? "Really clear?" : "Clear"}
              </Button>
            )}
          </>
        }
      />

      {error && <ErrorBar message={error} />}
      {history.error && <ErrorBar message={history.error} />}

      <Card padded={false}>
        {entries.length > 0 && (
          <div className="px-5 py-3 border-b border-line">
            <SearchInput value={search} onChange={setSearch} placeholder="Filter by name or source" />
          </div>
        )}
        {filtered.length === 0 ? (
          <EmptyState
            icon={<HistoryIcon />}
            title={
              history.loading && history.loadedAt === null
                ? "Loading history…"
                : entries.length === 0
                  ? "Nothing installed yet"
                  : "No entry matches the filter"
            }
          />
        ) : (
          <ul className="divide-y divide-line">
            {filtered.map((e, i) => (
              <li key={`${e.timestamp}-${e.package}-${i}`} className="flex items-start gap-3 px-5 py-3">
                <OutcomeIcon status={e.status} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium text-fg">{e.package}</div>
                  <div className="text-sm text-muted">
                    <span className="font-mono text-[13px] selectable">
                      {e.from_version}
                      <ArrowRight className="inline w-3.5 h-3.5 mx-1 -mt-0.5 text-faint" />
                      {e.to_version}
                    </span>
                    <span className="text-faint"> · {e.backend} · {formatDate(e.timestamp)}</span>
                  </div>
                  {e.status !== "updated" && e.detail && (
                    <div className="text-sm text-danger/90 mt-0.5 selectable">{e.detail}</div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
