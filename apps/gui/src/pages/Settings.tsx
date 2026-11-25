import { useId, useState } from "react";
import { CheckCircle2, Lock, LockOpen, Monitor, Moon, Sun } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store-context";
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
import type { BackendDto, Config, Hold } from "../types";

/** The exclusion list as it is stored, from what was typed. */
const parseExclude = (text: string) =>
  text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export default function Settings() {
  const { config, backends, sysInfo } = useStore();
  const theme = useTheme();
  // Unsaved edits. `null` means "no edits": the form shows the saved config.
  const [edits, setEdits] = useState<Config | null>(null);
  const draft = edits ?? config.data;
  // The exclusion list as typed. Parsing it on every keystroke would remove
  // the line break the moment it is entered.
  const [excludeText, setExcludeText] = useState<string | null>(null);
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
    const toSave =
      excludeText === null
        ? draft
        : { ...draft, policy: { ...draft.policy, exclude: parseExclude(excludeText) } };
    setSaving(true);
    setError(null);
    try {
      await api.saveConfig(toSave);
      await Promise.all([config.reload(), backends.reload()]);
      setEdits(null);
      setExcludeText(null);
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
    setExcludeText(null);
    setSaved(false);
    setError(null);
  };

  // Holds are released on the spot and are not part of what is saved here, so
  // they do not make the form dirty.
  const comparable = (c: Config) => JSON.stringify({ ...c, policy: { ...c.policy, holds: [] } });
  const excludeEdited =
    excludeText !== null &&
    config.data !== null &&
    parseExclude(excludeText).join("\n") !== config.data.policy.exclude.join("\n");
  const dirty =
    excludeEdited ||
    (edits !== null && config.data !== null && comparable(edits) !== comparable(config.data));

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
                  description="Skip an update when either version cannot be read. A guess is how downgrades happen."
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
                <Holds holds={config.data?.policy.holds ?? []} />
              </div>
            </Card>

            <StartupCard platform={platform} />

            <Card title="Automation">
              <div className="space-y-5">
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
              <label htmlFor="proxy" className="block text-sm font-medium mb-1.5">
                HTTP proxy
              </label>
              <input
                id="proxy"
                type="text"
                value={draft.proxy_url ?? ""}
                onChange={(e) => patch({ proxy_url: e.target.value.trim() || null })}
                aria-describedby="proxy-hint"
                className="w-full h-9 px-3 rounded-lg border border-control bg-canvas text-sm placeholder:text-faint focus:border-accent outline-none"
                placeholder="http://proxy.example.com:8080"
              />
              <p id="proxy-hint" className="text-sm text-muted mt-1.5">
                Leave empty for a direct connection.
              </p>
            </Card>

            <Card
              title="Excluded packages"
              description="One per line, as a bare id (vscode) or source:id (pip:requests). Excluded packages are never updated."
            >
              <textarea
                aria-label="Excluded packages, one per line"
                value={excludeText ?? draft.policy.exclude.join("\n")}
                onChange={(e) => {
                  setExcludeText(e.target.value);
                  setSaved(false);
                }}
                onBlur={() => {
                  if (excludeText === null) return;
                  patchPolicy({ exclude: parseExclude(excludeText) });
                  setExcludeText(null);
                }}
                rows={4}
                className="w-full px-3 py-2 rounded-lg border border-control bg-canvas text-sm font-mono placeholder:text-faint focus:border-accent outline-none selectable"
                placeholder={"vscode\npip:requests"}
              />
            </Card>

            {error && <ErrorBar message={error} />}

            <div className="sticky bottom-0 -mx-8 px-8 py-3 bg-canvas border-t border-line flex items-center gap-3">
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

/**
 * Packages that are never updated until they are released.
 *
 * Read from the saved config, not from unsaved edits: holding and releasing
 * take effect at once.
 */
function Holds({ holds }: { holds: Hold[] }) {
  const { config } = useStore();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [released, setReleased] = useState<string | null>(null);

  const release = async (pkg: string) => {
    setBusy(pkg);
    setError(null);
    setReleased(null);
    try {
      // The text of the hold is enough: one written by hand may name no source.
      await api.unhold({ backend: "", id: pkg });
      await config.reload();
      setReleased(pkg);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  if (holds.length === 0 && !error && !released) return null;
  return (
    <div className="pt-4 border-t border-line">
      <div className="text-sm font-medium flex items-center gap-1.5 mb-1">
        <Lock aria-hidden className="w-3.5 h-3.5 text-muted" />
        Held packages
      </div>
      <p className="text-sm text-muted">
        Never updated until released. Hold a package from the Updates page.
      </p>
      <ul className="mt-2 divide-y divide-line">
        {holds.map((h) => {
          return (
            <li key={h.package} className="flex items-center gap-3 py-2">
              <span className="flex-1 min-w-0">
                <span className="block text-sm font-mono break-all selectable">
                  {h.package}
                  {h.pin ? ` @ ${h.pin}` : ""}
                </span>
                {h.note && <span className="block text-xs text-muted">{h.note}</span>}
              </span>
              <Button
                size="sm"
                variant="ghost"
                busy={busy === h.package}
                disabled={busy !== null}
                aria-label={`Release ${h.package}`}
                icon={<LockOpen className="w-3.5 h-3.5" />}
                onClick={() => void release(h.package)}
              >
                Release
              </Button>
            </li>
          );
        })}
      </ul>
      {error && <ErrorBar message={`Could not release: ${error}`} />}
      {released && (
        <p role="status" className="text-sm text-muted mt-1">
          {released} is offered again after the next scan.
        </p>
      )}
    </div>
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
          description="Registered for your user only. No administrator rights needed."
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
        {autostart.error && (
          <ErrorBar message={`Whether Odysync starts with you could not be read: ${autostart.error}`} />
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
  const id = useId();
  // What is being typed. Clamping it on every keystroke would turn the "1" of
  // "120" into the minimum before the rest can follow.
  const [text, setText] = useState<string | null>(null);
  const commit = () => {
    if (text === null) return;
    const parsed = parseInt(text, 10);
    // An emptied field keeps the value it had.
    onChange(Number.isNaN(parsed) ? value : Math.min(max, Math.max(min, parsed)));
    setText(null);
  };
  return (
    <div>
      <label htmlFor={id} className="block text-sm font-medium mb-1.5">
        {label}
      </label>
      <input
        id={id}
        type="number"
        min={min}
        max={max}
        value={text ?? String(value)}
        aria-describedby={`${id}-hint`}
        onChange={(e) => {
          const next = e.target.value;
          setText(next);
          // A value that is already valid counts at once, so that Save is
          // offered without leaving the field first.
          const parsed = Number(next);
          if (next !== "" && Number.isInteger(parsed) && parsed >= min && parsed <= max) {
            onChange(parsed);
          }
        }}
        onBlur={commit}
        className="w-full h-9 px-3 rounded-lg border border-control bg-canvas text-sm tabular-nums focus:border-accent outline-none"
      />
      <p id={`${id}-hint`} className="text-xs text-faint mt-1">
        {hint}
      </p>
    </div>
  );
}
