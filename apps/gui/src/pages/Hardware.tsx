import { motion } from "framer-motion";
import { Cpu, HardDrive, Info, Zap } from "lucide-react";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  PageHeader,
  RefreshButton,
  ResourceView,
} from "../components/ui";

export default function Hardware() {
  const { hardware } = useStore();
  const info = hardware.data;

  return (
    <div className="max-w-4xl mx-auto space-y-6">
      <PageHeader
        title="Hardware Info"
        subtitle="Detected CPU, memory, graphics and storage"
        actions={
          <RefreshButton onClick={() => void hardware.refresh()} spinning={hardware.loading} />
        }
      />

      <ResourceView resource={hardware} loadingLabel="Detecting hardware...">
        {info ? (
          <div className="space-y-6">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              {[
                {
                  label: "CPU",
                  value: info.cpu || "Unknown",
                  sub: `${info.cpu_cores} logical cores`,
                  icon: <Cpu className="w-5 h-5" />,
                },
                {
                  label: "Memory",
                  value:
                    info.total_memory_gb > 0
                      ? `${info.total_memory_gb.toFixed(1)} GB`
                      : "Unknown",
                  sub: "Total RAM",
                  icon: <HardDrive className="w-5 h-5" />,
                },
                {
                  label: "OS",
                  value: info.os,
                  sub: "Operating System",
                  icon: <Info className="w-5 h-5" />,
                },
              ].map((c, i) => (
                <motion.div
                  key={c.label}
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: i * 0.05 }}
                  className="p-4 rounded-xl bg-cyber-surface border border-cyber-border glow-cyan"
                >
                  <div className="text-accent mb-2">{c.icon}</div>
                  <div className="text-sm font-bold truncate" title={c.value}>
                    {c.value}
                  </div>
                  <div className="text-xs text-cyber-text-dim mt-0.5">{c.sub}</div>
                  <div className="text-[10px] text-cyber-text-faint mt-1 uppercase tracking-wider">
                    {c.label}
                  </div>
                </motion.div>
              ))}
            </div>

            {info.gpu.length > 0 && (
              <Card title="Graphics Cards">
                <div className="space-y-2">
                  {info.gpu.map((g, i) => (
                    <div
                      key={`${g.name}-${i}`}
                      className="flex items-center gap-3 p-3 rounded-lg bg-cyber-bg/50 border border-cyber-border text-xs"
                    >
                      <Zap className="w-4 h-4 text-purple-neon flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium text-cyber-text truncate">{g.name}</div>
                        <div className="text-cyber-text-faint truncate">{g.vendor}</div>
                      </div>
                      <span className="text-cyber-text-dim font-mono flex-shrink-0">
                        Driver: {g.driver_version || "unknown"}
                      </span>
                    </div>
                  ))}
                </div>
              </Card>
            )}

            {info.disks.length > 0 && (
              <Card title="Disks">
                <div className="space-y-2">
                  {info.disks.map((d, i) => (
                    <div
                      key={`${d.name}-${i}`}
                      className="flex items-center gap-3 p-3 rounded-lg bg-cyber-bg/50 border border-cyber-border text-xs"
                    >
                      <HardDrive className="w-4 h-4 text-success flex-shrink-0" />
                      <span className="font-medium text-cyber-text flex-1 truncate">
                        {d.name}
                      </span>
                      <span className="text-cyber-text-dim font-mono">
                        {d.size_gb.toFixed(0)} GB
                      </span>
                      <span className="text-cyber-text-faint">{d.filesystem}</span>
                    </div>
                  ))}
                </div>
              </Card>
            )}
          </div>
        ) : (
          <EmptyState
            icon={<Cpu className="w-12 h-12" />}
            title="No hardware information available."
            hint="Hardware detection is currently implemented for Windows only."
          />
        )}
      </ResourceView>
    </div>
  );
}
