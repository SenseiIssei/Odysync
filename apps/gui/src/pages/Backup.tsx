import { useState } from "react";
import { DatabaseBackup, Plus, RefreshCw, RotateCcw } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Button,
  Card,
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
      await backup.refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setCreating(false);
    }
  };

  const restore = async (sequenceNumber: number, name: string) => {
    if (confirmRestore !== sequenceNumber) {
      setConfirmRestore(sequenceNumber);
      return;
    }
    setConfirmRestore(null);
    setError(null);
    setSuccess(null);
    try {
      await api.restoreBackup(sequenceNumber);
      setSuccess(`Returning to "${name}". Windows will restart.`);
    } catch (e) {
      setError(errorText(e));
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
      {elevated && !protectionEnabled && backup.loadedAt !== null && (
        <WarningBar message="System Protection is turned off for the system drive, so Windows will not keep restore points. Turn it on under System Properties → System Protection." />
      )}
      {success && <SuccessBar message={success} />}
      {error && <ErrorBar message={error} />}
      {backup.error && <ErrorBar message={backup.error} />}

      <Card title="Create a restore point">
        <div className="flex flex-wrap gap-3">
          <input
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="flex-1 min-w-[240px] h-9 px-3 rounded-lg border border-line bg-canvas text-sm placeholder:text-faint outline-none focus:border-accent"
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
                  : "No restore points"
              }
            />
          </div>
        ) : (
          <ul className="divide-y divide-line border-t border-line">
            {backups.map((b, i) => (
              <li key={`${b.sequence_number}-${i}`} className="flex items-center gap-4 px-5 py-3">
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium text-fg truncate">{b.name}</div>
                  <div className="text-sm text-muted">
                    {formatDate(b.created_at)} · #{b.sequence_number}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant={confirmRestore === b.sequence_number ? "danger" : "ghost"}
                  icon={<RotateCcw className="w-3.5 h-3.5" />}
                  disabled={!elevated}
                  onClick={() => void restore(b.sequence_number, b.name)}
                >
                  {confirmRestore === b.sequence_number ? "Restart and restore" : "Restore"}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
