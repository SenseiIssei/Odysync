import { useState } from "react";
import { motion } from "framer-motion";
import { Layers } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
  RefreshButton,
  errorText,
} from "../components/ui";

export default function Profiles() {
  const { profiles, config } = useStore();
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPackages, setNewPackages] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const list = profiles.data;

  const create = async () => {
    const name = newName.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      const packages = newPackages
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean);
      await api.createProfile(name, packages);
      setNewName("");
      setNewPackages("");
      setShowCreate(false);
      await Promise.allSettled([profiles.refresh(), config.refresh()]);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (name: string) => {
    if (confirmDelete !== name) {
      setConfirmDelete(name);
      return;
    }
    setConfirmDelete(null);
    setBusy(true);
    setError(null);
    try {
      await api.deleteProfile(name);
      await Promise.allSettled([profiles.refresh(), config.refresh()]);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4">
      <PageHeader
        title="Update Profiles"
        subtitle="Named sets of packages, for updating a subset at a time"
        actions={
          <>
            <RefreshButton onClick={() => void profiles.refresh()} spinning={profiles.loading} />
            <motion.button
              type="button"
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={() => setShowCreate((v) => !v)}
              className="px-4 py-1.5 rounded-lg bg-accent/10 border border-accent/30 text-accent text-xs font-medium hover:bg-accent/20 transition-all glow-cyan"
            >
              {showCreate ? "Cancel" : "New Profile"}
            </motion.button>
          </>
        }
      />

      {error && <ErrorBar message={error} />}
      {profiles.error && <ErrorBar message={profiles.error} />}

      {showCreate && (
        <motion.div initial={{ opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }}>
          <Card>
            <div className="space-y-3">
              <div>
                <label htmlFor="pname" className="block text-sm font-medium mb-1">
                  Profile name
                </label>
                <input
                  id="pname"
                  type="text"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm focus:border-accent"
                  placeholder="e.g. Gaming, Development, Minimal"
                />
              </div>
              <div>
                <label htmlFor="ppkgs" className="block text-sm font-medium mb-1">
                  Packages (one per line)
                </label>
                <textarea
                  id="ppkgs"
                  value={newPackages}
                  onChange={(e) => setNewPackages(e.target.value)}
                  rows={4}
                  className="w-full px-3 py-2 rounded-lg border border-cyber-border bg-cyber-bg text-sm font-mono focus:border-accent"
                  placeholder={"Mozilla.Firefox\nGit.Git\nMicrosoft.VisualStudioCode"}
                />
              </div>
              <button
                type="button"
                onClick={create}
                disabled={busy || !newName.trim()}
                className="px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 disabled:opacity-50 transition-all"
              >
                {busy ? "Creating..." : "Create"}
              </button>
            </div>
          </Card>
        </motion.div>
      )}

      {list.length === 0 && !showCreate && (
        <EmptyState
          icon={<Layers className="w-12 h-12" />}
          title={
            profiles.loading && profiles.loadedAt === null
              ? "Loading profiles..."
              : "No profiles configured."
          }
          hint="Create one to group packages for targeted updates."
        />
      )}

      {list.length > 0 && (
        <div className="space-y-2">
          {list.map((p, i) => (
            <motion.div
              key={p.name}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: Math.min(i * 0.05, 0.4) }}
            >
              <Card>
                <div className="flex items-center justify-between mb-2 gap-3">
                  <h3 className="font-bold text-sm text-accent truncate">{p.name}</h3>
                  <button
                    type="button"
                    onClick={() => void remove(p.name)}
                    disabled={busy}
                    className="text-xs text-danger hover:underline disabled:opacity-50 flex-shrink-0"
                  >
                    {confirmDelete === p.name ? "Confirm delete" : "Delete"}
                  </button>
                </div>
                {p.packages.length === 0 ? (
                  <p className="text-xs text-cyber-text-faint">No packages in this profile.</p>
                ) : (
                  <div className="flex flex-wrap gap-1.5">
                    {p.packages.map((pkg) => (
                      <span
                        key={pkg}
                        className="text-[10px] px-2 py-0.5 rounded-full bg-cyber-bg border border-cyber-border text-cyber-text-dim font-mono"
                      >
                        {pkg}
                      </span>
                    ))}
                  </div>
                )}
              </Card>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}
