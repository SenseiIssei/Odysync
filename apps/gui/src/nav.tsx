import {
  Brush,
  CalendarClock,
  Cpu,
  HardDriveDownload,
  History,
  Info,
  LayoutDashboard,
  Layers,
  ListChecks,
  MemoryStick,
  Package,
  Power,
  RotateCcw,
  ScrollText,
  Settings,
  ShieldCheck,
} from "lucide-react";
import type { ReactNode } from "react";

/**
 * Tab ids are persisted in localStorage, so they stay stable even when a
 * page's label changes: `maintenance` is the page now called "Cleanup".
 */
export const TABS = [
  "dashboard",
  "updates",
  "drivers",
  "maintenance",
  "startup",
  "security",
  "backup",
  "hardware",
  "packages",
  "history",
  "schedule",
  "profiles",
  "offline",
  "settings",
  "logs",
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
  /** Empty for the items pinned to the bottom of the sidebar. */
  group: string;
}

const icon = (C: typeof Cpu) => <C className="w-[17px] h-[17px]" strokeWidth={1.75} />;

export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Overview", icon: icon(LayoutDashboard), group: "" },
  { id: "updates", label: "Updates", icon: icon(Package), group: "Keep current" },
  { id: "drivers", label: "Drivers", icon: icon(MemoryStick), group: "Keep current" },
  { id: "maintenance", label: "Cleanup", icon: icon(Brush), group: "Care" },
  { id: "startup", label: "Startup", icon: icon(Power), group: "Care" },
  { id: "security", label: "Security", icon: icon(ShieldCheck), group: "Care" },
  { id: "backup", label: "Restore points", icon: icon(RotateCcw), group: "Care" },
  { id: "hardware", label: "System info", icon: icon(Cpu), group: "System" },
  { id: "packages", label: "Installed", icon: icon(Layers), group: "System" },
  { id: "history", label: "History", icon: icon(History), group: "System" },
  { id: "schedule", label: "Schedule", icon: icon(CalendarClock), group: "Automation" },
  { id: "profiles", label: "Profiles", icon: icon(ListChecks), group: "Automation" },
  { id: "offline", label: "Offline cache", icon: icon(HardDriveDownload), group: "Automation" },
  { id: "settings", label: "Settings", icon: icon(Settings), group: "footer" },
  { id: "logs", label: "Logs", icon: icon(ScrollText), group: "footer" },
  { id: "about", label: "About", icon: icon(Info), group: "footer" },
];

export const NAV_GROUPS = [...new Set(NAV_ITEMS.map((n) => n.group))].filter(
  (g) => g !== "footer",
);
