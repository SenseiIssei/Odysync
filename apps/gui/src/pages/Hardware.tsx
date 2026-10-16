import { BatteryMedium, Cpu, HardDrive, HeartPulse, MemoryStick, Monitor, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Meter,
  PageHeader,
  ResourceView,
} from "../components/ui";
import { formatSize } from "../format";
import type { HealthReportDto, HealthState } from "../types";

const STATE_LABEL: Record<HealthState, { text: string; tone: "success" | "warning" | "danger" | "neutral" }> = {
  good: { text: "Healthy", tone: "success" },
  warning: { text: "Needs attention", tone: "warning" },
  bad: { text: "Failing", tone: "danger" },
  unknown: { text: "Not reported", tone: "neutral" },
};

export default function Hardware() {
  const { hardware, health } = useStore();
  const info = hardware.data;

  return (
    <>
      <PageHeader
        title="System & health"
        subtitle="What this machine is made of, and how its disks and battery are holding up — read from the hardware itself."
        actions={
          <Button
            onClick={() => {
              void hardware.refresh();
              void health.refresh();
            }}
            busy={hardware.loading || health.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

      <ResourceView resource={health} loadingLabel="Asking the disks how they are…">
        {health.data && <HealthCard report={health.data} />}
      </ResourceView>

      <ResourceView resource={hardware} loadingLabel="Reading system information…">
        {info ? (
          <>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <Fact icon={<Monitor />} label="System" value={info.os} hint={info.machine ?? undefined} />
              <Fact icon={<Cpu />} label="Processor" value={info.cpu} hint={`${info.cpu_threads} threads`} />
              <Fact
                icon={<MemoryStick />}
                label="Memory"
                value={info.memory_bytes > 0 ? formatSize(info.memory_bytes) : "Unknown"}
              />
            </div>

            {info.gpus.length > 0 && (
              <Card title="Graphics" padded={false}>
                <ul className="divide-y divide-line border-t border-line">
                  {info.gpus.map((g, i) => (
                    <li key={`${g.name}-${i}`} className="flex items-center gap-4 px-5 py-3">
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium text-fg truncate">{g.name}</div>
                        <div className="text-sm text-muted truncate">{g.vendor}</div>
                      </div>
                      {g.driver_version && (
                        <span className="text-sm text-muted font-mono selectable">
                          driver {g.driver_version}
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              </Card>
            )}

            {info.disks.length > 0 && (
              <Card title="Storage" padded={false}>
                <ul className="divide-y divide-line border-t border-line">
                  {info.disks.map((d) => {
                    const used = d.size_bytes > 0 ? 1 - d.free_bytes / d.size_bytes : 0;
                    return (
                      <li key={d.mount} className="px-5 py-3.5">
                        <div className="flex items-center gap-3 mb-2">
                          <HardDrive className="w-4 h-4 text-faint" />
                          <span className="text-sm font-medium">
                            {d.mount}
                            {d.label && <span className="text-muted font-normal"> · {d.label}</span>}
                          </span>
                          <span className="text-xs text-faint">{d.filesystem}</span>
                          <span className="ml-auto text-sm text-muted tabular-nums">
                            {formatSize(d.free_bytes)} free of {formatSize(d.size_bytes)}
                          </span>
                        </div>
                        <Meter value={used} tone={used > 0.9 ? "danger" : used > 0.8 ? "warning" : "accent"} />
                      </li>
                    );
                  })}
                </ul>
              </Card>
            )}
          </>
        ) : (
          <Card>
            <EmptyState icon={<Cpu />} title="No system information yet" />
          </Card>
        )}
      </ResourceView>
    </>
  );
}

function Fact({
  icon,
  label,
  value,
  hint,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="rounded-2xl border border-line glass p-5">
      <div className="flex items-center gap-2 text-sm text-muted [&>svg]:w-4 [&>svg]:h-4">
        {icon}
        {label}
      </div>
      <div className="mt-2 text-[15px] font-semibold text-fg leading-snug selectable">{value}</div>
      {hint && <div className="text-sm text-muted mt-0.5 selectable">{hint}</div>}
    </div>
  );
}

function HealthCard({ report }: { report: HealthReportDto }) {
  return (
    <Card
      title={
        <span className="flex items-center gap-2">
          <HeartPulse className="w-4 h-4 text-accent" /> Hardware health
        </span>
      }
      description="Disks and batteries wear out on their own. Odysync reads what they report about themselves — nothing is estimated."
      padded={false}
    >
      <ul className="divide-y divide-line border-t border-line">
        {report.disks.map((d, i) => {
          const state = STATE_LABEL[d.state];
          const facts = [
            d.kind,
            d.size_bytes > 0 ? formatSize(d.size_bytes) : null,
            d.temperature_c !== null ? `${d.temperature_c} °C` : null,
            d.power_on_hours !== null ? `${d.power_on_hours.toLocaleString()} h on` : null,
          ].filter(Boolean);
          return (
            <li key={`${d.name}-${i}`} className="px-5 py-3.5">
              <div className="flex items-center gap-3">
                <HardDrive className="w-4 h-4 text-faint flex-shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-medium text-fg truncate">{d.name}</div>
                  <div className="text-sm text-muted truncate">{facts.join(" · ")}</div>
                </div>
                <Badge tone={state.tone}>{state.text}</Badge>
              </div>
              {d.wear_percent !== null && (
                <div className="mt-2.5 pl-7">
                  <div className="flex justify-between text-xs text-muted mb-1">
                    <span>Write endurance used</span>
                    <span className="tabular-nums">{d.wear_percent}%</span>
                  </div>
                  <Meter
                    value={d.wear_percent / 100}
                    tone={d.wear_percent >= 80 ? "danger" : d.wear_percent >= 50 ? "warning" : "success"}
                  />
                </div>
              )}
              {d.state !== "good" && <p className="text-sm text-muted mt-1.5 pl-7">{d.summary}</p>}
            </li>
          );
        })}
        {report.battery && (
          <li className="px-5 py-3.5">
            <div className="flex items-center gap-3">
              <BatteryMedium className="w-4 h-4 text-faint flex-shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium text-fg">Battery</div>
                <div className="text-sm text-muted">
                  {report.battery.summary}
                  {report.battery.cycle_count !== null && ` · ${report.battery.cycle_count} charge cycles`}
                </div>
              </div>
              <Badge tone={STATE_LABEL[report.battery.state].tone}>
                {STATE_LABEL[report.battery.state].text}
              </Badge>
            </div>
            {report.battery.capacity_percent !== null && (
              <div className="mt-2.5 pl-7">
                <Meter
                  value={report.battery.capacity_percent / 100}
                  tone={
                    report.battery.capacity_percent >= 80
                      ? "success"
                      : report.battery.capacity_percent >= 60
                        ? "warning"
                        : "danger"
                  }
                />
              </div>
            )}
          </li>
        )}
        {report.disks.length === 0 && !report.battery && (
          <li className="px-5 py-4 text-sm text-muted">Nothing could be read on this machine.</li>
        )}
      </ul>
      {report.notes.length > 0 && (
        <div className="px-5 py-3 border-t border-line text-sm text-muted space-y-1">
          {report.notes.map((n) => (
            <p key={n}>{n}</p>
          ))}
        </div>
      )}
    </Card>
  );
}
