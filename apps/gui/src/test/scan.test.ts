import { describe, expect, it, vi } from "vitest";
import { coalesce } from "../coalesce";
import { scanHealth } from "../scan-health";
import type { BackendDto, ScanResult } from "../types";

const backend = (kind: string, usable = true) =>
  ({ kind, name: kind, available: usable, enabled: true }) as BackendDto;

const result = (over: Partial<ScanResult> = {}): ScanResult => ({
  actionable: [],
  skipped: [],
  total: 0,
  failed_backends: [],
  ...over,
});

describe("scan health", () => {
  const backends = [backend("direct"), backend("pip"), backend("scoop", false)];

  it("is complete only when every source answered", () => {
    expect(scanHealth({ data: result(), error: null }, backends).state).toBe("complete");
    expect(scanHealth({ data: null, error: null }, backends).state).toBe("unknown");
  });

  it("is partial when some sources failed", () => {
    const data = result({ failed_backends: [{ backend: "pip", error: "timeout" }] });
    expect(scanHealth({ data, error: null }, backends)).toEqual({
      state: "partial",
      failed: data.failed_backends,
    });
  });

  it("is failed when the scan threw, whatever an earlier one had found", () => {
    expect(scanHealth({ data: result({ total: 3 }), error: "boom" }, backends)).toMatchObject({
      state: "failed",
      error: "boom",
    });
  });

  it("is failed when every usable source failed", () => {
    const failed = [
      { backend: "direct", error: "offline" },
      { backend: "pip", error: "offline" },
    ];
    const health = scanHealth({ data: result({ failed_backends: failed }), error: null }, backends);
    expect(health.state).toBe("failed");
  });
});

/** A loader whose answers the test hands out one by one. */
function controlled() {
  const answers: Array<(v: string) => void> = [];
  const values: string[] = [];
  const onRequest = vi.fn();
  const onEnd = vi.fn();
  const load = vi.fn(
    (begin: () => void) =>
      new Promise<string>((resolve) => {
        begin();
        answers.push(resolve);
      }),
  );
  const loads = coalesce<string>({
    load,
    onRequest,
    onValue: (v) => values.push(v),
    onError: () => {},
    onEnd,
  });
  const settle = () => new Promise((r) => setTimeout(r, 0));
  return { answers, values, load, loads, onRequest, onEnd, settle };
}

describe("coalesced loads", () => {
  it("joins a load that is already running", async () => {
    const c = controlled();
    const first = c.loads.refresh();
    expect(c.loads.refresh()).toBe(first);
    c.answers[0]("a");
    await first;
    expect(c.load).toHaveBeenCalledTimes(1);
    expect(c.values).toEqual(["a"]);
  });

  it("reloads exactly once more after a change, and never shows the stale answer", async () => {
    const c = controlled();
    const done = c.loads.refresh();
    void c.loads.reload();
    void c.loads.reload();
    c.answers[0]("stale");
    await c.settle();
    expect(c.load).toHaveBeenCalledTimes(2);
    expect(c.values).toEqual([]);
    expect(c.onEnd).not.toHaveBeenCalled();

    c.answers[1]("fresh");
    await done;
    expect(c.load).toHaveBeenCalledTimes(2);
    expect(c.values).toEqual(["fresh"]);
    expect(c.onEnd).toHaveBeenCalledTimes(1);
  });

  it("does not load twice for a change that came while the load was still queued", async () => {
    const begins: Array<() => void> = [];
    const answers: Array<(v: string) => void> = [];
    const load = vi.fn(
      (begin: () => void) =>
        new Promise<string>((resolve) => {
          begins.push(begin);
          answers.push(resolve);
        }),
    );
    const values: string[] = [];
    const loads = coalesce<string>({
      load,
      onRequest: () => {},
      onValue: (v) => values.push(v),
      onError: () => {},
      onEnd: () => {},
    });
    const done = loads.refresh();
    void loads.reload();
    begins[0]();
    answers[0]("fresh");
    await done;
    expect(load).toHaveBeenCalledTimes(1);
    expect(values).toEqual(["fresh"]);
  });

  it("tells a background poll from a load somebody asked for", async () => {
    const c = controlled();
    const done = c.loads.refresh({ quiet: true });
    expect(c.onRequest).toHaveBeenLastCalledWith(true);
    // Somebody asks while the poll runs: it becomes visible.
    void c.loads.refresh();
    expect(c.onRequest).toHaveBeenLastCalledWith(false);
    c.answers[0]("a");
    await done;
    expect(c.load).toHaveBeenCalledTimes(1);
  });
});
