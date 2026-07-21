import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { CheckCircle, Lock, ShieldCheck } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Card,
  ErrorBar,
  PageHeader,
  RefreshButton,
  ResourceView,
  Toggle,
  errorText,
} from "../components/ui";
import type { Config } from "../types";

export default function Settings() {
  const { config, backends, sysInfo } = useStore();
  const [draft, setDraft] = useState<Config | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Adopt the loaded config as the edit buffer, but never clobber unsaved edits.
  useEffect(() => {
    if (config.data && draft === null) setDraft(config.data);
  }, [config.data, draft]);

  const patch = (p: Partial<Config>) => {
    setDraft((d) => (d ? { ...d, ...p } : d));
    setSaved(false);
  };

  const patchPolicy = (p: Partial<Config["policy"]>) => {
    setDraft((d) => (d ? { ...d, policy: { ...d.policy, ...p } } : d));
    setSaved(false);
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    setError(null);
    try {
      await api.saveConfig(draft);
      await config.refresh();
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const revert = async () => {
    await config.refresh();
    setDraft(null);
    setSaved(false);
    setError(null);
  };

  const dirty =
    draft !== null &&
    config.data !== null &&
    JSON.stringify(draft) !== JSON.stringify(config.data);

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <PageHeader
        title="Settings"
        subtitle="Safety policy, automation and network behaviour"
        actions={<RefreshButton onClick={() => void revert()} spinning={config.loading} label="Reload" />}
      />

      <ResourceView resource={config} loadingLabel="Loading configuration...">
        {draft && (
          <div className="space-y-4">
            <Card title="Backends">
              {backends.data.length === 0 ? (
                <p className="text-xs text-cyber-text-faint">
                  {backends.loading ? "Detecting backends..." : "No backends detected."}
                </p>
              ) : (
                <div className="space-y-1.5 max-h-64 overflow-y-auto">
                  {backends.data.map((b) => {
                    const disabled = draft.disabled_backends.some(
                      (d) => d.toLowerCase() === b.kind.toLowerCase(),
                    );
                    return (
                      <div key={b.kind} className="flex items-center justify-between gap-3 text-xs">
                        <span className="text-cyber-text-dim truncate" title={b.kind}>
                          {b.name}
                        </span>
                        <div className="flex items-center gap-2 flex-shrink-0">
                          <span
                            className={`px-2 py-0.5 rounded-full text-[10px] font-medium ${
                              b.available
                                ? "bg-success/20 text-success"
                                : "bg-cyber-bg text-cyber-text-faint"
                            }`}
                          >
                            {b.available ? "Available" : "Not installed"}
                          </span>
                          <button
                            type="button"
                            onClick={() =>
                              patch({
                                disabled_backends: disabled
                                  ? draft.disabled_backends.filter(
                                      (d) => d.toLowerCase() !== b.kind.toLowerCase(),
                                    )
                                  : [...draft.disabled_backends, b.kind],
                              })
                            }
                            className={`px-2 py-0.5 rounded text-[10px] border transition-all ${
                              disabled
                                ? "border-danger/30 text-danger hover:bg-danger/10"
                                : "border-cyber-border text-cyber-text-dim hover:border-accent/30 hover:text-accent"
                            }`}
                          >
                            {disabled ? "Disabled" : "Enabled"}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            <Card title="Safety Policy">
              <div className="space-y-4">
                <Toggle
                  label="Stable releases only"
                  description="Never install alpha, beta, rc or nightly builds"
                  checked={draft.policy.stable_only}
                  onChange={(v) => patchPolicy({ stable_only: v })}
                />
                <Toggle
                  label="Require known versions"
                  description="Refuse updates where either version string is unparseable"
                  checked={draft.policy.require_known_versions}
                  onChange={(v) => patchPolicy({ require_known_versions: v })}
                />
                <Toggle
                  label="Create a restore point before applying"
                  description="Windows only — takes a system restore point before each run"
                  checked={draft.restore_point}
                  onChange={(v) => patch({ restore_point: v })}
                />

                <div className="flex items-center justify-between gap-4 pt-2 border-t border-cyber-border">
                  <div>
                    <div className="text-sm font-medium flex items-center gap-1.5">
                      <ShieldCheck className="w-3.5 h-3.5 text-cyber-text-dim" />
                      Elevation
                    </div>
                    <div className="text-xs text-cyber-text-dim">
                      Detected from the running process — not a setting
                    </div>
                  </div>
                  <span
                    className={`px-2 py-0.5 rounded-full text-[10px] font-medium flex-shrink-0 ${
                      sysInfo.data?.elevated
                        ? "bg-warning/20 text-warning"
                        : "bg-cyber-bg text-cyber-text-faint"
                    }`}
                  >
                    {sysInfo.data?.elevated ? "Elevated" : "Standard user"}
                  </span>
                </div>

                {draft.policy.holds.length > 0 && (
                  <div className="pt-2 border-t border-cyber-border">
                    <div className="text-sm font-medium flex items-center gap-1.5 mb-2">
                      <Lock className="w-3.5 h-3.5 text-cyber-text-dim" />
                      Held packages ({draft.policy.holds.length})
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {draft.policy.holds.map((h) => (
                        <span
                          key={h.package}
                          title={h.note ?? undefined}
                          className="text-[10px] px-2 py-0.5 rounded-full bg-cyber-bg border border-cyber-border text-cyber-text-dim font-mono"
                        >
                          {h.package}
                          {h.pin ? ` @ ${h.pin}` : ""}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </Card>

            <StartupCard />

            <Card title="Automation">
              <div className="space-y-4">
                <Toggle
                  label="Auto-apply updates"
                  description="Apply everything the policy allows straight after a scan"
                  checked={draft.auto_apply}
                  onChange={(v) => patch({ auto_apply: v })}
                />
                <Toggle
                  label="Desktop notifications"
                  description="Notify on scan results and when an apply finishes"
                  checked={draft.notifications}
                  onChange={(v) => patch({ notifications: v })}
                />

                <NumberField
                  label="Scan interval (hours)"
                  hint="0 = manual only"
                  min={0}
                  max={168}
                  value={draft.scan_interval_hours}
                  onChange={(v) => patch({ scan_interval_hours: v })}
                />
                <NumberField
                  label="Max retries"
                  hint="Retry a failed update up to N times"
                  min={0}
                  max={10}
                  value={draft.max_retries}
                  onChange={(v) => patch({ max_retries: v })}
                />
                <NumberField
                  label="Backend timeout (seconds)"
                  hint="Time limit for a single backend operation"
                  min={10}
                  max={3600}
                  value={draft.backend_timeout_secs}
                  onChange={(v) => patch({ backend_timeout_secs: v })}
                />
              </div>
            </Card>

            <Card title="Network">
              <div className="space-y-4">
                <div>
                  <label htmlFor="proxy" className="block text-sm font-medium mb-1">
                    HTTP proxy URL
                  </label>
                  <input
                    id="proxy"
                    type="text"
                    value={draft.proxy_url ?? ""}
                    onChange={(e) => patch({ proxy_url: e.target.value.trim() || null })}
                    className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
                    placeholder="http://proxy.example.com:8080"
                  />
                  <p className="text-xs text-cyber-text-dim mt-1">
                    Used for version lookups and installer downloads. Leave empty for a
                    direct connection.
                  </p>
                </div>

                <div>
                  <label htmlFor="conc" className="block text-sm font-medium mb-1">
                    Concurrency
                  </label>
                  <div className="flex items-center gap-3">
                    <input
                      id="conc"
                      type="range"
                      min={1}
                      max={16}
                      value={draft.concurrency}
                      onChange={(e) => patch({ concurrency: Number(e.target.value) })}
                      className="flex-1 accent-cyan-400"
                    />
                    <span className="text-sm font-mono w-8 text-center text-accent">
                      {draft.concurrency}
                    </span>
                  </div>
                  <p className="text-xs text-cyber-text-dim mt-1">
                    Maximum backends scanned or applied at the same time.
                  </p>
                </div>
              </div>
            </Card>

            <Card title="Excluded packages">
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
                className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm font-mono focus:border-accent"
                placeholder={"One package per line, as backend:id or a bare id"}
              />
            </Card>

            <div className="flex items-center gap-3 sticky bottom-0 py-3 bg-cyber-bg/90 backdrop-blur-sm">
              <motion.button
                type="button"
                whileHover={{ scale: 1.02 }}
                whileTap={{ scale: 0.98 }}
                onClick={save}
                disabled={saving || !dirty}
                className="px-5 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all glow-cyan"
              >
                {saving ? "Saving..." : "Save Settings"}
              </motion.button>
              {dirty && (
                <button
                  type="button"
                  onClick={() => void revert()}
                  className="px-4 py-2 rounded-lg border border-cyber-border text-sm hover:bg-cyber-surface-2 transition-all"
                >
                  Discard changes
                </button>
              )}
              {saved && !dirty && (
                <motion.span
                  initial={{ opacity: 0, x: -5 }}
                  animate={{ opacity: 1, x: 0 }}
                  className="text-sm text-success flex items-center gap-1"
                >
                  <CheckCircle className="w-4 h-4" />
                  Saved
                </motion.span>
              )}
            </div>

            {error && <ErrorBar message={error} />}
          </div>
        )}
      </ResourceView>
    </div>
  );
}

/**
 * Windows startup registration.
 *
 * Kept out of the config file deliberately: the source of truth is the HKCU Run
 * key, so a toggle here must read back from the registry rather than from
 * whatever the config last claimed.
 */
function StartupCard() {
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
      <div className="space-y-4">
        <Toggle
          label="Start Odysync with Windows"
          description="Registers Odysync for the current user — no administrator rights needed"
          checked={cfg?.enabled ?? false}
          disabled={busy || autostart.loading}
          onChange={(v) => void set(v, cfg?.minimized ?? true)}
        />
        <Toggle
          label="Start minimised to the tray"
          description="Launch hidden in the notification area instead of opening the window"
          checked={cfg?.minimized ?? false}
          disabled={busy || autostart.loading || !(cfg?.enabled ?? false)}
          onChange={(v) => void set(true, v)}
        />
        <Toggle
          label="Run a security audit at startup"
          description="Check for malware, persistence and unauthorised access each time Odysync starts"
          checked={security.scanOnStartup}
          onChange={security.setScanOnStartup}
        />
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
      <label className="block text-sm font-medium mb-1">{label}</label>
      <div className="flex items-center gap-3">
        <input
          type="number"
          min={min}
          max={max}
          value={value}
          onChange={(e) => {
            const parsed = parseInt(e.target.value, 10);
            onChange(Number.isNaN(parsed) ? min : clamp(parsed));
          }}
          className="w-24 px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
        />
        <span className="text-xs text-cyber-text-dim">{hint}</span>
      </div>
    </div>
  );
}
