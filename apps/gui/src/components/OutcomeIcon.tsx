import { CheckCircle2, Clock, XCircle } from "lucide-react";
import type { ApplyStatus } from "../types";

const WORDS: Record<ApplyStatus, string> = {
  updated: "Updated",
  "did-not-converge": "Failed",
  "verification-failed": "Failed",
  failed: "Failed",
  skipped: "Skipped",
};

/** How an update went, as an icon for the eye and a word for a screen reader. */
export function OutcomeIcon({ status }: { status: ApplyStatus }) {
  const icon =
    status === "updated" ? (
      <CheckCircle2 aria-hidden className="w-4 h-4 text-success" />
    ) : status === "skipped" ? (
      <Clock aria-hidden className="w-4 h-4 text-faint" />
    ) : (
      <XCircle aria-hidden className="w-4 h-4 text-danger" />
    );
  return (
    <span className="flex-shrink-0 mt-0.5">
      {icon}
      <span className="sr-only">{WORDS[status]}: </span>
    </span>
  );
}
