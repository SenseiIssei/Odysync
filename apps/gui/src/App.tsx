import { useCallback, useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlertTriangle,
  Minus,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  ShieldCheck,
  Square,
  Sun,
  X,
} from "lucide-react";
import * as api from "./api";
import { StoreProvider, useStore } from "./store";
import { NAV_GROUPS, NAV_ITEMS, TABS, isTab, type NavItem, type Tab } from "./nav";
import { safeListen } from "./events";
import { inTauri } from "./bridge";
import { useTheme } from "./theme";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Logo } from "./components/Logo";

import Dashboard from "./pages/Dashboard";
import Updates from "./pages/Updates";
import Drivers from "./pages/Drivers";
import Security from "./pages/Security";
import History from "./pages/History";
import Packages from "./pages/Packages";
import Hardware from "./pages/Hardware";
import Maintenance from "./pages/Maintenance";
import Logs from "./pages/Logs";
import Schedule from "./pages/Schedule";
import Profiles from "./pages/Profiles";
import Offline from "./pages/Offline";
import Startup from "./pages/Startup";
import BackupPage from "./pages/Backup";
import Settings from "./pages/Settings";
import About from "./pages/About";

const TAB_STORAGE_KEY = "odysync.activeTab";
const SIDEBAR_KEY = "odysync.sidebar";

export default function App() {
  return (
    <ErrorBoundary label="Odysync">
      <StoreProvider>
        <Shell />
      </StoreProvider>
    </ErrorBoundary>
  );
}

