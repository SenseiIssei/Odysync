import { useState } from "react";
import { motion } from "framer-motion";
import { AlertTriangle, DatabaseBackup } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  RefreshButton,
  SuccessBar,
  errorText,
  formatDate,
} from "../components/ui";

export default function Backup() {
  const { backup, sysInfo } = useStore();
  const [creating, setCreating] = useState(false);
  const [backupName, setBackupName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<number | null>(null);

  const { backups, protectionEnabled } = backup.data;
  const elevated = sysInfo.data?.elevated ?? false;

  const create = async () => {
    setCreating(true);
    setError(null);
    setSuccess(null);
    try {
      const name = backupName.trim() || `Odysync Backup ${new Date().toLocaleString()}`;
      await api.createBackup(name);
      setSuccess("Restore point created.");
      setBackupName("");
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
      setSuccess(`Restore to "${name}" started. Windows will restart.`);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="max-w-3xl mx-auto space-y-4">
      <PageHeader
        title="Backups & Restore Points"
        subtitle="Create a Windows system restore point before applying updates"
        actions={<RefreshButton onClick={() => void backup.refresh()} spinning={backup.loading} />}
      />

      {!elevated && (
        <div className="flex items-start gap-2 text-xs text-warning bg-warning/5 rounded-lg px-4 py-3 border border-warning/30">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
          Creating and listing restore points requires administrator rights. Use "Run as
          Admin" in the title bar to manage them.
        </div>
      )}

      {elevated && !protectionEnabled && backup.loadedAt !== null && (
        <div className="flex items-start gap-2 text-xs text-warning bg-warning/5 rounded-lg px-4 py-3 border border-warning/30">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
          System Protection appears to be disabled. Enable it in Windows System Properties
          before relying on restore points.
        </div>
      )}

      <Card title="Create New Restore Point">
        <div className="flex gap-3 flex-wrap">
          <input
            type="text"
            value={backupName}
            onChange={(e) => setBackupName(e.target.value)}
            className="flex-1 min-w-[200px] px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            placeholder="Description (optional)"
          />
          <motion.button
            type="button"
            whileHover={{ scale: 1.02 }}
            whileTap={{ scale: 0.98 }}
            onClick={create}
            disabled={creating}
            className="flex items-center gap-2 px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
          >
            <DatabaseBackup className={`w-4 h-4 ${creating ? "animate-pulse" : ""}`} />
            {creating ? "Creating..." : "Create"}
          </motion.button>
        </div>
        <p className="text-xs text-cyber-text-faint mt-2">
          Windows only creates one automatic restore point per 24 hours by default; a manual
          one may be silently skipped if a recent one exists.
        </p>
      </Card>

      {success && <SuccessBar message={success} />}
      {error && <ErrorBar message={error} />}
      {backup.error && <ErrorBar message={backup.error} />}

      {backups.length === 0 && (
        <EmptyState
          icon={<DatabaseBackup className="w-12 h-12" />}
          title={
            backup.loading && backup.loadedAt === null
              ? "Reading restore points..."
              : "No restore points found."
          }
        />
      )}

      {backups.length > 0 && (
        <div className="space-y-1">
          <h3 className="text-sm font-bold text-accent mb-2">Existing Restore Points</h3>
          {backups.map((b, i) => (
            <motion.div
              key={`${b.sequence_number}-${i}`}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: Math.min(i * 0.03, 0.3) }}
              className="flex items-center gap-3 p-3 rounded-lg bg-cyber-surface border border-cyber-border text-xs"
            >
              <DatabaseBackup className="w-4 h-4 text-accent flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <div className="font-medium text-cyber-text truncate">{b.name}</div>
                <div className="text-cyber-text-faint mt-0.5">
                  #{b.sequence_number} · {formatDate(b.created_at)} · {b.backup_type}
                </div>
              </div>
              <button
                type="button"
                onClick={() => void restore(b.sequence_number, b.name)}
                className="px-3 py-1 rounded bg-warning/10 border border-warning/30 text-warning text-xs font-medium hover:bg-warning/20 transition-all flex-shrink-0"
              >
                {confirmRestore === b.sequence_number ? "Confirm — restarts PC" : "Restore"}
              </button>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}
