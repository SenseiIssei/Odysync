import { describe, expect, it } from "vitest";
import { assess, safeCleanupItems, type CareInput } from "../care";
import type { CleanupItemDto, UpdateDto } from "../types";

const GB = 1024 ** 3;

const update = (name: string, backend = "direct"): UpdateDto =>
  ({ backend, id: name, name, installed: "1", available: "2", size_bytes: null }) as UpdateDto;

const cleanupItem = (over: Partial<CleanupItemDto>): CleanupItemDto =>
  ({
    id: "temp-files",
    label: "Temporary files",
    consequence: "",
    bytes: 3 * GB,
    file_count: 10,
    reversibility: "regenerated",
    selected_by_default: true,
    report_only: false,
    error: null,
    note: null,
    ...over,
  }) as CleanupItemDto;

const clean: CareInput = {
  updates: [],
  attendedUpdates: [],
  needsAdmin: 0,
  devices: 0,
  outdatedRuntimes: 0,
  seriousFindings: 0,
  minorFindings: 0,
  defender: null,
  safeCleanup: [],
  health: null,
};

describe("care score", () => {
  it("is 100 with nothing listed when there is nothing to do", () => {
    const a = assess(clean);
    expect(a.score).toBe(100);
    expect(a.grade).toBe("Excellent");
    expect(a.issues).toEqual([]);
  });

  it("lists every point it takes off, and the score is what is left", () => {
    const a = assess({
      ...clean,
      updates: [update("VS Code"), update("Git")],
      devices: 1,
      safeCleanup: [cleanupItem({})],
    });
    const total = a.issues.reduce((sum, i) => sum + i.points, 0);
    expect(a.score).toBe(100 - total);
    // Warnings before information, the bigger one first within each.
    expect(a.issues.map((i) => i.id)).toEqual(["devices", "updates", "cleanup"]);
  });

  it("never offers to install drivers in the one-click fix", () => {
    const a = assess({ ...clean, attendedUpdates: [update("GeForce driver", "nvidia-gpu")] });
    const drivers = a.issues.find((i) => i.id === "drivers");
    expect(drivers?.fix).toBeNull();
    expect(a.fixable).toBe(0);
  });

  it("puts what is dangerous first", () => {
    const a = assess({
      ...clean,
      updates: [update("A")],
      defender: {
        real_time_protection: false,
        tamper_protection: true,
        antivirus_enabled: true,
        signature_age_days: 0,
        signature_version: "1",
        last_quick_scan: null,
        last_full_scan: null,
      },
    });
    expect(a.issues[0].id).toBe("defender-off");
    expect(a.issues[0].tone).toBe("danger");
  });

  it("does not mention a few megabytes of cache", () => {
    const a = assess({ ...clean, safeCleanup: [cleanupItem({ bytes: 50 * 1024 * 1024 })] });
    expect(a.issues).toEqual([]);
  });

  it("only clears categories that are safe without asking", () => {
    const items = safeCleanupItems([
      cleanupItem({ id: "temp-files" }),
      cleanupItem({ id: "recycle-bin", reversibility: "loses-capability", selected_by_default: false }),
      cleanupItem({ id: "browser-cache", reversibility: "discardable", selected_by_default: false }),
      cleanupItem({ id: "thumbnails", bytes: 0 }),
      cleanupItem({ id: "broken", error: "access denied" }),
    ]);
    expect(items.map((i) => i.id)).toEqual(["temp-files"]);
  });
});
