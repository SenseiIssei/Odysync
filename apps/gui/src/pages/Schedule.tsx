import { useCallback, useEffect, useState } from "react";
import { CalendarClock, Trash2 } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  ErrorBar,
  PageHeader,
  Segmented,
  SuccessBar,
} from "../components/ui";
import { errorText } from "../format";

const SCHEDULER: Record<string, string> = {
  windows: "Windows Task Scheduler",
  macos: "launchd",
  linux: "a systemd user timer",
};

export default function Schedule() {
  const { sysInfo } = useStore();
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

  // Debounced so typing in the name field does not query the scheduler per
  // keystroke.
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
      setMessage(`Odysync will check for updates ${frequency === "daily" ? "every day" : "every week"} at ${time}.`);
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
      setMessage(existed ? "The schedule was removed." : "There was no schedule to remove.");
      await check(taskName);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const scheduler = SCHEDULER[sysInfo.data?.platform ?? "windows"] ?? "the system scheduler";

  return (
    <>
      <PageHeader
        title="Schedule"
        subtitle={`Let ${scheduler} run Odysync for you. A scheduled run installs the updates that pass every safety rule — the same ones the Updates page shows as ready — and nothing else.`}
      />

      {message && <SuccessBar message={message} />}
      {error && <ErrorBar message={error} />}

      <Card
        title="Automatic scan"
        actions={
          <Badge tone={scheduled ? "success" : "neutral"}>
            {checking || scheduled === null ? "Checking…" : scheduled ? "Active" : "Not scheduled"}
          </Badge>
        }
      >
        <div className="grid gap-5 sm:grid-cols-3">
          <div>
            <div className="text-sm font-medium mb-1.5">How often</div>
            <Segmented
              label="Frequency"
              value={frequency}
              onChange={setFrequency}
              options={[
                { value: "daily", label: "Daily" },
                { value: "weekly", label: "Weekly" },
              ]}
            />
          </div>
          <div>
            <label htmlFor="time" className="block text-sm font-medium mb-1.5">
              At
            </label>
            <input
              id="time"
              type="time"
              value={time}
              onChange={(e) => setTime(e.target.value)}
              className="w-full h-9 px-3 rounded-lg border border-line bg-canvas text-sm outline-none focus:border-accent"
            />
          </div>
          <div>
            <label htmlFor="task" className="block text-sm font-medium mb-1.5">
              Task name
            </label>
            <input
              id="task"
              type="text"
              value={taskName}
              onChange={(e) => setTaskName(e.target.value)}
              className="w-full h-9 px-3 rounded-lg border border-line bg-canvas text-sm outline-none focus:border-accent"
            />
          </div>
        </div>
        <div className="flex gap-2 mt-5">
          <Button
            variant="primary"
            onClick={() => void create()}
            busy={busy}
            disabled={!taskName.trim()}
            icon={<CalendarClock className="w-4 h-4" />}
          >
            {scheduled ? "Update schedule" : "Create schedule"}
          </Button>
          {scheduled && (
            <Button variant="ghost" onClick={() => void remove()} disabled={busy} icon={<Trash2 className="w-4 h-4" />}>
              Remove
            </Button>
          )}
        </div>
      </Card>
    </>
  );
}
