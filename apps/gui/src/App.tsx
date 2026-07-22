import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Layers, Minus, Shield, ShieldCheck, X } from "lucide-react";
import "./App.css";
import * as api from "./api";
import { StoreProvider, useStore } from "./store";
import { NAV_GROUPS, NAV_ITEMS, TABS, isTab, type Tab } from "./nav";
import { safeListen } from "./events";
import { RefreshButton } from "./components/ui";
import { ErrorBoundary } from "./components/ErrorBoundary";

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

export default function App() {
  return (
    <ErrorBoundary label="Odysync">
      <StoreProvider>
        <Shell />
      </StoreProvider>
    </ErrorBoundary>
  );
}

function Shell() {
  const store = useStore();
  const [tab, setTab] = useState<Tab>(() => {
    const saved = localStorage.getItem(TAB_STORAGE_KEY);
    return isTab(saved) ? saved : "dashboard";
  });
  const [sidebarOpen, setSidebarOpen] = useState(true);

  // Resolved lazily and defensively: `getCurrentWindow` throws outside a Tauri
  // webview, and a throw here used to blank the entire window.
  const minimize = useCallback(() => {
    try {
      void getCurrentWindow().minimize();
    } catch (e) {
      console.error("[odysync] minimize failed:", e);
    }
  }, []);

  useEffect(() => {
    localStorage.setItem(TAB_STORAGE_KEY, tab);
  }, [tab]);

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

  const elevated = store.sysInfo.data?.elevated ?? false;

  return (
    <div className="app-window h-screen flex flex-col text-cyber-text bg-cyber-bg grid-bg">
      {/* Titlebar. `data-tauri-drag-region` is what makes a frameless Tauri v2
          window draggable — the CSS `-webkit-app-region` this used before is an
          Electron feature and does nothing in WebView2. */}
      <div
        data-tauri-drag-region
        className="titlebar flex items-center justify-between px-4 py-2.5 border-b border-cyber-border bg-cyber-surface/80 backdrop-blur-sm flex-shrink-0"
      >
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() => setSidebarOpen((v) => !v)}
            className="text-cyber-text-dim hover:text-accent transition-colors p-1"
            title="Toggle sidebar"
            aria-label="Toggle sidebar"
          >
            <motion.div animate={{ rotate: sidebarOpen ? 0 : 180 }}>
              <Layers className="w-4 h-4" />
            </motion.div>
          </button>
          <div className="flex items-center gap-2">
            <Shield className="w-5 h-5 text-accent text-glow-cyan" />
            <span className="text-sm font-bold tracking-wide">ODYSYNC</span>
            <span className="text-xs text-cyber-text-faint">
              v{store.sysInfo.data?.version ?? "2.0"}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <RefreshButton
            onClick={() => void store.refreshAll()}
            spinning={store.booting}
            label="Refresh all"
          />
          {!elevated && (
            <motion.button
              type="button"
              whileHover={{ scale: 1.05 }}
              whileTap={{ scale: 0.95 }}
              onClick={() => void api.restartAsAdmin().catch(() => {})}
              className="flex items-center gap-1.5 px-3 py-1 rounded-lg bg-accent/10 border border-accent/30 text-accent text-xs font-medium hover:bg-accent/20 transition-all glow-cyan"
              title="Restart Odysync with administrator privileges"
            >
              <ShieldCheck className="w-3.5 h-3.5" />
              Run as Admin
            </motion.button>
          )}
          <button
            type="button"
            onClick={minimize}
            className="p-1.5 rounded hover:bg-cyber-surface-2 text-cyber-text-dim hover:text-accent transition-all"
            title="Minimize"
            aria-label="Minimize"
          >
            <Minus className="w-4 h-4" />
          </button>
          <button
            type="button"
            onClick={() => void api.quitApp()}
            className="p-1.5 rounded hover:bg-danger/20 text-cyber-text-dim hover:text-danger transition-all"
            title="Quit Odysync"
            aria-label="Quit"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Startup progress */}
      <AnimatePresence>
        {store.booting && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 2 }}
            exit={{ opacity: 0, height: 0 }}
            className="bg-cyber-surface flex-shrink-0 overflow-hidden"
          >
            <motion.div
              className="h-full bg-accent glow-cyan"
              animate={{ width: `${Math.round(store.bootProgress * 100)}%` }}
              transition={{ duration: 0.4 }}
            />
          </motion.div>
        )}
      </AnimatePresence>

      {/* A config file we could not parse is worth interrupting for: the app is
          running on defaults, and the user's holds are in the .corrupt backup. */}
      {store.sysInfo.data?.config_error && (
        <div className="flex items-start gap-2 mx-6 mt-4 text-xs text-warning bg-warning/5 rounded-lg px-4 py-3 border border-warning/30 flex-shrink-0">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
          <span className="break-words">{store.sysInfo.data.config_error}</span>
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <AnimatePresence initial={false}>
          {sidebarOpen && (
            <motion.nav
              initial={{ width: 0, opacity: 0 }}
              animate={{ width: 200, opacity: 1 }}
              exit={{ width: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: "easeOut" }}
              className="border-r border-cyber-border bg-cyber-surface/50 overflow-hidden flex-shrink-0"
            >
              <div className="w-[200px] py-3 overflow-y-auto h-full">
                {NAV_GROUPS.map((group) => (
                  <div key={group} className="mb-3">
                    <div className="px-4 mb-1.5 text-[10px] uppercase tracking-widest text-cyber-text-faint font-bold">
                      {group}
                    </div>
                    {NAV_ITEMS.filter((n) => n.group === group).map((item) => (
                      <button
                        type="button"
                        key={item.id}
                        onClick={() => navigate(item.id)}
                        aria-current={tab === item.id ? "page" : undefined}
                        className={`relative w-full flex items-center gap-3 px-4 py-2 text-sm whitespace-nowrap transition-all ${
                          tab === item.id
                            ? "text-accent text-glow-cyan"
                            : "text-cyber-text-dim hover:text-cyber-text hover:bg-cyber-surface-2"
                        }`}
                      >
                        {tab === item.id && (
                          <motion.div
                            layoutId="sidebar-active"
                            className="absolute left-0 top-0 bottom-0 w-[2px] bg-accent glow-cyan"
                          />
                        )}
                        {item.icon}
                        {item.label}
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </motion.nav>
          )}
        </AnimatePresence>

        {/*
          Every page stays mounted for the lifetime of the app. Only the active
          one is displayed. This is what makes navigation instant and keeps each
          page's scroll position, filters and in-flight work intact — swapping
          the subtree on every tab change was what made the app feel broken.
        */}
        <main className="flex-1 relative">
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

/**
 * A permanently mounted page. Inactive pages are hidden rather than unmounted,
 * and are made inert so their controls stay out of the tab order.
 */
function Page({ active, children }: { active: boolean; children: React.ReactNode }) {
  return (
    <div
      hidden={!active}
      aria-hidden={!active}
      className={`absolute inset-0 overflow-y-auto p-6 ${active ? "" : "pointer-events-none"}`}
      style={{ display: active ? "block" : "none" }}
    >
      {children}
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
