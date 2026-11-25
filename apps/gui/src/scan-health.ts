/**
 * How far an update scan can be trusted.
 *
 * "No updates found" and "nobody could be asked" both arrive as an empty list.
 * Everything that wants to say "up to date" asks here first.
 */
import type { BackendDto, BackendErrorDto, ScanResult } from "./types";

export type ScanHealth =
  /** No scan has finished yet. */
  | { state: "unknown" }
  | { state: "complete" }
  /** Some sources answered, these did not. */
  | { state: "partial"; failed: BackendErrorDto[] }
  /** Nothing was checked. */
  | { state: "failed"; error: string; failed: BackendErrorDto[] };

export function scanHealth(
  scan: { data: ScanResult | null; error: string | null },
  backends: BackendDto[],
): ScanHealth {
  if (scan.error !== null) return { state: "failed", error: scan.error, failed: [] };
  if (scan.data === null) return { state: "unknown" };

  const failed = scan.data.failed_backends;
  if (failed.length === 0) return { state: "complete" };

  // Every usable source failed, and nothing was found by any other: that is
  // not a partial result, it is none at all.
  const usable = backends.filter((b) => b.available && b.enabled).map((b) => b.kind);
  const failedKinds = new Set(failed.map((f) => f.backend));
  if (scan.data.total === 0 && usable.length > 0 && usable.every((k) => failedKinds.has(k))) {
    return {
      state: "failed",
      error: failed.every(isCancelled)
        ? "The check was cancelled before any source answered."
        : "None of the update sources could be reached.",
      failed,
    };
  }
  return { state: "partial", failed };
}

export function isCancelled(f: BackendErrorDto): boolean {
  return f.error === "cancelled";
}

/** "Windows Update, winget and 3 more", for a line of text. */
export function nameList(names: string[], limit = 3): string {
  if (names.length <= limit) return names.join(", ");
  return `${names.slice(0, limit).join(", ")} and ${names.length - limit} more`;
}
