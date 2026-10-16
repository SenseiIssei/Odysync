import { Cpu, HardDrive, MemoryStick, Monitor, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useStore } from "../store";
import {
  Button,
  Card,
  EmptyState,
  Meter,
  PageHeader,
  ResourceView,
} from "../components/ui";
import { formatSize } from "../format";

export default function Hardware() {
  const { hardware } = useStore();
  const info = hardware.data;

  return (
    <>
      <PageHeader
        title="System info"
        subtitle="What this machine is made of, read from the operating system's own sources."
        actions={
          <Button
            onClick={() => void hardware.refresh()}
            busy={hardware.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

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
    <div className="rounded-xl border border-line bg-surface p-5">
      <div className="flex items-center gap-2 text-sm text-muted [&>svg]:w-4 [&>svg]:h-4">
        {icon}
        {label}
      </div>
      <div className="mt-2 text-[15px] font-semibold text-fg leading-snug selectable">{value}</div>
      {hint && <div className="text-sm text-muted mt-0.5 selectable">{hint}</div>}
    </div>
  );
}
