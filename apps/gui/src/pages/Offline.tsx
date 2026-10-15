import { useState } from "react";
import { CheckCircle2, HardDriveDownload, RefreshCw, ShieldCheck, Trash2, XCircle } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  ResourceView,
  SuccessBar,
} from "../components/ui";
import { errorText, formatDate, formatSize } from "../format";

export default function Offline() {
  const { offline } = useStore();
  const [verifying, setVerifying] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** Verification result keyed by `backend\u0000package_id`, so it survives reordering. */
  const [verified, setVerified] = useState<Record<string, boolean>>({});
  const [confirmClear, setConfirmClear] = useState(false);

  const { status, entries } = offline.data;
  const key = (backend: string, id: string) => `${backend}\u0000${id}`;

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const verify = async () => {
    setVerifying(true);
    setError(null);
    try {
      const results = await api.verifyOfflineCache();
      const map: Record<string, boolean> = {};
      entries.forEach((e, i) => {
        map[key(e.backend, e.package_id)] = results[i] ?? false;
      });
      setVerified(map);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setVerifying(false);
    }
  };

  const clearAll = () => {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
    void run(async () => {
      await api.clearOfflineManifest();
      setVerified({});
      await offline.refresh();
    });
  };

  const prune = () =>
    void run(async () => {
      const dropped = await api.pruneOfflineCache();
      setNotice(
        dropped === 0
          ? "Every cached entry still has its file on disk."
          : `Removed ${dropped} entr${dropped === 1 ? "y" : "ies"} whose file was missing.`,
      );
      await offline.refresh();
    });

  return (
    <>
      <PageHeader
        title="Offline cache"
        subtitle={`Installers kept on disk, for updating a machine without internet. ${entries.length} cached · ${formatSize(status?.cache_size_bytes ?? 0)}.`}
        actions={
          <Button
            onClick={() => void offline.refresh()}
            busy={offline.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {notice && <SuccessBar message={notice} />}

      <ResourceView resource={offline} loadingLabel="Checking the cache…">
        <Card
          padded={false}
          title="Cached installers"
          description="Each file's code signature is checked when it is cached; a signature that does not validate is refused."
          actions={
            entries.length > 0 ? (
              <>
                <Button size="sm" onClick={() => void verify()} busy={verifying} disabled={busy} icon={<ShieldCheck className="w-3.5 h-3.5" />}>
                  Verify
                </Button>
                <Button size="sm" variant="ghost" onClick={prune} disabled={busy || verifying}>
                  Prune missing
                </Button>
                <Button size="sm" variant={confirmClear ? "danger" : "ghost"} onClick={clearAll} disabled={busy} icon={<Trash2 className="w-3.5 h-3.5" />}>
                  {confirmClear ? "Delete all?" : "Clear"}
                </Button>
              </>
            ) : undefined
          }
        >
          {entries.length === 0 ? (
            <div className="border-t border-line">
              <EmptyState
                icon={<HardDriveDownload />}
                title="Nothing cached"
                hint="Most update sources download and verify inside their own process, so there is usually no installer file to keep. Entries appear here only for downloads cached on purpose."
              />
            </div>
          ) : (
            <ul className="divide-y divide-line border-t border-line">
              {entries.map((e) => {
                const v = verified[key(e.backend, e.package_id)];
                return (
                  <li key={key(e.backend, e.package_id)} className="flex items-center gap-3 px-5 py-3">
                    {v === true && <CheckCircle2 className="w-4 h-4 text-success" aria-label="Checksum verified" />}
                    {v === false && <XCircle className="w-4 h-4 text-danger" aria-label="Checksum mismatch or file missing" />}
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium text-fg truncate">{e.package_id}</div>
                      <div className="text-sm text-muted">
                        {e.backend} · v{e.version} · {formatSize(e.size_bytes)} · {formatDate(e.cached_at)}
                      </div>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          await api.removeOfflineEntry(e.package_id, e.backend);
                          await offline.refresh();
                        })
                      }
                    >
                      Remove
                    </Button>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
      </ResourceView>
    </>
  );
}
