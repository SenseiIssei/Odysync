import { useState } from "react";
import { DatabaseBackup, Plus, RefreshCw, RotateCcw } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store-context";
import {
  Button,
  Card,
  ConfirmBar,
  EmptyState,
  ErrorBar,
  InfoBar,
  PageHeader,
  SuccessBar,
  WarningBar,
} from "../components/ui";
import { errorText, formatDate } from "../format";

export default function Backup() {
  const { backup, sysInfo } = useStore();
  const [creating, setCreating] = useState(false);
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<number | null>(null);
  const [restoring, setRestoring] = useState(false);

  const { backups, protectionEnabled } = backup.data;
  const elevated = sysInfo.data?.elevated ?? false;
  const platform = sysInfo.data?.platform ?? "windows";

  if (platform !== "windows") {
    return (
      <>
        <PageHeader title="Restore points" />
        <Card>
          <EmptyState
            icon={<DatabaseBackup />}
            title="Restore points are a Windows feature"
            hint={
              platform === "macos"
                ? "On macOS, Time Machine keeps the same kind of safety net."
                : "On Linux, Timeshift or your file system's snapshots (Btrfs, ZFS) do this job."
            }
          />
        </Card>
      </>
    );
  }

  const create = async () => {
    setCreating(true);
    setError(null);
    setSuccess(null);
    try {
      const name = description.trim() || `Odysync, ${new Date().toLocaleString()}`;
      await api.createBackup(name);
      setSuccess("Restore point created.");
      setDescription("");
      await backup.reload();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setCreating(false);
    }
  };

  const restore = async (sequenceNumber: number, name: string) => {
    setConfirmRestore(null);
    setRestoring(true);
    setError(null);
    setSuccess(null);
    try {
      await api.restoreBackup(sequenceNumber);
      setSuccess(`Returning to "${name}". Windows will restart.`);
    } catch (e) {
      setError(`The restore did not start: ${errorText(e)}`);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <>
      <PageHeader
        title="Restore points"
        subtitle="Snapshots of Windows' system state. Odysync takes one before installing updates; you can take one yourself before any change you are unsure about."
        actions={
          <Button
            onClick={() => void backup.refresh()}
            busy={backup.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

      {!elevated && (
        <InfoBar message="Creating and listing restore points needs administrator rights. Use Run as admin in the title bar." />
      )}
      {elevated && !protectionEnabled && backup.loadedAt !== null && backup.error === null && (
        <WarningBar message="System Protection is turned off for the system drive, so Windows will not keep restore points. Turn it on under System Properties → System Protection." />
      )}
      {success && <SuccessBar message={success} />}
      {error && <ErrorBar message={error} />}
      {backup.error && <ErrorBar message={`Restore points could not be read: ${backup.error}`} />}

      <Card title="Create a restore point">
        <div className="flex flex-wrap gap-3">
          <input
            type="text"
            aria-label="Description of the restore point"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="flex-1 min-w-[240px] h-9 px-3 rounded-lg border border-control bg-canvas text-sm placeholder:text-faint outline-none focus:border-accent"
            placeholder="Description, e.g. before installing a new GPU driver"
          />
          <Button
            variant="primary"
            onClick={() => void create()}
            busy={creating}
            disabled={!elevated}
            icon={<Plus className="w-4 h-4" />}
          >
            Create
          </Button>
        </div>
        <p className="text-sm text-muted mt-3">
          Windows keeps at most one automatic restore point per day by default, and may quietly
          skip a manual one if another was made recently.
        </p>
      </Card>

      <Card title="Existing restore points" padded={false}>
        {backups.length === 0 ? (
          <div className="border-t border-line">
            <EmptyState
              icon={<DatabaseBackup />}
              title={
                backup.loading && backup.loadedAt === null
                  ? "Reading restore points…"
                  : backup.error
                    ? "Restore points could not be read"
                    : backup.loadedAt === null
                      ? "Not read yet"
                      : "No restore points"
              }
            />
          </div>
        ) : (
          <ul className="divide-y divide-line border-t border-line">
            {backups.map((b, i) => (
              <li key={`${b.sequence_number}-${i}`} className="px-5 py-3">
                <div className="flex items-center gap-4">
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-fg truncate">{b.name}</div>
                    <div className="text-sm text-muted">
                      {formatDate(b.created_at)} · #{b.sequence_number}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<RotateCcw className="w-3.5 h-3.5" />}
                    disabled={!elevated || restoring || confirmRestore === b.sequence_number}
                    aria-label={`Restore ${b.name}`}
                    onClick={() => setConfirmRestore(b.sequence_number)}
                  >
                    Restore
                  </Button>
                </div>
                {confirmRestore === b.sequence_number && (
                  <ConfirmBar
                    className="mt-3"
                    message={`Windows restarts and goes back to "${b.name}" from ${formatDate(b.created_at)}. Programs, drivers and updates installed since then are removed; your files stay. Save your work first.`}
                    confirmLabel="Restart and restore"
                    onConfirm={() => void restore(b.sequence_number, b.name)}
                    onCancel={() => setConfirmRestore(null)}
                  />
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
