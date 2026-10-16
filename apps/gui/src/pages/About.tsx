import { useStore } from "../store";
import { Card, Facts } from "../components/ui";
import { Logo } from "../components/Logo";

const HIGHLIGHTS = [
  "Updates installed programs straight from their publishers — winget is optional",
  "Every installer Odysync downloads must carry its publisher's signature",
  "NVIDIA drivers from NVIDIA's own service, WHQL only",
  "Mac apps through their own Sparkle feeds, checked against the app's own key",
  "Cleanup that measures first and asks before anything permanent",
  "Runtimes, device problems and driver backups in one place",
  "A calmer interface with light and dark themes",
];

const PRINCIPLES = [
  ["Never downgrade", "An update must be newer by real version comparison, not by string order."],
  ["Never guess", "An unreadable version is a reason to skip, never a reason to install."],
  ["Never reinstall", "A failed update stays failed; nothing is installed over a working copy."],
  ["Name who checked", "Every source says who verified what it installs."],
];

export default function About() {
  const { sysInfo } = useStore();
  const info = sysInfo.data;

  return (
    <>
      <section className="rounded-xl border border-line bg-surface p-8 flex flex-col items-center text-center">
        <Logo className="w-16 h-16" />
        <h1 className="text-2xl font-semibold tracking-tight mt-4">Odysync</h1>
        <p className="text-muted mt-1">Safe software and driver updates for Windows, macOS and Linux.</p>
        <p className="text-sm text-faint mt-1">Version {info?.version ?? "…"} · free and open source (MIT)</p>
      </section>

      <div className="grid gap-4 md:grid-cols-2">
        <Card title="New in this version">
          <ul className="space-y-2 text-sm text-muted">
            {HIGHLIGHTS.map((line) => (
              <li key={line} className="flex gap-2.5">
                <span className="mt-2 w-1.5 h-1.5 rounded-full bg-accent flex-shrink-0" />
                {line}
              </li>
            ))}
          </ul>
        </Card>
        <Card title="Rules it never breaks">
          <ul className="space-y-3">
            {PRINCIPLES.map(([title, text]) => (
              <li key={title}>
                <div className="text-sm font-medium">{title}</div>
                <div className="text-sm text-muted">{text}</div>
              </li>
            ))}
          </ul>
        </Card>
      </div>

      {info && (
        <Card title="This installation">
          <Facts
            items={[
              ["System", info.os],
              ["Odysync", info.version],
              ["Running as", info.elevated ? "Administrator" : "Standard user"],
              ["Source code", "github.com/SenseiIssei/Odysync"],
            ]}
          />
        </Card>
      )}
    </>
  );
}
