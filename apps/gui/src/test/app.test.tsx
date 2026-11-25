/** The window around the pages, as it is inside Tauri: title bar and sidebar. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

vi.mock("../bridge", async () => {
  const { demoInvoke } = await import("../demo/demo");
  return { inTauri: true, invoke: demoInvoke };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../api")>();
  return { ...real, quitApp: vi.fn(), restartAsAdmin: vi.fn() };
});

import * as api from "../api";
import App from "../App";

beforeEach(() => {
  vi.mocked(api.quitApp).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.restartAsAdmin).mockReset().mockResolvedValue(undefined);
});

const quitButton = () => screen.findByRole("button", { name: "Quit Odysync" });

describe("quitting", () => {
  it("just quits when nothing is being installed", async () => {
    render(<App />);
    fireEvent.click(await quitButton());
    await waitFor(() => expect(api.quitApp).toHaveBeenCalledWith(false));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("asks during an install, stays by default, and quits only when told to", async () => {
    vi.mocked(api.quitApp).mockImplementation(async (force) => {
      if (!force) throw "apply-running";
    });
    render(<App />);
    fireEvent.click(await quitButton());
    const bar = await screen.findByRole("alertdialog");
    expect(bar.textContent).toContain("An update is being installed. Quit anyway?");
    const stay = within(bar).getByRole("button", { name: "Stay" });
    expect(document.activeElement).toBe(stay);
    fireEvent.click(stay);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(api.quitApp).not.toHaveBeenCalledWith(true);

    fireEvent.click(await quitButton());
    fireEvent.click(await screen.findByRole("button", { name: "Quit anyway" }));
    await waitFor(() => expect(api.quitApp).toHaveBeenLastCalledWith(true));
  });

  it("shows why a restart as administrator did not happen", async () => {
    vi.mocked(api.restartAsAdmin).mockRejectedValue("apply-running");
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Run as admin" }));
    expect(await screen.findByText(/Not restarted as administrator\. An update is being installed/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText(/Not restarted as administrator/)).toBeNull();
  });
});

describe("navigation", () => {
  it("moves the focus to the heading of the page that opens", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Settings"));
    expect(document.activeElement?.tagName).toBe("H1");
  });

  it("names every entry of the collapsed sidebar", async () => {
    localStorage.setItem("odysync.sidebar", "collapsed");
    render(<App />);
    const nav = await screen.findByRole("navigation", { name: "Sections" });
    for (const button of within(nav).getAllByRole("button")) {
      expect(button.getAttribute("aria-label")).toBeTruthy();
    }
    expect(screen.getByRole("button", { name: "Expand sidebar" }).getAttribute("aria-expanded")).toBe("false");
  });
});
