import { Shield } from "lucide-react";
import { useStore } from "../store";

const CHANGELOG = [
  "Security page: malware, persistence, network and hardening audit",
  "Hardware Updates page for drivers, firmware and vendor tools",
  "Start with Windows, optionally minimised to the tray",
  "Log viewer auto-scrolls and follows new entries",
  "All pages load in the background at startup and keep their state",
  "Settings now round-trip correctly and no longer reset holds or exclusions",
  "Update history is recorded and persisted after every apply",
  "Startup program enable/disable works in both directions",
  "Backend detection is cached instead of re-probed on every page",
];

export default function About() {
  const { sysInfo } = useStore();

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <div className="text-center py-8">
        <Shield className="w-16 h-16 mx-auto mb-4 text-accent text-glow-cyan" />
        <h2 className="text-2xl font-bold text-glow-cyan">Odysync</h2>
        <p className="text-sm text-cyber-text-dim mt-1">
          v{sysInfo.data?.version ?? "…"}
        </p>
        <p className="text-xs text-cyber-text-faint mt-2">
          Safe, verified software and driver updates
        </p>
      </div>

      <div className="gradient-border p-4 space-y-2">
        <h3 className="text-sm font-bold text-accent">What&apos;s new</h3>
        <div className="text-xs text-cyber-text-dim space-y-1 font-mono">
          {CHANGELOG.map((line) => (
            <div key={line}>
              <span className="text-success">+</span> {line}
            </div>
          ))}
        </div>
      </div>

      {sysInfo.data && (
        <div className="rounded-lg border border-cyber-border bg-cyber-surface p-4 text-xs space-y-1.5">
          <Row label="Operating system" value={sysInfo.data.os} />
          <Row label="Version" value={sysInfo.data.version} />
          <Row
            label="Privileges"
            value={sysInfo.data.elevated ? "Elevated" : "Standard user"}
          />
        </div>
      )}

      <div className="text-center text-xs text-cyber-text-faint">
        <p>Built with Tauri, React and Rust.</p>
        <p className="mt-1 select-text">github.com/SenseiIssei/Odysync</p>
      </div>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-cyber-text-dim">{label}</span>
      <span className="text-cyber-text font-mono truncate">{value}</span>
    </div>
  );
}