/** Read a tab from the URL hash, so the browser demo can deep-link a page. */
function tabFromHash(): Tab | null {
  const hash = typeof window !== "undefined" ? window.location.hash.replace(/^#\/?/, "") : "";
  return isTab(hash) ? hash : null;
}

function Shell() {
  const store = useStore();
  const theme = useTheme();
  const [tab, setTab] = useState<Tab>(() => {
    const fromHash = tabFromHash();
    if (fromHash) return fromHash;
    const saved = localStorage.getItem(TAB_STORAGE_KEY);
    return isTab(saved) ? saved : "dashboard";
  });
  const [sidebarOpen, setSidebarOpen] = useState(
    () => localStorage.getItem(SIDEBAR_KEY) !== "collapsed",
  );

  useEffect(() => {
    localStorage.setItem(TAB_STORAGE_KEY, tab);
    if (!inTauri) window.history.replaceState(null, "", `#${tab}`);
  }, [tab]);

  useEffect(() => {
    localStorage.setItem(SIDEBAR_KEY, sidebarOpen ? "open" : "collapsed");
  }, [sidebarOpen]);

  useEffect(() => {
    const onHash = () => {
      const next = tabFromHash();
      if (next) setTab(next);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  const navigate = useCallback((next: Tab) => setTab(next), []);

  const refreshScan = store.scan.refresh;
  useEffect(
    () =>
      safeListen("tray-scan", () => {
        setTab("updates");
        void refreshScan();
      }),
    [refreshScan],
  );

  // Ctrl+R refreshes the data rather than reloading the webview — a reload in a
  // Tauri window discards every page's state and re-runs the whole startup load.
  const refreshAll = store.refreshAll;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r") {
        e.preventDefault();
        void refreshAll();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [refreshAll]);

  const info = store.sysInfo.data;
  const platform = info?.platform ?? (inTauri ? "windows" : "web");
  // Windows draws no frame (decorations are off), so the app draws its own
  // title bar and window buttons. macOS keeps its traffic lights over our
  // header; Linux keeps the window manager's frame.
  const frameless = inTauri && platform === "windows";
  const macOverlay = inTauri && platform === "macos";
  const elevated = info?.elevated ?? false;
  const updateCount = store.scan.data?.actionable.length ?? 0;

  const badges = useMemo<Partial<Record<Tab, number>>>(
    () => ({
      updates: updateCount,
      drivers: store.devices.data.length,
    }),
    [updateCount, store.devices.data.length],
  );

  return (
    <div
      className="app-window h-screen flex flex-col text-fg bg-canvas"
      data-frameless={frameless ? "true" : "false"}
    >
      <TitleBar
        frameless={frameless}
        macOverlay={macOverlay}
        version={info?.version}
        elevated={elevated}
        platform={platform}
        busy={store.booting}
        onRefresh={() => void store.refreshAll()}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        theme={theme.resolved}
        onToggleTheme={() => theme.setChoice(theme.resolved === "dark" ? "light" : "dark")}
      />

      {store.booting && (
        <div className="h-0.5 bg-transparent flex-shrink-0 overflow-hidden">
          <div
            className="h-full bg-accent/70 transition-[width] duration-300"
            style={{ width: `${Math.round(store.bootProgress * 100)}%` }}
          />
        </div>
      )}

      {/* A config file we could not parse is worth interrupting for: the app is
          running on defaults, and the user's holds are in the .corrupt backup. */}
      {info?.config_error && (
        <div className="flex items-start gap-2 mx-6 mt-4 text-sm text-warning bg-warning/10 rounded-lg px-4 py-3 border border-warning/25 flex-shrink-0">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
          <span className="break-words">{info.config_error}</span>
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        <Sidebar open={sidebarOpen} tab={tab} onNavigate={navigate} badges={badges} />

        {/*
          Every page stays mounted for the lifetime of the app. Only the active
          one is displayed. This is what makes navigation instant and keeps each
          page's scroll position, filters and in-flight work intact.
        */}
        <main className="flex-1 relative min-w-0">
          {TABS.map((id) => (
            <Page key={id} active={tab === id}>
              <ErrorBoundary label={NAV_ITEMS.find((n) => n.id === id)?.label ?? id}>
                {renderPage(id, navigate)}
              </ErrorBoundary>
            </Page>
          ))}
        </main>
      </div>
    </div>
  );
}

function TitleBar({
  frameless,
  macOverlay,
  version,
  elevated,
  platform,
  busy,
  onRefresh,
  sidebarOpen,
  onToggleSidebar,
  theme,
  onToggleTheme,
}: {
  frameless: boolean;
  macOverlay: boolean;
  version?: string;
  elevated: boolean;
  platform: string;
  busy: boolean;
  onRefresh: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  theme: "light" | "dark";
  onToggleTheme: () => void;
}) {
  // Resolved lazily and defensively: `getCurrentWindow` throws outside a Tauri
  // webview, and a throw here used to blank the entire window.
  const windowAction = (action: "minimize" | "toggleMaximize") => {
    try {
      void getCurrentWindow()[action]();
    } catch (e) {
      console.error(`[odysync] ${action} failed:`, e);
    }
  };

  return (
    <div
      data-tauri-drag-region={frameless || macOverlay ? true : undefined}
      className={`titlebar h-12 flex items-center justify-between gap-3 border-b border-line bg-surface/60 flex-shrink-0 ${
        macOverlay ? "pl-[84px] pr-3" : "px-3"
      }`}
    >
      <div className="flex items-center gap-2 min-w-0" data-tauri-drag-region={frameless || macOverlay ? true : undefined}>
        <IconButton
          label={sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          onClick={onToggleSidebar}
        >
          {sidebarOpen ? <PanelLeftClose className="w-4 h-4" /> : <PanelLeftOpen className="w-4 h-4" />}
        </IconButton>
        <Logo className="w-6 h-6 flex-shrink-0" />
        <span className="text-sm font-semibold tracking-tight">Odysync</span>
        {version && <span className="text-xs text-faint">v{version}</span>}
      </div>

      <div className="flex items-center gap-1">
        <IconButton label="Refresh everything (Ctrl+R)" onClick={onRefresh} disabled={busy}>
          <RefreshCw className={`w-4 h-4 ${busy ? "animate-spin" : ""}`} />
        </IconButton>
        <IconButton
          label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          onClick={onToggleTheme}
        >
          {theme === "dark" ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
        </IconButton>
        {!elevated && platform === "windows" && (
          <button
            type="button"
            onClick={() => void api.restartAsAdmin().catch(() => {})}
            className="ml-1 inline-flex items-center gap-1.5 h-8 px-3 rounded-lg border border-line text-xs font-medium text-muted hover:text-fg hover:bg-raised transition-colors"
            title="Windows updates, drivers and machine-wide apps need administrator rights"
          >
            <ShieldCheck className="w-3.5 h-3.5" />
            Run as admin
          </button>
        )}
        {frameless && (
          <div className="flex items-center ml-2 pl-2 border-l border-line gap-0.5">
            <IconButton label="Minimize" onClick={() => windowAction("minimize")}>
              <Minus className="w-4 h-4" />
            </IconButton>
            <IconButton label="Maximize" onClick={() => windowAction("toggleMaximize")}>
              <Square className="w-3.5 h-3.5" />
            </IconButton>
            <IconButton label="Close to tray" danger onClick={() => void api.quitApp()}>
              <X className="w-4 h-4" />
            </IconButton>
          </div>
        )}
      </div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  children,
  disabled,
  danger,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  disabled?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className={`h-8 w-8 inline-flex items-center justify-center rounded-lg text-muted transition-colors disabled:opacity-50 ${
        danger ? "hover:bg-danger/15 hover:text-danger" : "hover:bg-raised hover:text-fg"
      }`}
    >
      {children}
    </button>
  );
}

function Sidebar({
  open,
  tab,
  onNavigate,
  badges,
}: {
  open: boolean;
  tab: Tab;
  onNavigate: (t: Tab) => void;
  badges: Partial<Record<Tab, number>>;
}) {
  const item = (n: NavItem) => (
    <NavButton
      key={n.id}
      item={n}
      active={tab === n.id}
      compact={!open}
      badge={badges[n.id]}
      onClick={() => onNavigate(n.id)}
    />
  );

  return (
    <nav
      aria-label="Sections"
      className={`flex flex-col border-r border-line bg-surface/40 flex-shrink-0 transition-[width] duration-200 ${
        open ? "w-[224px]" : "w-[60px]"
      }`}
    >
      <div className="flex-1 overflow-y-auto overflow-x-hidden py-3 px-2.5 space-y-4">
        {NAV_GROUPS.map((group) => (
          <div key={group || "top"}>
            {group && open && (
              <div className="px-2.5 mb-1 text-[11px] font-medium uppercase tracking-[0.08em] text-faint">
                {group}
              </div>
            )}
            {group && !open && <div className="mx-2 mb-2 border-t border-line" />}
            <div className="space-y-0.5">
              {NAV_ITEMS.filter((n) => n.group === group).map(item)}
            </div>
          </div>
        ))}
      </div>
      <div className="border-t border-line py-2 px-2.5 space-y-0.5">
        {NAV_ITEMS.filter((n) => n.group === "footer").map(item)}
      </div>
    </nav>
  );
}

function NavButton({
  item,
  active,
  compact,
  badge,
  onClick,
}: {
  item: NavItem;
  active: boolean;
  compact: boolean;
  badge?: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      title={compact ? item.label : undefined}
      className={`group relative w-full flex items-center gap-3 h-9 rounded-lg text-sm transition-colors ${
        compact ? "justify-center px-0" : "px-2.5"
      } ${
        active
          ? "bg-raised text-fg font-medium"
          : "text-muted hover:text-fg hover:bg-raised/60"
      }`}
    >
      {active && (
        <span className="absolute left-0 top-2 bottom-2 w-[3px] rounded-full bg-accent" aria-hidden />
      )}
      <span className={active ? "text-accent" : "text-faint group-hover:text-muted"}>
        {item.icon}
      </span>
      {!compact && <span className="truncate">{item.label}</span>}
      {!!badge && badge > 0 && (
        <span
          className={`${
            compact ? "absolute top-1 right-1.5 min-w-4 h-4 text-[10px]" : "ml-auto min-w-5 h-5 text-[11px]"
          } px-1.5 inline-flex items-center justify-center rounded-full bg-accent/15 text-accent font-semibold tabular-nums`}
        >
          {badge}
        </span>
      )}
    </button>
  );
}

/**
 * A permanently mounted page. Inactive pages are hidden rather than unmounted,
 * and are made inert so their controls stay out of the tab order.
 */
function Page({ active, children }: { active: boolean; children: React.ReactNode }) {
  return (
    <div
      hidden={!active}
      aria-hidden={!active}
      className={`absolute inset-0 overflow-y-auto ${active ? "" : "pointer-events-none"}`}
      style={{ display: active ? "block" : "none" }}
    >
      <div className="mx-auto max-w-[1180px] px-8 py-7 space-y-6">{children}</div>
    </div>
  );
}

function renderPage(id: Tab, navigate: (tab: Tab) => void) {
  switch (id) {
    case "dashboard":
      return <Dashboard onNavigate={navigate} />;
    case "updates":
      return <Updates />;
    case "drivers":
      return <Drivers />;
    case "security":
      return <Security />;
    case "history":
      return <History />;
    case "packages":
      return <Packages />;
    case "hardware":
      return <Hardware />;
    case "maintenance":
      return <Maintenance />;
    case "logs":
      return <Logs />;
    case "schedule":
      return <Schedule />;
    case "profiles":
      return <Profiles />;
    case "offline":
      return <Offline />;
    case "startup":
      return <Startup />;
    case "backup":
      return <BackupPage />;
    case "settings":
      return <Settings />;
    case "about":
      return <About />;
  }
}
