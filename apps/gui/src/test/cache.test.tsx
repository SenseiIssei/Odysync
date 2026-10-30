import { afterEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { StoreProvider } from "../store";
import Updates from "../pages/Updates";

const remembered = (version: number, at = Date.now() - 60_000) =>
  JSON.stringify({
    v: version,
    at,
    data: {
      actionable: [
        {
          backend: "direct",
          id: "user:remembered-app",
          name: "Remembered App",
          installed: "1.0.0",
          available: "1.1.0",
          size_bytes: null,
        },
      ],
      skipped: [],
      total: 1,
      failed_backends: [],
    },
  });

afterEach(() => localStorage.clear());

describe("remembered results", () => {
  it("shows the last scan at once and replaces it with the fresh one", async () => {
    localStorage.setItem("odysync.cache.scan", remembered(1));
    render(
      <StoreProvider>
        <Updates />
      </StoreProvider>,
    );
    // Before any command has answered.
    expect(screen.getByText("Remembered App")).toBeTruthy();

    // The demo scan arrives and takes its place.
    expect(await screen.findByText("Visual Studio Code", {}, { timeout: 6000 })).toBeTruthy();
    await waitFor(() => expect(screen.queryByText("Remembered App")).toBeNull());
    // …and is remembered for next time.
    expect(localStorage.getItem("odysync.cache.scan")).toContain("Visual Studio Code");
  }, 15000);

  it("ignores a result written by another cache version or too long ago", () => {
    localStorage.setItem("odysync.cache.scan", remembered(0));
    const { unmount } = render(
      <StoreProvider>
        <Updates />
      </StoreProvider>,
    );
    expect(screen.queryByText("Remembered App")).toBeNull();
    unmount();

    localStorage.setItem("odysync.cache.scan", remembered(1, Date.now() - 30 * 24 * 3600 * 1000));
    render(
      <StoreProvider>
        <Updates />
      </StoreProvider>,
    );
    expect(screen.queryByText("Remembered App")).toBeNull();
  });
});
