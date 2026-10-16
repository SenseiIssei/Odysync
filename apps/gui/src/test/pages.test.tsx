import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

// The pages talk to the backend only through `api`; spying on the two calls
// that change things lets the tests assert what would have been sent.
vi.mock("../api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../api")>();
  return {
    ...real,
    scan: vi.fn(real.scan),
    apply: vi.fn(real.apply),
    cleanupRun: vi.fn(async () => []),
    saveConfig: vi.fn(async () => undefined),
    checkSchedule: vi.fn(async () => false),
    createSchedule: vi.fn(async () => "OdysyncCare"),
  };
});

import * as api from "../api";
import { StoreProvider } from "../store";
import Dashboard from "../pages/Dashboard";
import Maintenance from "../pages/Maintenance";
import Settings from "../pages/Settings";
import Updates from "../pages/Updates";

function withStore(node: React.ReactNode) {
  return render(<StoreProvider>{node}</StoreProvider>);
}

describe("Updates", () => {
  it("lists the updates the demo scan returns, with their version jump", async () => {
    withStore(<Updates />);
    const row = (await screen.findByText("Visual Studio Code", {}, { timeout: 4000 })).closest("tr");
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText("1.140.0")).toBeTruthy();
    expect(within(row as HTMLElement).getByText("1.142.0")).toBeTruthy();
    // Held-back updates say why.
    expect(screen.getAllByText("requires administrator privileges").length).toBeGreaterThan(0);
  });
});

describe("Cleanup", () => {
  it("asks a second time before anything that cannot be undone is deleted", async () => {
    withStore(<Maintenance />);
    const recycle = await screen.findByLabelText("Clean Recycle Bin", {}, { timeout: 4000 });
    fireEvent.click(recycle);

    const clean = await screen.findByRole("button", { name: /Clean \d+ categor/ });
    fireEvent.click(clean);

    // The first click only arms the confirmation and names the consequence.
    expect(api.cleanupRun).not.toHaveBeenCalled();
    expect(await screen.findByText(/cannot be undone:/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Yes, delete permanently" }));
    await waitFor(() => expect(api.cleanupRun).toHaveBeenCalledTimes(1));
    const ids = vi.mocked(api.cleanupRun).mock.calls[0][0];
    expect(ids).toContain("recycle-bin");
    // Defaults ride along; opt-in categories that were not ticked do not.
    expect(ids).toContain("temp-files");
    expect(ids).not.toContain("browser-cache");
    expect(ids).not.toContain("previous-windows");
  });
});

describe("Settings", () => {
  it("turns an optional source on through enabled_backends, not disabled_backends", async () => {
    withStore(<Settings />);
    const winget = await screen.findByRole("switch", { name: "Use winget" }, { timeout: 4000 });
    expect(winget.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(winget);
    expect(winget.getAttribute("aria-checked")).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(api.saveConfig).mock.calls[0][0];
    expect(saved.enabled_backends).toContain("winget");
    expect(saved.disabled_backends).not.toContain("winget");
  });
});

describe("Home", () => {
  it("fixes safe updates and junk files in one click, and leaves drivers alone", async () => {
    const real = await vi.importActual<typeof import("../api")>("../api");
    vi.mocked(api.scan).mockImplementation(async () => {
      const result = await real.scan();
      return {
        ...result,
        actionable: [
          ...result.actionable,
          {
            backend: "windows-drivers",
            id: "intel-wifi",
            name: "Intel Wi-Fi driver",
            installed: "23.1",
            available: "23.4",
            size_bytes: null,
          },
        ],
      };
    });

    withStore(<Dashboard onNavigate={() => {}} />);
    const fix = await screen.findByRole("button", { name: "Fix everything" }, { timeout: 8000 });
    fireEvent.click(fix);

    await waitFor(() => expect(api.apply).toHaveBeenCalledTimes(1), { timeout: 4000 });
    const sent = vi.mocked(api.apply).mock.calls[0][0].updates.map((u) => u.backend);
    expect(sent.length).toBeGreaterThan(0);
    expect(sent).not.toContain("windows-drivers");
    // Developer packages are the developer's call, not a background job.
    expect(sent).not.toContain("pip");
    expect(sent).not.toContain("npm");

    await waitFor(() => expect(api.cleanupRun).toHaveBeenCalled(), { timeout: 4000 });
    const ids = vi.mocked(api.cleanupRun).mock.calls.at(-1)![0];
    expect(ids).toContain("temp-files");
    expect(ids).not.toContain("recycle-bin");
    expect(ids).not.toContain("browser-cache");

    // The driver is still there, for a person to start.
    expect(await screen.findByText(/driver or system update to review/, {}, { timeout: 4000 })).toBeTruthy();
  }, 20000);

  it("turns automatic care on as a scheduled care run", async () => {
    withStore(<Dashboard onNavigate={() => {}} />);
    const toggle = await screen.findByRole("switch", { name: "Automatic care" });
    await waitFor(() => expect((toggle as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(toggle);
    await waitFor(() => expect(api.createSchedule).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.createSchedule).mock.calls[0][0]).toMatchObject({
      task_name: "OdysyncCare",
      care: true,
    });
  });
});
