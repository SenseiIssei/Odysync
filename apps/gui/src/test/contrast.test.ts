/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * The colour tokens in index.css, measured against WCAG 2.2 AA: 4.5:1 for
 * text, 3:1 for the outline of a control.
 *
 * Translucent colours are composited the way the browser does it. The aurora
 * behind the window is counted at full strength, which is brighter (dark
 * theme) or darker (light theme) than it ever gets, so the real ratios are a
 * little better than the ones asserted here.
 */

// Read from disk: vitest hands out an empty module for a stylesheet import.
const css = readFileSync("src/index.css", "utf8");

/** Every component's source, for the class name checks at the end. */
const sources = import.meta.glob<string>(["../**/*.tsx", "!../test/**"], {
  query: "?raw",
  import: "default",
  eager: true,
});

type Rgba = [number, number, number, number];

function parse(value: string): Rgba | null {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const rgb = /^rgb\((\d+) (\d+) (\d+) \/ ([\d.]+)\)$/.exec(value);
  return rgb ? [Number(rgb[1]), Number(rgb[2]), Number(rgb[3]), Number(rgb[4])] : null;
}

function tokens(theme: "dark" | "light"): Record<string, Rgba> {
  const at = css.indexOf(`:root[data-theme="${theme}"] {`);
  const body = css.slice(css.indexOf("{", at) + 1, css.indexOf("}", at));
  const out: Record<string, Rgba> = {};
  for (const m of body.matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    const colour = parse(m[2].trim());
    if (colour) out[m[1]] = colour;
  }
  return out;
}

/** `top` at `alpha` over an opaque `bottom`. */
function over(top: Rgba, bottom: Rgba, alpha = top[3]): Rgba {
  const mix = (i: number) => top[i] * alpha + bottom[i] * (1 - alpha);
  return [mix(0), mix(1), mix(2), 1];
}

function luminance(c: Rgba): number {
  const [r, g, b] = c.slice(0, 3).map((x) => {
    const v = x / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function ratio(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Everything text can sit on. */
function backgrounds(t: Record<string, Rgba>): Record<string, Rgba> {
  const litViolet = over(t["glow-1"], t.canvas);
  const litBlue = over(t["glow-2"], t.canvas);
  const glass = over(t.glass, litViolet);
  return {
    canvas: t.canvas,
    surface: t.surface,
    raised: t.raised,
    "canvas with the violet glow": litViolet,
    "canvas with the blue glow": litBlue,
    glass,
    "hovered row": over(t.raised, glass, 0.5),
  };
}

const TONES = ["accent", "success", "warning", "danger", "info"] as const;
/** The strongest tint a tone is used at behind its own text. */
const MAX_TINT = 0.12;

describe.each(["dark", "light"] as const)("contrast, %s theme", (theme) => {
  const t = tokens(theme);
  const surfaces = Object.entries(backgrounds(t));

  it("reads the tokens it measures", () => {
    for (const name of ["canvas", "fg", "muted", "faint", "control", "accent-fg", "accent-2", ...TONES]) {
      expect(t[name], name).toBeDefined();
    }
  });

  it.each(["fg", "muted", "faint", ...TONES])("%s text reaches 4.5:1 on every surface", (name) => {
    for (const [surface, bg] of surfaces) {
      expect(ratio(t[name], bg), `${name} on ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each(TONES)("%s text reaches 4.5:1 on its own tint", (name) => {
    for (const [surface, bg] of surfaces) {
      for (const tint of [0.1, MAX_TINT]) {
        expect(
          ratio(t[name], over(t[name], bg, tint)),
          `${name} on a ${tint} tint over ${surface}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("button text reaches 4.5:1 across the whole primary gradient", () => {
    for (const p of [0, 0.25, 0.5, 0.75, 1]) {
      const stop = over(t["accent-2"], t.accent, p);
      expect(ratio(t["accent-fg"], stop), `at ${p}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("control outlines reach 3:1 inside and out", () => {
    for (const [surface, bg] of surfaces) {
      expect(ratio(t.control, bg), `outline on ${surface}`).toBeGreaterThanOrEqual(3);
      // A switch that is on has no outline: the gradient has to carry it.
      expect(ratio(t.accent, bg), `switch that is on, over ${surface}`).toBeGreaterThanOrEqual(3);
      expect(ratio(t["accent-2"], bg), `its far end, over ${surface}`).toBeGreaterThanOrEqual(3);
    }
    // The thumb of a switch that is off.
    expect(ratio(t.faint, t.raised)).toBeGreaterThanOrEqual(3);
  });
});

describe("class names", () => {
  it("use no tint stronger than the one measured above", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
    for (const [file, text] of Object.entries(sources)) {
      for (const m of text.matchAll(/\bbg-(?:accent|success|warning|danger|info)\/(\d+)/g)) {
        expect(Number(m[1]) / 100, `${m[0]} in ${file}`).toBeLessThanOrEqual(MAX_TINT);
      }
    }
  });

  it("outline inputs with the control token and put no white text on the gradient", () => {
    for (const [file, text] of Object.entries(sources)) {
      for (const m of text.matchAll(/<(?:input|select|textarea)\b[^]*?className="([^"]*)"/g)) {
        expect(m[1], `in ${file}`).not.toMatch(/\bborder-line(?![-a-z])/);
      }
      expect(text, `in ${file}`).not.toMatch(/bg-gradient-accent text-white/);
    }
  });
});
