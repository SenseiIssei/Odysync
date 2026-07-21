import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import * as api from "../api";
import {
  Card,
  ErrorBar,
  PageHeader,
  SuccessBar,
  errorText,
} from "../components/ui";

export default function Schedule() {
  const [frequency, setFrequency] = useState("daily");
  const [time, setTime] = useState("09:00");
  const [taskName, setTaskName] = useState("Odysync");
  const [scheduled, setScheduled] = useState<boolean | null>(null);
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const check = useCallback(async (name: string) => {
    if (!name.trim()) {
      setScheduled(null);
      return;
    }
    setChecking(true);
    try {
      setScheduled(await api.checkSchedule(name));
    } catch (e) {
      setError(errorText(e));
      setScheduled(null);
    } finally {
      setChecking(false);
    }
  }, []);

  // Debounced so typing in the task-name field does not spawn a schtasks
  // query per keystroke.
  useEffect(() => {
    const id = setTimeout(() => void check(taskName), 400);
    return () => clearTimeout(id);
  }, [taskName, check]);

  const create = async () => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.createSchedule({ frequency, time, task_name: taskName.trim() || null });
      setMessage(`Scheduled "${taskName}" ${frequency} at ${time}.`);
      await check(taskName);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const existed = await api.removeSchedule(taskName);
      setMessage(existed ? `Removed "${taskName}".` : "No schedule existed.");
      await check(taskName);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const statusLabel =
    checking || scheduled === null ? "Checking..." : scheduled ? "Scheduled" : "Not scheduled";

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <PageHeader
        title="Scheduled Updates"
        subtitle="Run a scan automatically on a schedule via the system task scheduler."
      />

      <Card>
        <div className="space-y-4">
          <div>
            <label htmlFor="freq" className="block text-sm font-medium mb-1">
              Frequency
            </label>
            <select
              id="freq"
              value={frequency}
              onChange={(e) => setFrequency(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            >
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
            </select>
          </div>

          <div>
            <label htmlFor="time" className="block text-sm font-medium mb-1">
              Time (24h)
            </label>
            <input
              id="time"
              type="time"
              value={time}
              onChange={(e) => setTime(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            />
          </div>

          <div>
            <label htmlFor="task" className="block text-sm font-medium mb-1">
              Task name
            </label>
            <input
              id="task"
              type="text"
              value={taskName}
              onChange={(e) => setTaskName(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
            />
          </div>

          <div className="flex items-center gap-2 text-sm">
            <span
              className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                scheduled
                  ? "bg-success/20 text-success glow-green"
                  : "bg-cyber-bg text-cyber-text-faint"
              }`}
            >
              {statusLabel}
            </span>
          </div>

          <div className="flex gap-3">
            <motion.button
              type="button"
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={create}
              disabled={busy || !taskName.trim()}
              className="px-5 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
            >
              {busy ? "Working..." : "Create Schedule"}
            </motion.button>
            <motion.button
              type="button"
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={remove}
              disabled={busy || !taskName.trim()}
              className="px-5 py-2 rounded-lg border border-cyber-border text-sm font-medium hover:bg-cyber-surface-2 disabled:opacity-50 transition-all"
            >
              Remove
            </motion.button>
          </div>
        </div>
      </Card>

      {message && <SuccessBar message={message} />}
      {error && <ErrorBar message={error} />}
    </div>
  );
}
