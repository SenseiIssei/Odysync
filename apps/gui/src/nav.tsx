import {
  Activity,
  Calendar,
  Cpu,
  DatabaseBackup,
  History,
  Info,
  Layers,
  MemoryStick,
  Package,
  Power,
  ScrollText,
  Settings,
  ShieldAlert,
  WifiOff,
  Wrench,
} from "lucide-react";
import type { ReactNode } from "react";

export const TABS = [
  "dashboard",
  "updates",
  "drivers",
  "history",
  "packages",
  "security",
  "hardware",
  "maintenance",
  "logs",
  "schedule",
  "profiles",
  "offline",
  "startup",
  "backup",
  "settings",
  "about",
] as const;

export type Tab = (typeof TABS)[number];

export function isTab(value: unknown): value is Tab {
  return typeof value === "string" && (TABS as readonly string[]).includes(value);
}

export interface NavItem {
  id: Tab;
  label: string;
  icon: ReactNode;
  group: string;
}

export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Dashboard", icon: <Activity className="w-4 h-4" />, group: "Overview" },
  { id: "updates", label: "Updates", icon: <Package className="w-4 h-4" />, group: "Overview" },
  // Label kept to a single word so the sidebar row never wraps; the page
  // itself is still titled "Hardware Updates".
  { id: "drivers", label: "Drivers", icon: <MemoryStick className="w-4 h-4" />, group: "Overview" },
  { id: "history", label: "History", icon: <History className="w-4 h-4" />, group: "Overview" },
  { id: "packages", label: "Packages", icon: <Layers className="w-4 h-4" />, group: "Overview" },
  { id: "security", label: "Security", icon: <ShieldAlert className="w-4 h-4" />, group: "System" },
  { id: "hardware", label: "Hardware", icon: <Cpu className="w-4 h-4" />, group: "System" },
  { id: "maintenance", label: "Maintenance", icon: <Wrench className="w-4 h-4" />, group: "System" },
  { id: "logs", label: "Logs", icon: <ScrollText className="w-4 h-4" />, group: "System" },
  { id: "schedule", label: "Schedule", icon: <Calendar className="w-4 h-4" />, group: "Automation" },
  { id: "profiles", label: "Profiles", icon: <Layers className="w-4 h-4" />, group: "Automation" },
  { id: "offline", label: "Offline", icon: <WifiOff className="w-4 h-4" />, group: "Automation" },
  { id: "startup", label: "Startup", icon: <Power className="w-4 h-4" />, group: "Automation" },
  { id: "backup", label: "Backup", icon: <DatabaseBackup className="w-4 h-4" />, group: "Automation" },
  { id: "settings", label: "Settings", icon: <Settings className="w-4 h-4" />, group: "Config" },
  { id: "about", label: "About", icon: <Info className="w-4 h-4" />, group: "Config" },
];

export const NAV_GROUPS = [...new Set(NAV_ITEMS.map((n) => n.group))];
