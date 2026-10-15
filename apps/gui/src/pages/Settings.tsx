import { useState } from "react";
import { CheckCircle2, Lock, Monitor, Moon, Sun } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import { useTheme, type ThemeChoice } from "../theme";
import {
  Badge,
  Button,
  Card,
  ErrorBar,
  PageHeader,
  ResourceView,
  Segmented,
  Switch,
  Toggle,
} from "../components/ui";
import { errorText } from "../format";
import type { BackendDto, Config } from "../types";

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export default function Settings() {
  const { config, backends, sysInfo } = useStore();
  const theme = useTheme();
  // Unsaved edits. `null` means "no edits": the form shows the saved config.
  const [edits, setEdits] = useState<Config | null>(null);
  const draft = edits ?? config.data;
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const patch = (p: Partial<Config>) => {
    setEdits((d) => {
      const base = d ?? config.data;
      return base ? { ...base, ...p } : base;
    });
    setSaved(false);
  };

  const patchPolicy = (p: Partial<Config["policy"]>) => {
    setEdits((d) => {
      const base = d ?? config.data;
      return base ? { ...base, policy: { ...base.policy, ...p } } : base;
    });
    setSaved(false);
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    setError(null);
    try {
      await api.saveConfig(draft);
      await Promise.all([config.refresh(), backends.refresh()]);
      setEdits(null);
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const revert = async () => {
    await config.refresh();
    setEdits(null);
    setSaved(false);
    setError(null);
  };

  const dirty =
    edits !== null && config.data !== null && JSON.stringify(edits) !== JSON.stringify(config.data);

  /** Whether a source is switched on in the draft. */
  const isOn = (b: BackendDto) => {
    if (!draft) return b.enabled;
    if (draft.disabled_backends.some((d) => same(d, b.kind))) return false;
    if (b.opt_in) return (draft.enabled_backends ?? []).some((e) => same(e, b.kind));
    return true;
  };

  const setOn = (b: BackendDto, on: boolean) => {
    if (!draft) return;
    const disabled = draft.disabled_backends.filter((d) => !same(d, b.kind));
    const enabled = (draft.enabled_backends ?? []).filter((e) => !same(e, b.kind));
    if (b.opt_in) {
      patch({ disabled_backends: disabled, enabled_backends: on ? [...enabled, b.kind] : enabled });
    } else {
      patch({
        disabled_backends: on ? disabled : [...disabled, b.kind],
        enabled_backends: enabled,
      });
    }
  };

  const standard = backends.data.filter((b) => !b.opt_in);
  const optional = backends.data.filter((b) => b.opt_in);
  const platform = sysInfo.data?.platform ?? "windows";

  return (
    <>
      <PageHeader
        title="Settings"
        subtitle="How Odysync looks, where it gets updates from, and the rules it never breaks."
      />

      <Card title="Appearance">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="text-sm font-medium">Theme</div>
            <div className="text-sm text-muted">Follow the system, or pick one.</div>
          </div>
          <Segmented<ThemeChoice>
            label="Theme"
            value={theme.choice}
            onChange={theme.setChoice}
            options={[
              { value: "system", label: "System", icon: <Monitor className="w-3.5 h-3.5" /> },
              { value: "light", label: "Light", icon: <Sun className="w-3.5 h-3.5" /> },
              { value: "dark", label: "Dark", icon: <Moon className="w-3.5 h-3.5" /> },
            ]}
          />
        </div>
      </Card>

      <ResourceView resource={config} loadingLabel="Loading settings…">
        {draft && (
          <>
            <Card
              title="Update sources"
              description="Everything that is installed on this machine is used by default. Each source names who verifies what it installs."
              padded={false}
            >
              <SourceList backends={standard} isOn={isOn} setOn={setOn} />
              {optional.length > 0 && (
                <>
                  <div className="px-5 pt-5 pb-2 border-t border-line">
                    <div className="text-sm font-medium">Optional sources</div>
                    <p className="text-sm text-muted">
                      Off until you turn them on. Odysync does not need winget: installed
                      programs are updated straight from their publishers.
                    </p>
                  </div>
                  <SourceList backends={optional} isOn={isOn} setOn={setOn} />
                </>
              )}
            </Card>

            <Card title="Safety rules">
              <div className="space-y-5">
                <Toggle
                  label="Stable releases only"
                  description="Never install alpha, beta, release-candidate or nightly builds."
                  checked={draft.policy.stable_only}
                  onChange={(v) => patchPolicy({ stable_only: v })}
                />
                <Toggle
                  label="Require known versions"
                  description="Skip an update when either version cannot be read — a guess is how downgrades happen."
                  checked={draft.policy.require_known_versions}
                  onChange={(v) => patchPolicy({ require_known_versions: v })}
                />
                {platform === "windows" && (
                  <Toggle
                    label="Create a restore point before installing"
                    description="A Windows system restore point before each run. Needs administrator rights."
                    checked={draft.restore_point}
                    onChange={(v) => patch({ restore_point: v })}
                  />
                )}
                {draft.policy.holds.length > 0 && (
                  <div className="pt-4 border-t border-line">
                    <div className="text-sm font-medium flex items-center gap-1.5 mb-2">
                      <Lock className="w-3.5 h-3.5 text-muted" />
                      Held packages
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {draft.policy.holds.map((h) => (
                        <span key={h.package} title={h.note ?? undefined}>
                          <Badge>
                            <span className="font-mono">
                              {h.package}
                              {h.pin ? ` @ ${h.pin}` : ""}
                            </span>
                          </Badge>
                        </span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </Card>

            <StartupCard platform={platform} />

            <Card title="Automation">
              <div className="space-y-5">
                <Toggle
                  label="Install automatically after a scan"
                  description="Applies everything the safety rules allow without asking. Off is the careful choice."
                  checked={draft.auto_apply}
                  onChange={(v) => patch({ auto_apply: v })}
                />
                <Toggle
                  label="Notifications"
                  description="When a scan finds updates and when an installation finishes."
                  checked={draft.notifications}
                  onChange={(v) => patch({ notifications: v })}
                />
                <div className="grid gap-5 sm:grid-cols-3 pt-1">
                  <NumberField
                    label="Scan every (hours)"
                    hint="0 means only when you ask"
                    min={0}
                    max={168}
                    value={draft.scan_interval_hours}
                    onChange={(v) => patch({ scan_interval_hours: v })}
                  />
                  <NumberField
                    label="Retries"
                    hint="For network hiccups"
                    min={0}
                    max={10}
                    value={draft.max_retries}
                    onChange={(v) => patch({ max_retries: v })}
                  />
                  <NumberField
                    label="Timeout (seconds)"
                    hint="Per source"
                    min={10}
                    max={3600}
                    value={draft.backend_timeout_secs}
                    onChange={(v) => patch({ backend_timeout_secs: v })}
                  />
                </div>
              </div>
            </Card>

            <Card title="Network">
              <div className="grid gap-5 sm:grid-cols-[1fr_200px]">
                <div>
                  <label htmlFor="proxy" className="block text-sm font-medium mb-1.5">
                    HTTP proxy
                  </label>
                  <input
                    id="proxy"
                    type="text"
                    value={draft.proxy_url ?? ""}
                    onChange={(e) => patch({ proxy_url: e.target.value.trim() || null })}
                    className="w-full h-9 px-3 rounded-lg border border-line bg-canvas text-sm placeholder:text-faint focus:border-accent outline-none"
                    placeholder="http://proxy.example.com:8080"
                  />
                  <p className="text-sm text-muted mt-1.5">Leave empty for a direct connection.</p>
                </div>
                <div>
                  <label htmlFor="conc" className="block text-sm font-medium mb-1.5">
                    Parallel sources: <span className="tabular-nums">{draft.concurrency}</span>
                  </label>
                  <input
                    id="conc"
                    type="range"
                    min={1}
                    max={16}
                    value={draft.concurrency}
                    onChange={(e) => patch({ concurrency: Number(e.target.value) })}
                    className="w-full accent-[var(--accent)] mt-2"
                  />
                </div>
              </div>
            </Card>

            <Card
              title="Excluded packages"
              description="One per line, as a bare id (vscode) or source:id (pip:requests). Excluded packages are never updated."
            >
              <textarea
                value={draft.policy.exclude.join("\n")}
                onChange={(e) =>
                  patchPolicy({
                    exclude: e.target.value
                      .split("\n")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
                rows={4}
                className="w-full px-3 py-2 rounded-lg border border-line bg-canvas text-sm font-mono placeholder:text-faint focus:border-accent outline-none selectable"
                placeholder={"vscode\npip:requests"}
              />
            </Card>

            {error && <ErrorBar message={error} />}

            <div className="sticky bottom-0 -mx-8 px-8 py-3 bg-canvas/90 backdrop-blur-sm border-t border-line flex items-center gap-3">
              <Button variant="primary" onClick={() => void save()} busy={saving} disabled={!dirty}>
                Save changes
              </Button>
              {dirty && (
                <Button variant="ghost" onClick={() => void revert()}>
                  Discard
                </Button>
              )}
              {saved && !dirty && (
                <span className="text-sm text-success flex items-center gap-1.5">
                  <CheckCircle2 className="w-4 h-4" />
                  Saved
                </span>
              )}
            </div>
          </>
        )}
      </ResourceView>
    </>
  );
}

function SourceList({
  backends,
  isOn,
  setOn,
}: {
  backends: BackendDto[];
  isOn: (b: BackendDto) => boolean;
  setOn: (b: BackendDto, on: boolean) => void;
}) {
  return (
    <ul className="divide-y divide-line border-t border-line">
      {backends.map((b) => (
        <li key={b.kind} className="flex items-center gap-4 px-5 py-3">
          <div className="flex-1 min-w-0">
            <div className="text-sm text-fg truncate" title={b.kind}>
              {b.name}
            </div>
            <div className="text-xs text-faint" title={b.verification_detail}>
              {b.verification}
            </div>
          </div>
          {!b.available && <Badge>not installed</Badge>}
          <Switch
            label={`Use ${b.name}`}
            checked={b.available && isOn(b)}
            disabled={!b.available}
            onChange={(v) => setOn(b, v)}
          />
        </li>
      ))}
    </ul>
  );
}

/**
 * Starting with the operating system.
 *
 * Kept out of the config file deliberately: the source of truth is the OS's
 * own startup registration, so a toggle here reads back from it rather than
 * from whatever the config last claimed.
 */
function StartupCard({ platform }: { platform: string }) {
  const { autostart, security } = useStore();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cfg = autostart.data;

  const set = async (enabled: boolean, minimized: boolean) => {
    setBusy(true);
    setError(null);
    try {
      if (enabled) await api.enableAutostart(minimized);
      else await api.disableAutostart();
      await autostart.refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card title="Startup">
      <div className="space-y-5">
        <Toggle
          label="Start Odysync when you sign in"
          description="Registered for your user only — no administrator rights needed."
          checked={cfg?.enabled ?? false}
          disabled={busy || autostart.loading}
          onChange={(v) => void set(v, cfg?.minimized ?? true)}
        />
        <Toggle
          label="Start in the background"
          description="Stay in the tray instead of opening the window."
          checked={cfg?.minimized ?? false}
          disabled={busy || autostart.loading || !(cfg?.enabled ?? false)}
          onChange={(v) => void set(true, v)}
        />
        {platform === "windows" && (
          <Toggle
            label="Run the security audit at startup"
            description="Checks for persistence, tampering and weak settings each time Odysync starts."
            checked={security.scanOnStartup}
            onChange={security.setScanOnStartup}
          />
        )}
        {error && <ErrorBar message={error} />}
      </div>
    </Card>
  );
}

function NumberField({
  label,
  hint,
  min,
  max,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  min: number;
  max: number;
  value: number;
  onChange: (v: number) => void;
}) {
  const clamp = (n: number) => Math.min(max, Math.max(min, n));
  return (
    <div>
      <label className="block text-sm font-medium mb-1.5">{label}</label>
      <input
        type="number"
        min={min}
        max={max}
        value={value}
        onChange={(e) => {
          const parsed = parseInt(e.target.value, 10);
          onChange(Number.isNaN(parsed) ? min : clamp(parsed));
        }}
        className="w-full h-9 px-3 rounded-lg border border-line bg-canvas text-sm tabular-nums focus:border-accent outline-none"
      />
      <p className="text-xs text-faint mt-1">{hint}</p>
    </div>
  );
}
