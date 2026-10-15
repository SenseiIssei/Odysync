/**
 * Light, dark, or whatever the operating system prefers.
 *
 * The choice lives in localStorage — it is a per-machine display preference,
 * not part of the update configuration — and is applied as `data-theme` on
 * `<html>`, which is all the stylesheet keys off.
 */
import { useCallback, useEffect, useState } from "react";

export type ThemeChoice = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const KEY = "odysync.theme";

function readChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

function systemTheme(): ResolvedTheme {
  return typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

export function resolve(choice: ThemeChoice): ResolvedTheme {
  return choice === "system" ? systemTheme() : choice;
}

export function applyTheme(theme: ResolvedTheme) {
  document.documentElement.dataset.theme = theme;
}

/** Apply the saved theme before React renders, so there is no flash. */
export function applySavedTheme() {
  applyTheme(resolve(readChoice()));
}

export function useTheme() {
  const [choice, setChoiceState] = useState<ThemeChoice>(readChoice);
  const [resolved, setResolved] = useState<ResolvedTheme>(() => resolve(readChoice()));

  useEffect(() => {
    const apply = () => {
      const next = resolve(choice);
      setResolved(next);
      applyTheme(next);
    };
    apply();
    if (choice !== "system") return;
    const media = window.matchMedia?.("(prefers-color-scheme: light)");
    media?.addEventListener("change", apply);
    return () => media?.removeEventListener("change", apply);
  }, [choice]);

  const setChoice = useCallback((next: ThemeChoice) => {
    try {
      localStorage.setItem(KEY, next);
    } catch {
      // Private mode or blocked storage: the choice still applies this session.
    }
    setChoiceState(next);
  }, []);

  return { choice, resolved, setChoice };
}
