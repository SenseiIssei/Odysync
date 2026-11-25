/** Formatting helpers shared by every page. */

export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "-";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes < 1024 ** 4) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${(bytes / 1024 ** 4).toFixed(1)} TB`;
}

/** Dates coming out of PowerShell/WMI are not always parseable — never throw. */
export function formatDate(value: string): string {
  if (!value) return "-";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString();
}

/** Codes the command layer rejects with, in words. */
const ERROR_CODES: Record<string, string> = {
  "apply-running": "An update is being installed. Wait for it to finish, or cancel it.",
  cancelled: "Cancelled before it answered.",
};

/** Whether a command was refused because an update is being installed. */
export function isApplyRunning(e: unknown): boolean {
  return e === "apply-running" || (e instanceof Error && e.message === "apply-running");
}

export function errorText(e: unknown): string {
  if (e instanceof Error) return ERROR_CODES[e.message] ?? e.message;
  if (typeof e === "string") return ERROR_CODES[e] ?? e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
