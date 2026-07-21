import { useState } from "react";
import { motion } from "framer-motion";
import { HardDrive, RefreshCw, Stethoscope, Wrench } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import { Card, ErrorBar, PageHeader, errorText } from "../components/ui";

interface Action {
  id: string;
  label: string;
  icon: React.ReactNode;
  desc: string;
  /** Needs admin rights to do anything useful. */
  needsElevation?: boolean;
  destructive?: boolean;
}

const ACTIONS: Action[] = [
  {
    id: "temp_cleanup",
    label: "Temp Cleanup",
    icon: <Wrench className="w-5 h-5" />,
    desc: "Delete temporary files and folders that are no longer in use",
  },
  {
    id: "clean_recycle_bin",
    label: "Empty Recycle Bin",
    icon: <HardDrive className="w-5 h-5" />,
    desc: "Permanently delete everything currently in the Recycle Bin",
    destructive: true,
  },
  {
    id: "system_health",
    label: "System Health (DISM/SFC)",
    icon: <Stethoscope className="w-5 h-5" />,
    desc: "Scan and repair Windows system file integrity",
    needsElevation: true,
  },
];

export default function Maintenance() {
  const { sysInfo, logs } = useStore();
  const [running, setRunning] = useState<string | null>(null);
  const [result, setResult] = useState<{ action: string; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

  const elevated = sysInfo.data?.elevated ?? false;

  const runAction = async (action: Action) => {
    if (action.destructive && confirming !== action.id) {
      setConfirming(action.id);
      return;
    }
    setConfirming(null);
    setRunning(action.id);
    setError(null);
    setResult(null);
    try {
      const text = await api.runMaintenance(action.id);
      setResult({ action: action.label, text });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setRunning(null);
      void logs.refresh();
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <PageHeader
        title="System Maintenance"
        subtitle="These actions clean and inspect your system — they are not package updates."
      />

      <div className="grid gap-3">
        {ACTIONS.map((a, i) => {
          const blocked = a.needsElevation && !elevated;
          return (
            <motion.div
              key={a.id}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: i * 0.05 }}
              className="flex items-center gap-4 p-4 rounded-lg bg-cyber-surface border border-cyber-border hover:border-accent/20 transition-all"
            >
              <div className="text-accent flex-shrink-0">{a.icon}</div>
              <div className="flex-1 min-w-0">
                <h3 className="font-medium text-sm">{a.label}</h3>
                <p className="text-xs text-cyber-text-dim">{a.desc}</p>
                {blocked && (
                  <p className="text-xs text-warning mt-1">
                    Requires administrator rights — use "Run as Admin" in the title bar.
                  </p>
                )}
              </div>
              <button
                type="button"
                onClick={() => void runAction(a)}
                disabled={running !== null || blocked}
                className={`px-4 py-1.5 rounded-lg border text-sm font-medium transition-all disabled:opacity-50 flex-shrink-0 ${
                  confirming === a.id
                    ? "bg-danger/10 border-danger/30 text-danger hover:bg-danger/20"
                    : "bg-accent/10 border-accent/30 text-accent hover:bg-accent/20"
                }`}
              >
                {running === a.id ? (
                  <RefreshCw className="w-4 h-4 animate-spin" />
                ) : confirming === a.id ? (
                  "Confirm"
                ) : (
                  "Run"
                )}
              </button>
            </motion.div>
          );
        })}
      </div>

      {confirming && (
        <div className="text-xs text-warning">
          This action cannot be undone. Click Confirm to proceed, or run a different action
          to cancel.
        </div>
      )}

      {result && (
        <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
          <Card title={`Result — ${result.action}`}>
            <pre className="text-xs whitespace-pre-wrap break-words text-cyber-text-dim font-mono max-h-64 overflow-y-auto select-text">
              {result.text || "(no output)"}
            </pre>
          </Card>
        </motion.div>
      )}

      {error && <ErrorBar message={error} />}
    </div>
  );
}
