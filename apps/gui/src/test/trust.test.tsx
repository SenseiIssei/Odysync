/**
 * What the pages say when something went wrong, and what they ask before they
 * act: the places where a wrong word or a missing question does harm.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

vi.mock("../api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../api")>();
  return {
    ...real,
    scan: vi.fn(),
    apply: vi.fn(),
    cancelApply: vi.fn(async () => undefined),
    cancelScan: vi.fn(async () => undefined),
    hold: vi.fn(async () => undefined),
    unhold: vi.fn(async () => undefined),
    listBackends: vi.fn(real.listBackends),
    getSystemInfo: vi.fn(real.getSystemInfo),
    getConfig: vi.fn(real.getConfig),
    saveConfig: vi.fn(async () => undefined),
    cleanupScan: vi.fn(real.cleanupScan),
    cleanupRun: vi.fn(async () => []),
    restoreBackup: vi.fn(async () => undefined),
    checkSchedule: vi.fn(async () => false),
    getDefenderStatus: vi.fn(real.getDefenderStatus),
    deviceProblems: vi.fn(real.deviceProblems),
    componentsStatus: vi.fn(real.componentsStatus),
  };
});

import * as api from "../api";
import { emitLocal } from "../events";
import { StoreProvider } from "../store";
import Backup from "../pages/Backup";
import Dashboard from "../pages/Dashboard";
import Drivers from "../pages/Drivers";
import Security from "../pages/Security";
import Settings from "../pages/Settings";
import Updates from "../pages/Updates";
import type { ApplyResultDto, BackendDto, ScanResult, UpdateDto } from "../types";

const withStore = (node: React.ReactNode) => render(<StoreProvider>{node}</StoreProvider>);
const slow = { timeout: 8000 };

const update = (backend: string, name: string): UpdateDto => ({
  backend,
  id: `${backend}:${name}`,
  name,
  installed: "1.0",
  available: "2.0",
  size_bytes: null,
});

const scanOf = (over: Partial<ScanResult> = {}): ScanResult => ({
  actionable: [],
  skipped: [],
  total: 0,
  failed_backends: [],
  ...over,
});

const resultOf = (over: Partial<ApplyResultDto> = {}): ApplyResultDto => ({
  updated: 0,
  failed: 0,
  skipped: 0,
  reboot_required: false,
  exit_code: 0,
  entries: [],
  ...over,
});

const firmwareSource: BackendDto = {
  kind: "dell-firmware",
  name: "Dell firmware",
  available: true,
  enabled: true,
  opt_in: false,
  verification: "",
  verification_detail: "",
  attended: true,
  developer: false,
};

beforeEach(async () => {
  const real = await vi.importActual<typeof import("../api")>("../api");
  vi.mocked(api.scan).mockReset().mockResolvedValue(scanOf());
  vi.mocked(api.apply).mockReset().mockResolvedValue(resultOf());
  vi.mocked(api.cleanupScan).mockReset().mockResolvedValue([]);
  vi.mocked(api.deviceProblems).mockReset().mockResolvedValue([]);
  vi.mocked(api.componentsStatus).mockReset().mockResolvedValue([]);
  vi.mocked(api.getSystemInfo).mockReset().mockImplementation(real.getSystemInfo);
  vi.mocked(api.listBackends)
    .mockReset()
    .mockImplementation(async () => [...(await real.listBackends()), firmwareSource]);
  for (const fn of [api.hold, api.unhold, api.saveConfig, api.restoreBackup, api.cancelApply]) {
    vi.mocked(fn).mockClear();
  }
});

afterEach(() => vi.restoreAllMocks());

describe("a check that failed", () => {
  it("is not called up to date on Updates, and names the source", async () => {
    vi.mocked(api.scan).mockResolvedValue(
      scanOf({ failed_backends: [{ backend: "winget", error: "timeout" }] }),
    );
    withStore(<Updates />);
    expect(
      await screen.findByText("No updates from the sources that answered", {}, slow),
    ).toBeTruthy();
    expect(screen.getByText(/winget could not be checked, so the list may be incomplete/)).toBeTruthy();
    expect(screen.queryByText("Everything is up to date")).toBeNull();
    expect(screen.getAllByRole("button", { name: "Try again" }).length).toBeGreaterThan(0);
  });

  it("is up to date when every source answered", async () => {
    withStore(<Updates />);
    expect(await screen.findByText("Everything is up to date", {}, slow)).toBeTruthy();
  });

  it("is reported with a retry when the whole scan failed", async () => {
    vi.mocked(api.scan).mockRejectedValue("no network");
    withStore(<Updates />);
    expect(await screen.findByText(/Updates could not be checked: no network/, {}, slow)).toBeTruthy();
    expect(screen.queryByText("Everything is up to date")).toBeNull();

    vi.mocked(api.scan).mockResolvedValue(scanOf());
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Everything is up to date", {}, slow)).toBeTruthy();
  });

  it("takes the score off Home instead of showing 100", async () => {
    vi.mocked(api.scan).mockRejectedValue("no network");
    withStore(<Dashboard onNavigate={() => {}} />);
    expect(
      await screen.findByRole("heading", { level: 1, name: "Updates could not be checked" }, slow),
    ).toBeTruthy();
    expect(screen.getByRole("img", { name: "Not checked" })).toBeTruthy();
    expect(screen.queryByText("Everything is taken care of")).toBeNull();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  }, 15000);

  it("is listed on Home with the source that did not answer", async () => {
    vi.mocked(api.scan).mockResolvedValue(
      scanOf({ failed_backends: [{ backend: "windows-update", error: "timeout" }] }),
    );
    withStore(<Dashboard onNavigate={() => {}} />);
    expect(await screen.findByText("1 update source could not be checked", {}, slow)).toBeTruthy();
    expect(await screen.findByText(/security . quality.*There may be updates/, {}, slow)).toBeTruthy();
    expect(screen.getByText("Not fully checked")).toBeTruthy();
    expect(screen.queryByText("Everything is taken care of")).toBeNull();
  }, 15000);

  it("is shown on Drivers rather than passing for a healthy machine", async () => {
    vi.mocked(api.scan).mockRejectedValue("no network");
    vi.mocked(api.deviceProblems).mockRejectedValue("access denied");
    withStore(<Drivers />);
    expect(await screen.findByText(/Driver updates could not be checked: no network/, {}, slow)).toBeTruthy();
    expect(await screen.findByText(/Devices could not be checked: access denied/, {}, slow)).toBeTruthy();
    expect(screen.queryByText("No driver updates")).toBeNull();
    expect(screen.queryByText(/Every device reports/)).toBeNull();
  });

  it("calls an unreadable Defender unknown, not off", async () => {
    vi.mocked(api.getDefenderStatus).mockResolvedValue({
      real_time_protection: null,
      tamper_protection: true,
      antivirus_enabled: null,
      signature_age_days: null,
      signature_version: "",
      last_quick_scan: null,
      last_full_scan: null,
    });
    withStore(<Security />);
    const card = await screen.findByText("Real-time protection", {}, slow);
    expect(card.parentElement?.textContent).toContain("could not be read");
    expect(screen.queryByText("off")).toBeNull();
  });
});

describe("choosing updates", () => {
  const three = scanOf({
    actionable: [
      update("direct", "Editor"),
      update("pip", "requests"),
      update("dell-firmware", "BIOS"),
    ],
    total: 3,
  });

  it("ticks neither developer packages nor anything attended by itself", async () => {
    vi.mocked(api.scan).mockResolvedValue(three);
    withStore(<Updates />);
    await waitFor(
      () => expect((screen.getByLabelText("Include Editor") as HTMLInputElement).checked).toBe(true),
      slow,
    );
    expect((screen.getByLabelText("Include requests") as HTMLInputElement).checked).toBe(false);
    expect((screen.getByLabelText("Include BIOS") as HTMLInputElement).checked).toBe(false);
  });

  it("asks before firmware, forgets the question when the selection changes, and sends the restore point as ticked", async () => {
    vi.mocked(api.scan).mockResolvedValue(three);
    const real = await vi.importActual<typeof import("../api")>("../api");
    vi.mocked(api.getConfig).mockImplementation(async () => ({
      ...(await real.getConfig()),
      restore_point: false,
    }));
    withStore(<Updates />);
    fireEvent.click(await screen.findByLabelText("Include BIOS", {}, slow));
    await waitFor(() =>
      expect((screen.getByLabelText("Create a restore point first") as HTMLInputElement).checked).toBe(false),
    );

    const install = await screen.findByRole("button", { name: "Install 2" }, slow);
    fireEvent.click(install);
    expect(api.apply).not.toHaveBeenCalled();
    const bar = screen.getByRole("alertdialog");
    // A second click in the same place does nothing, and Cancel has the focus.
    expect((install as HTMLButtonElement).disabled).toBe(true);
    expect(document.activeElement).toBe(within(bar).getByRole("button", { name: "Cancel" }));

    fireEvent.click(screen.getByLabelText("Include requests"));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Install 3" }));
    fireEvent.click(screen.getByRole("button", { name: "Install 3, including firmware" }));
    await waitFor(() => expect(api.apply).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.apply).mock.calls[0][0].restore_point).toBe(false);
  });

  it("holds a package under its own id, without the source twice", async () => {
    vi.mocked(api.scan).mockResolvedValue(three);
    withStore(<Updates />);
    fireEvent.click(await screen.findByRole("button", { name: "Hold Editor" }, slow));
    await waitFor(() => expect(api.hold).toHaveBeenCalledWith({ backend: "direct", id: "Editor" }));
    fireEvent.click(await screen.findByRole("button", { name: "Release Editor" }));
    await waitFor(() => expect(api.unhold).toHaveBeenCalledWith({ backend: "direct", id: "Editor" }));
  });
});

describe("installing", () => {
  it("runs once, however many pages ask, and can be cancelled", async () => {
    vi.mocked(api.scan).mockResolvedValue(
      scanOf({ actionable: [update("direct", "Editor")], total: 1 }),
    );
    let finish: (r: ApplyResultDto) => void = () => {};
    vi.mocked(api.apply).mockImplementation(
      () => new Promise<ApplyResultDto>((resolve) => (finish = resolve)),
    );
    withStore(
      <>
        <Updates />
        <Dashboard onNavigate={() => {}} />
      </>,
    );
    await screen.findByRole("button", { name: "Fix everything" }, slow);
    fireEvent.click(screen.getByRole("button", { name: "Install 1" }));

    // Home sees the install and offers no second one.
    expect(await screen.findByRole("heading", { level: 1, name: "Installing updates…" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Fix everything" })).toBeNull();
    expect(api.apply).toHaveBeenCalledTimes(1);

    act(() =>
      emitLocal("apply-progress", {
        package: "Editor",
        current: 0,
        total: 1,
        phase: "installing",
        percent: 0,
      }),
    );
    expect(screen.getByRole("progressbar", { name: "Installation progress" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel the installation" }));
    await waitFor(() => expect(api.cancelApply).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Finishing Editor, then stopping")).toBeTruthy();

    act(() =>
      finish(resultOf({ skipped: 1, entries: [{ name: "Editor", status: "skipped", detail: "cancelled" }] })),
    );
    expect(await screen.findByText("Installation cancelled")).toBeTruthy();
  }, 20000);

  it("says which update failed and why, instead of Done", async () => {
    vi.mocked(api.scan).mockResolvedValue(
      scanOf({ actionable: [update("direct", "Editor")], total: 1 }),
    );
    vi.mocked(api.apply).mockResolvedValue(
      resultOf({ failed: 1, entries: [{ name: "Editor", status: "failed", detail: "it is running" }] }),
    );
    withStore(<Dashboard onNavigate={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Fix everything" }, slow));
    expect(await screen.findByText(/Finished, but not everything worked/, {}, slow)).toBeTruthy();
    expect(screen.getByText(/was not installed: it is running/)).toBeTruthy();
    expect(screen.queryByText("Done.")).toBeNull();
  }, 20000);

  it("explains a refusal because another install is running", async () => {
    vi.mocked(api.scan).mockResolvedValue(
      scanOf({ actionable: [update("direct", "Editor")], total: 1 }),
    );
    vi.mocked(api.apply).mockRejectedValue("apply-running");
    withStore(<Updates />);
    const install = await screen.findByRole("button", { name: "Install 1" }, slow);
    await waitFor(() => expect((install as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(install);
    expect(await screen.findByText(/An update is being installed\. Wait for it/)).toBeTruthy();
  });
});

describe("scanning", () => {
  it("says how many sources answered and which are still out, and can be cancelled", async () => {
    let finish: (r: ScanResult) => void = () => {};
    vi.mocked(api.scan).mockImplementation(
      () => new Promise<ScanResult>((resolve) => (finish = resolve)),
    );
    withStore(<Updates />);
    await screen.findByText("Checking every update source…");
    await waitFor(() => expect(api.scan).toHaveBeenCalled(), slow);
    const event = (backend: string, name: string, state: string, done: number) =>
      emitLocal("scan-progress", { backend, name, state, found: 0, done, total: 3 });
    act(() => {
      event("direct", "Direct", "started", 0);
      event("windows-update", "Windows Update", "started", 0);
      event("pip", "pip", "started", 0);
      event("direct", "Direct", "done", 1);
      event("pip", "pip", "failed", 2);
    });
    expect(
      screen.getByText("Checked 2 of 3 sources, still waiting for Windows Update"),
    ).toBeTruthy();
    const bar = screen.getByRole("progressbar", { name: "Update sources checked" });
    expect(bar.getAttribute("aria-valuenow")).toBe("67");

    fireEvent.click(screen.getByRole("button", { name: "Cancel the check" }));
    await waitFor(() => expect(api.cancelScan).toHaveBeenCalledTimes(1));
    act(() =>
      finish(scanOf({ failed_backends: [{ backend: "windows-update", error: "cancelled" }] })),
    );
    expect(
      await screen.findByText(/The check was cancelled before Windows Update answered/),
    ).toBeTruthy();
  });

  it("does not poll the security audit when the startup audit is off", () => {
    const intervals = vi.spyOn(globalThis, "setInterval");
    localStorage.setItem("odysync.securityScanOnStartup", "false");
    withStore(<div />);
    expect(intervals.mock.calls.map((c) => c[1])).not.toContain(600_000);
    localStorage.setItem("odysync.securityScanOnStartup", "true");
    withStore(<div />);
    expect(intervals.mock.calls.map((c) => c[1])).toContain(600_000);
  });
});

describe("restore points", () => {
  it("never restore on a double click", async () => {
    vi.mocked(api.getSystemInfo).mockResolvedValue({
      os: "Windows",
      platform: "windows",
      elevated: true,
      version: "0",
      config_error: null,
    });
    withStore(<Backup />);
    const button = await screen.findByRole("button", { name: /^Restore Odysync/ }, slow);
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(button);
    fireEvent.click(button);
    expect(api.restoreBackup).not.toHaveBeenCalled();

    const bar = screen.getByRole("alertdialog");
    fireEvent.keyDown(bar, { key: "Escape" });
    expect(screen.queryByRole("alertdialog")).toBeNull();

    fireEvent.click(button);
    fireEvent.click(screen.getByRole("button", { name: "Restart and restore" }));
    await waitFor(() => expect(api.restoreBackup).toHaveBeenCalledWith(41));
  });
});

describe("settings", () => {
  it("lets Enter through in the exclusion list and stores it parsed", async () => {
    withStore(<Settings />);
    const box = (await screen.findByLabelText(
      "Excluded packages, one per line",
      {},
      slow,
    )) as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: "vscode\n" } });
    expect(box.value).toBe("vscode\n");
    fireEvent.change(box, { target: { value: "vscode\n pip:requests \n\n" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.saveConfig).mock.calls[0][0].policy.exclude).toEqual([
      "vscode",
      "pip:requests",
    ]);
  });

  it("clamps a number when the field is left, not while it is typed", async () => {
    withStore(<Settings />);
    const field = (await screen.findByLabelText("Timeout (seconds)", {}, slow)) as HTMLInputElement;
    fireEvent.change(field, { target: { value: "1" } });
    expect(field.value).toBe("1");
    fireEvent.blur(field);
    expect(field.value).toBe("10");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(field.value).toBe("10");
  });

  it("releases a hold", async () => {
    const real = await vi.importActual<typeof import("../api")>("../api");
    vi.mocked(api.getConfig).mockImplementation(async () => {
      const c = await real.getConfig();
      return {
        ...c,
        policy: {
          ...c.policy,
          holds: [
            { package: "direct:user:vscode", pin: null, note: null },
            { package: "bare-name", pin: null, note: null },
          ],
        },
      };
    });
    withStore(<Settings />);
    fireEvent.click(await screen.findByRole("button", { name: "Release direct:user:vscode" }, slow));
    await waitFor(() =>
      expect(api.unhold).toHaveBeenCalledWith({ backend: "direct", id: "user:vscode" }),
    );
    // Nothing the release command could address, so nothing to press.
    expect(screen.queryByRole("button", { name: "Release bare-name" })).toBeNull();
  });

  it("moves a choice with the arrow keys", async () => {
    withStore(<Settings />);
    const system = await screen.findByRole("radio", { name: "System" });
    expect(system.tabIndex).toBe(0);
    fireEvent.keyDown(system, { key: "ArrowRight" });
    const light = screen.getByRole("radio", { name: "Light" });
    expect(light.getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(light);
    expect(system.tabIndex).toBe(-1);
  });
});
