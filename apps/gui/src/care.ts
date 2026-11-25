/**
 * The care score: one number for "how is this machine doing", and the list
 * of things that lower it.
 *
 * Optimizer suites are known for scores that are always alarming and never
 * explained. This one is the opposite on purpose: every point taken off is a
 * line in the list with the reason next to it, nothing that cannot be acted on
 * costs points, and a machine with nothing to do scores 100.
 *
 * It is also honest about what it does not know. A check that failed is an
 * entry in the list, never a silent zero, and when the update check failed as
 * a whole there is no score at all.
 */

import type { Tab } from "./nav";
import type {
  CleanupItemDto,
  DefenderStatusDto,
  HealthReportDto,
  UpdateDto,
} from "./types";
import { formatSize } from "./format";
import { isCancelled, nameList, type ScanHealth } from "./scan-health";

export type CareTone = "danger" | "warning" | "info";

export interface CareIssue {
  id: string;
  title: string;
  detail: string;
  /** Points this takes off the score. */
  points: number;
  tone: CareTone;
  /** Fixed by the one-click fix, or `null` when it needs a person. */
  fix: "updates" | "cleanup" | null;
  /** The check itself failed, so the way forward is to run it again. */
  retry?: boolean;
  /** Where to look at it. */
  tab: Tab;
}

/** A check other than the update scan that could not be completed. */
export interface FailedCheck {
  id: string;
  /** e.g. "Junk files". */
  label: string;
  error: string;
  tab: Tab;
}

export interface CareInput {
  /** How far the update scan got; its failures are part of the assessment. */
  scan: ScanHealth;
  /** The display name of an update source. */
  sourceName: (backend: string) => string;
  /** Checks other than the update scan that failed. */
  failedChecks: FailedCheck[];
  /** Audit sections that could not run, in a report that did arrive. */
  incompleteSecurity: number;
  /** The updates the one-click fix may install (no drivers, firmware, OS upgrades). */
  updates: UpdateDto[];
  /** Ready drivers, firmware and OS upgrades, for a person to start. */
  attendedUpdates: UpdateDto[];
  /** pip, npm, cargo and other packages with newer versions, for the developer. */
  developerUpdates: UpdateDto[];
  /** Updates that only need administrator rights to go ahead. */
  needsAdmin: number;
  devices: number;
  outdatedRuntimes: number;
  seriousFindings: number;
  minorFindings: number;
  defender: DefenderStatusDto | null;
  /** The cleanup categories that are safe to clear without asking. */
  safeCleanup: CleanupItemDto[];
  health: HealthReportDto | null;
}

export interface CareAssessment {
  score: number;
  /** False when too little could be checked for the score to mean anything. */
  scoreKnown: boolean;
  grade: string;
  tone: "success" | "accent" | "warning" | "danger";
  issues: CareIssue[];
  /** How many issues the one-click fix resolves. */
  fixable: number;
}

const plural = (n: number, one: string, many = `${one}s`) => (n === 1 ? one : many);

/** Below this, clearing caches is not worth a line. */
const CLEANUP_WORTH_MENTIONING = 200 * 1024 * 1024;

export function safeCleanupBytes(items: CleanupItemDto[]): number {
  return items.reduce((sum, c) => sum + c.bytes, 0);
}

/** Cleanup categories the one-click fix and automatic care may clear. */
export function safeCleanupItems(items: CleanupItemDto[]): CleanupItemDto[] {
  return items.filter(
    (c) => c.selected_by_default && !c.error && c.reversibility !== "loses-capability" && c.bytes > 0,
  );
}

