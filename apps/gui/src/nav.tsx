import {
  Brush,
  CalendarClock,
  Cpu,
  HardDriveDownload,
  History,
  House,
  Info,
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
  /**
   * "" for Home, "Care" for the everyday pages, `MORE_TOOLS` for the ones most
   * people never need (collapsed by default), "footer" for the bottom.
   */
  group: string;
}

/** The collapsible group of less-used pages. */
export const MORE_TOOLS = "More tools";

const icon = (C: typeof Cpu) => <C className="w-[17px] h-[17px]" strokeWidth={1.75} />;

export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Home", icon: icon(House), group: "" },
  { id: "updates", label: "Updates", icon: icon(Package), group: "Care" },
  { id: "drivers", label: "Drivers", icon: icon(MemoryStick), group: "Care" },
  { id: "maintenance", label: "Cleanup", icon: icon(Brush), group: "Care" },
  { id: "security", label: "Security", icon: icon(ShieldCheck), group: "Care" },
  { id: "hardware", label: "System & health", icon: icon(Cpu), group: MORE_TOOLS },
  { id: "startup", label: "Startup", icon: icon(Power), group: MORE_TOOLS },
  { id: "backup", label: "Restore points", icon: icon(RotateCcw), group: MORE_TOOLS },
  { id: "packages", label: "Installed", icon: icon(Layers), group: MORE_TOOLS },
  { id: "history", label: "History", icon: icon(History), group: MORE_TOOLS },
  { id: "schedule", label: "Schedule", icon: icon(CalendarClock), group: MORE_TOOLS },
  { id: "profiles", label: "Profiles", icon: icon(ListChecks), group: MORE_TOOLS },
  { id: "offline", label: "Offline cache", icon: icon(HardDriveDownload), group: MORE_TOOLS },
  { id: "logs", label: "Logs", icon: icon(ScrollText), group: MORE_TOOLS },
  { id: "settings", label: "Settings", icon: icon(Settings), group: "footer" },
  { id: "about", label: "About", icon: icon(Info), group: "footer" },
];

export const NAV_GROUPS = [...new Set(NAV_ITEMS.map((n) => n.group))].filter(
  (g) => g !== "footer",
);
