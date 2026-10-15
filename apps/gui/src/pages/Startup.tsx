import { useMemo, useState } from "react";
import { Power, RefreshCw, Search } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  Switch,
} from "../components/ui";
import { errorText } from "../format";

export default function Startup() {
  const { startup, sysInfo } = useStore();
  const [toggling, setToggling] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);

  const programs = startup.data;
  const elevated = sysInfo.data?.elevated ?? false;
  const platform = sysInfo.data?.platform ?? "windows";

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return programs;
    return programs.filter(
      (p) =>
        (p.label ?? p.name).toLowerCase().includes(needle) ||
        p.command.toLowerCase().includes(needle),
    );
  }, [programs, search]);

  const enabledCount = programs.filter((p) => p.enabled).length;
  const rowKey = (name: string, location: string) => `${location}\u0000${name}`;

  const toggle = async (name: string, location: string, enable: boolean) => {
    const k = rowKey(name, location);
    setToggling(k);
    setError(null);
    try {
      await api.toggleStartupProgram(name, location, enable);
      await startup.refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setToggling(null);
    }
  };

  return (
    <>
      <PageHeader
        title="Startup"
        subtitle={
          programs.length > 0
            ? `${enabledCount} of ${programs.length} programs start when you sign in. Turning one off only stops it starting — nothing is uninstalled, and you can turn it back on here.`
            : "Programs that start when you sign in."
        }
        actions={
          <Button
            onClick={() => void startup.refresh()}
            busy={startup.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {startup.error && <ErrorBar message={startup.error} />}

      <Card padded={false}>
        {programs.length > 0 && (
          <div className="px-5 py-3 border-b border-line">
            <label className="flex items-center gap-2 h-9 px-3 rounded-lg border border-line bg-canvas focus-within:border-accent">
              <Search className="w-4 h-4 text-faint" />
              <input
                type="text"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="flex-1 bg-transparent text-sm outline-none placeholder:text-faint"
                placeholder="Filter by name or command"
              />
            </label>
          </div>
        )}

        {filtered.length === 0 ? (
          <EmptyState
            icon={<Power />}
            title={
              startup.loading && startup.loadedAt === null
                ? "Reading startup entries…"
                : programs.length === 0
                  ? "Nothing starts automatically"
                  : "No entries match the filter"
            }
          />
        ) : (
          <ul className="divide-y divide-line">
            {filtered.map((p) => {
              const k = rowKey(p.name, p.location);
              const machineWide = p.location.toUpperCase().startsWith("HKLM");
              const blocked = platform === "windows" && machineWide && !elevated;
              return (
                <li key={k} className="flex items-center gap-4 px-5 py-3">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className={`text-sm font-medium truncate ${p.enabled ? "text-fg" : "text-muted"}`}>
                        {p.label ?? p.name}
                      </span>
                      {machineWide && <Badge>all users</Badge>}
                    </div>
                    <div
                      className="text-xs text-faint mt-0.5 truncate font-mono selectable"
                      title={p.command}
                    >
                      {p.command}
                    </div>
                  </div>
                  <span
                    title={blocked ? "Entries for all users need administrator rights" : undefined}
                  >
                    <Switch
                      label={`${p.enabled ? "Disable" : "Enable"} ${p.label ?? p.name} at startup`}
                      checked={p.enabled}
                      disabled={toggling !== null || blocked}
                      onChange={(v) => void toggle(p.name, p.location, v)}
                    />
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </Card>
    </>
  );
}