export function assess(input: CareInput): CareAssessment {
  const issues: CareIssue[] = [];
  const add = (issue: CareIssue) => issues.push(issue);

  // What could not be checked. Points come off because a machine nobody could
  // look at is not known to be fine, and the row says how to get them back.
  if (input.scan.state === "failed") {
    add({
      id: "scan-failed",
      title: "Updates could not be checked",
      detail: input.scan.error,
      points: 0,
      tone: "danger",
      fix: null,
      retry: true,
      tab: "updates",
    });
  } else if (input.scan.state === "partial") {
    const { failed } = input.scan;
    const n = failed.length;
    const cancelled = failed.every(isCancelled);
    add({
      id: "sources-unchecked",
      title: `${n} update ${plural(n, "source")} ${cancelled ? "not checked" : "could not be checked"}`,
      detail: `${nameList(failed.map((f) => input.sourceName(f.backend)), 4)}. ${
        cancelled ? "The check was cancelled." : "There may be updates nobody saw."
      }`,
      points: Math.min(12, 3 * n),
      tone: "warning",
      fix: null,
      retry: true,
      tab: "updates",
    });
  }
  for (const check of input.failedChecks) {
    add({
      id: `failed-${check.id}`,
      title: `${check.label} could not be checked`,
      detail: check.error,
      points: 3,
      tone: "warning",
      fix: null,
      retry: true,
      tab: check.tab,
    });
  }
  if (input.incompleteSecurity > 0) {
    const n = input.incompleteSecurity;
    add({
      id: "security-incomplete",
      title: `${n} security ${plural(n, "check")} could not run`,
      detail: "The Security page names them. Some need administrator rights.",
      points: 0,
      tone: "info",
      fix: null,
      tab: "security",
    });
  }

  if (input.updates.length > 0) {
    const n = input.updates.length;
    add({
      id: "updates",
      title: `${n} ${plural(n, "update")} ready`,
      detail: input.updates
        .slice(0, 3)
        .map((u) => u.name)
        .join(", ")
        .concat(n > 3 ? ` and ${n - 3} more` : ""),
      points: Math.min(24, 3 * n),
      tone: "warning",
      fix: "updates",
      tab: "updates",
    });
  }

  const reclaim = safeCleanupBytes(input.safeCleanup);
  if (reclaim >= CLEANUP_WORTH_MENTIONING) {
    add({
      id: "cleanup",
      title: `${formatSize(reclaim)} of junk files`,
      detail: "Temporary files and caches that programs rebuild on their own.",
      points: reclaim >= 10 * 1024 ** 3 ? 8 : reclaim >= 2 * 1024 ** 3 ? 5 : 2,
      tone: "info",
      fix: "cleanup",
      tab: "maintenance",
    });
  }

  if (input.attendedUpdates.length > 0) {
    const n = input.attendedUpdates.length;
    add({
      id: "drivers",
      title: `${n} driver or system ${plural(n, "update")} to review`,
      detail: "Drivers and firmware are never installed without you. Start them from the Drivers page.",
      points: Math.min(10, 4 * n),
      tone: "warning",
      fix: null,
      tab: "drivers",
    });
  }

  if (input.developerUpdates.length > 0) {
    const n = input.developerUpdates.length;
    add({
      id: "developer",
      title: `${n} developer ${plural(n, "package")} can be updated`,
      detail: "pip, npm and similar packages are only updated when you ask. Your projects may depend on the versions you have.",
      points: Math.min(4, n),
      tone: "info",
      fix: null,
      tab: "updates",
    });
  }

  if (input.needsAdmin > 0) {
    const n = input.needsAdmin;
    add({
      id: "admin",
      title: `${n} more ${plural(n, "update")} need administrator rights`,
      detail: "Use Run as admin in the title bar, then fix again.",
      points: Math.min(5, n),
      tone: "info",
      fix: null,
      tab: "updates",
    });
  }

  if (input.devices > 0) {
    const n = input.devices;
    add({
      id: "devices",
      title: `${n} ${plural(n, "device")} not working`,
      detail: "Windows reports a problem; the Drivers page says what it means.",
      points: Math.min(16, 8 * n),
      tone: "warning",
      fix: null,
      tab: "drivers",
    });
  }

  if (input.outdatedRuntimes > 0) {
    const n = input.outdatedRuntimes;
    add({
      id: "runtimes",
      title: `${n} ${plural(n, "runtime")} out of date`,
      detail: "Visual C++, DirectX or .NET packages from Microsoft.",
      points: Math.min(6, 3 * n),
      tone: "info",
      fix: null,
      tab: "drivers",
    });
  }

  const d = input.defender;
  if (d && d.real_time_protection === null) {
    // Unknown is not off: no points, but no all-clear either.
    add({
      id: "defender-unknown",
      title: "Microsoft Defender's status could not be read",
      detail: "Whether real-time protection is on is not known.",
      points: 0,
      tone: "info",
      fix: null,
      tab: "security",
    });
  } else if (d && d.real_time_protection === false) {
    add({
      id: "defender-off",
      title: "Real-time protection is off",
      detail: "Microsoft Defender is not watching for malware right now.",
      points: 20,
      tone: "danger",
      fix: null,
      tab: "security",
    });
  } else if (d && d.signature_age_days !== null && d.signature_age_days > 3) {
    add({
      id: "defender-old",
      title: `Virus definitions are ${d.signature_age_days} days old`,
      detail: "Defender recognises the newest threats only with current definitions.",
      points: 6,
      tone: "warning",
      fix: null,
      tab: "security",
    });
  }

  if (input.seriousFindings > 0) {
    const n = input.seriousFindings;
    add({
      id: "security-serious",
      title: `${n} serious security ${plural(n, "finding")}`,
      detail: "Look at these first. The Security page explains each one.",
      points: Math.min(30, 15 * n),
      tone: "danger",
      fix: null,
      tab: "security",
    });
  } else if (input.minorFindings > 0) {
    const n = input.minorFindings;
    add({
      id: "security-minor",
      title: `${n} minor security ${plural(n, "finding")}`,
      detail: "Hardening suggestions, nothing urgent.",
      points: Math.min(8, 2 * n),
      tone: "info",
      fix: null,
      tab: "security",
    });
  }

  for (const disk of input.health?.disks ?? []) {
    if (disk.state === "bad" || disk.state === "warning") {
      add({
        id: `disk-${disk.name}`,
        title: `${disk.name}: ${disk.state === "bad" ? "failing" : "needs attention"}`,
        detail: disk.summary,
        points: disk.state === "bad" ? 25 : 10,
        tone: disk.state === "bad" ? "danger" : "warning",
        fix: null,
        tab: "hardware",
      });
    }
  }
  const battery = input.health?.battery;
  if (battery && (battery.state === "bad" || battery.state === "warning")) {
    add({
      id: "battery",
      title: battery.state === "bad" ? "The battery is worn out" : "The battery is wearing",
      detail: battery.summary,
      points: battery.state === "bad" ? 6 : 3,
      tone: "warning",
      fix: null,
      tab: "hardware",
    });
  }

  const order: Record<CareTone, number> = { danger: 0, warning: 1, info: 2 };
  issues.sort((a, b) => order[a.tone] - order[b.tone] || b.points - a.points);

  const score = Math.max(0, 100 - issues.reduce((sum, i) => sum + i.points, 0));
  const [grade, tone] =
    score >= 90
      ? (["Excellent", "success"] as const)
      : score >= 75
        ? (["Good", "accent"] as const)
        : score >= 50
          ? (["Needs care", "warning"] as const)
          : (["At risk", "danger"] as const);

  return {
    score,
    scoreKnown: input.scan.state === "complete" || input.scan.state === "partial",
    grade,
    tone,
    issues,
    fixable: issues.filter((i) => i.fix !== null).length,
  };
}
