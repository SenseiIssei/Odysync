import { useState } from "react";
import { ListChecks, Plus, Trash2, X } from "lucide-react";
import * as api from "../api";
import { useStore } from "../store";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorBar,
  PageHeader,
} from "../components/ui";
import { errorText } from "../format";

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
    <>
      <PageHeader
        title="Profiles"
        subtitle="Named groups of packages, for updating only some things at a time — for example your work tools before a deadline. Use one with `odysync apply --profile <name>`."
        actions={
          <Button
            variant={showCreate ? "ghost" : "primary"}
            onClick={() => setShowCreate((v) => !v)}
            icon={showCreate ? <X className="w-4 h-4" /> : <Plus className="w-4 h-4" />}
          >
            {showCreate ? "Cancel" : "New profile"}
          </Button>
        }
      />

      {error && <ErrorBar message={error} />}
      {profiles.error && <ErrorBar message={profiles.error} />}

      {showCreate && (
        <Card title="New profile">
          <div className="grid gap-4 sm:grid-cols-[260px_1fr]">
            <div>
              <label htmlFor="pname" className="block text-sm font-medium mb-1.5">
                Name
              </label>
              <input
                id="pname"
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                className="w-full h-9 px-3 rounded-lg border border-line bg-canvas text-sm placeholder:text-faint outline-none focus:border-accent"
                placeholder="Work tools"
              />
            </div>
            <div>
              <label htmlFor="ppkgs" className="block text-sm font-medium mb-1.5">
                Packages, one per line
              </label>
              <textarea
                id="ppkgs"
                value={newPackages}
                onChange={(e) => setNewPackages(e.target.value)}
                rows={4}
                className="w-full px-3 py-2 rounded-lg border border-line bg-canvas text-sm font-mono placeholder:text-faint outline-none focus:border-accent selectable"
                placeholder={"vscode\ngit\nnodejs"}
              />
            </div>
          </div>
          <div className="mt-4">
            <Button variant="primary" onClick={() => void create()} busy={busy} disabled={!newName.trim()}>
              Create profile
            </Button>
          </div>
        </Card>
      )}

      {list.length === 0 && !showCreate ? (
        <Card>
          <EmptyState
            icon={<ListChecks />}
            title={
              profiles.loading && profiles.loadedAt === null ? "Loading profiles…" : "No profiles yet"
            }
            hint="Group packages to update them together, separately from everything else."
          />
        </Card>
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          {list.map((p) => (
            <Card
              key={p.name}
              title={p.name}
              description={`${p.packages.length} package${p.packages.length === 1 ? "" : "s"}`}
              actions={
                <Button
                  size="sm"
                  variant={confirmDelete === p.name ? "danger" : "ghost"}
                  onClick={() => void remove(p.name)}
                  disabled={busy}
                  icon={<Trash2 className="w-3.5 h-3.5" />}
                >
                  {confirmDelete === p.name ? "Delete?" : "Delete"}
                </Button>
              }
            >
              {p.packages.length === 0 ? (
                <p className="text-sm text-muted">This profile is empty.</p>
              ) : (
                <div className="flex flex-wrap gap-1.5">
                  {p.packages.map((pkg) => (
                    <Badge key={pkg}>
                      <span className="font-mono">{pkg}</span>
                    </Badge>
                  ))}
                </div>
              )}
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
