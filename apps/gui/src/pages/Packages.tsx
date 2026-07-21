import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import { Layers, Package as PackageIcon } from "lucide-react";
import { useStore } from "../store";
import { EmptyState, ErrorBar, PageHeader, RefreshButton } from "../components/ui";

export default function Packages() {
  const { packages } = useStore();
  const [search, setSearch] = useState("");
  const [backendFilter, setBackendFilter] = useState("");

  const all = packages.data;

  const backendNames = useMemo(
    () => [...new Set(all.map((p) => p.backend))].sort(),
    [all],
  );

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return all.filter(
      (p) =>
        (needle === "" ||
          p.name.toLowerCase().includes(needle) ||
          p.id.toLowerCase().includes(needle)) &&
        (backendFilter === "" || p.backend === backendFilter),
    );
  }, [all, search, backendFilter]);

  return (
    <div className="max-w-4xl mx-auto space-y-4">
      <PageHeader
        title="Installed Packages"
        subtitle="Everything the detected backends report as installed"
        actions={
          <RefreshButton onClick={() => void packages.refresh()} spinning={packages.loading} />
        }
      />

      {packages.error && <ErrorBar message={packages.error} />}

      <div className="flex gap-3 flex-wrap">
        <input
          type="text"
          placeholder="Search packages..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="flex-1 min-w-[200px] px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
        />
        <select
          value={backendFilter}
          onChange={(e) => setBackendFilter(e.target.value)}
          className="px-3 py-2 rounded-lg border border-cyber-border bg-cyber-surface text-sm focus:border-accent"
        >
          <option value="">All backends</option>
          {backendNames.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
      </div>

      {all.length === 0 && (
        <EmptyState
          icon={<Layers className="w-12 h-12" />}
          title={
            packages.loading ? "Enumerating installed packages..." : "No packages found."
          }
          hint={
            packages.loading
              ? "This queries every available package manager and can take a while."
              : undefined
          }
        />
      )}

      {all.length > 0 && (
        <>
          <div className="text-xs text-cyber-text-dim">
            Showing {filtered.length} of {all.length} packages
            {packages.loading && " (refreshing...)"}
          </div>
          <div className="space-y-1">
            {filtered.map((p, i) => (
              <motion.div
                key={`${p.backend}-${p.id}-${i}`}
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ delay: Math.min(i * 0.005, 0.3) }}
                className="flex items-center gap-3 p-3 rounded-lg bg-cyber-surface border border-cyber-border text-xs hover:border-cyber-border-bright transition-all"
              >
                <PackageIcon className="w-4 h-4 text-accent flex-shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="font-medium text-cyber-text truncate">{p.name}</div>
                  {p.id !== p.name && (
                    <div className="text-cyber-text-faint font-mono truncate">{p.id}</div>
                  )}
                </div>
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyber-bg text-cyber-text-dim font-mono flex-shrink-0">
                  {p.backend}
                </span>
                <span className="text-cyber-text-dim font-mono flex-shrink-0">
                  {p.version}
                </span>
              </motion.div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
