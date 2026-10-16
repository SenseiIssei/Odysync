import { useMemo, useState } from "react";
import { Layers, RefreshCw } from "lucide-react";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  SearchInput,
} from "../components/ui";

export default function Packages() {
  const { packages, backends } = useStore();
  const [search, setSearch] = useState("");
  const [source, setSource] = useState("");

  const all = packages.data;
  const sources = useMemo(() => [...new Set(all.map((p) => p.backend))].sort(), [all]);
  const nameOf = (kind: string) => backends.data.find((b) => b.kind === kind)?.name ?? kind;

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return all.filter(
      (p) =>
        (needle === "" ||
          p.name.toLowerCase().includes(needle) ||
          p.id.toLowerCase().includes(needle)) &&
        (source === "" || p.backend === source),
    );
  }, [all, search, source]);

  return (
    <>
      <PageHeader
        title="Installed"
        subtitle="Everything the update sources know about on this machine."
        actions={
          <Button
            onClick={() => void packages.refresh()}
            busy={packages.loading}
            icon={<RefreshCw className="w-4 h-4" />}
          >
            Refresh
          </Button>
        }
      />

      {packages.error && <ErrorBar message={packages.error} />}

      <Card padded={false}>
        <div className="flex flex-wrap items-center gap-3 px-5 py-3 border-b border-line">
          <SearchInput value={search} onChange={setSearch} placeholder="Filter by name or id" />
          <select
            value={source}
            onChange={(e) => setSource(e.target.value)}
            aria-label="Source"
            className="h-9 px-3 rounded-lg border border-line bg-canvas text-sm outline-none focus:border-accent"
          >
            <option value="">All sources</option>
            {sources.map((b) => (
              <option key={b} value={b}>
                {nameOf(b)}
              </option>
            ))}
          </select>
          <span className="text-sm text-faint tabular-nums">
            {filtered.length} of {all.length}
          </span>
        </div>

        {filtered.length === 0 ? (
          <EmptyState
            icon={<Layers />}
            title={
              packages.loading && packages.loadedAt === null
                ? "Asking every source what is installed…"
                : all.length === 0
                  ? "Nothing found"
                  : "No package matches the filter"
            }
          />
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-faint">
                <th className="py-2.5 pl-5 font-medium">Name</th>
                <th className="py-2.5 font-medium">Source</th>
                <th className="py-2.5 pr-5 font-medium text-right">Version</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((p, i) => (
                <tr key={`${p.backend}-${p.id}-${i}`} className="border-t border-line">
                  <td className="py-2.5 pl-5 pr-4">
                    <div className="font-medium text-fg">{p.name}</div>
                    {p.id !== p.name && (
                      <div className="text-xs text-faint font-mono selectable">{p.id}</div>
                    )}
                  </td>
                  <td className="py-2.5 pr-4">
                    <Badge>{nameOf(p.backend)}</Badge>
                  </td>
                  <td className="py-2.5 pr-5 text-right font-mono text-[13px] text-muted selectable">
                    {p.version}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </>
  );
}
