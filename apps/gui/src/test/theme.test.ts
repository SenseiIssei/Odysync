import { describe, expect, it } from "vitest";
import { applySavedTheme, resolve } from "../theme";
import { errorText, formatSize } from "../format";

describe("theme", () => {
  it("resolves an explicit choice to itself and system to a real theme", () => {
    expect(resolve("light")).toBe("light");
    expect(resolve("dark")).toBe("dark");
    expect(["light", "dark"]).toContain(resolve("system"));
  });

  it("applies the saved choice to <html> before the first render", () => {
    localStorage.setItem("odysync.theme", "light");
    applySavedTheme();
    expect(document.documentElement.dataset.theme).toBe("light");
    localStorage.setItem("odysync.theme", "dark");
    applySavedTheme();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("ignores a stored value it does not know", () => {
    localStorage.setItem("odysync.theme", "neon");
    applySavedTheme();
    expect(["light", "dark"]).toContain(document.documentElement.dataset.theme);
  });
});

describe("format", () => {
  it("formats sizes the way people read them", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatSize(3.4 * 1024 * 1024 * 1024)).toBe("3.4 GB");
    expect(formatSize(-1)).toBe("-");
  });

  it("turns any thrown value into readable text", () => {
    expect(errorText(new Error("boom"))).toBe("boom");
    expect(errorText("plain")).toBe("plain");
    expect(errorText({ code: 5 })).toBe('{"code":5}');
  });
});
