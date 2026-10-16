import type { ReactNode } from "react";
import { useCountUp } from "../useCountUp";

export type RingTone = "success" | "accent" | "warning" | "danger";
export type RingMode = "idle" | "busy" | "done";

const STOPS: Record<RingTone, [string, string]> = {
  success: ["var(--success)", "var(--accent-2)"],
  accent: ["var(--accent)", "var(--accent-2)"],
  warning: ["var(--warm)", "var(--danger)"],
  danger: ["var(--danger)", "var(--warm)"],
};

const SIZE = 260;
const R = 104;
const C = 2 * Math.PI * R;

/**
 * The care ring: the score while idle, a spinning sweep while Odysync looks
 * or fixes, and a ripple when it is done.
 *
 * `value` is 0–100 — the score, or the progress of the running job.
 */
export function CareRing({
  mode,
  value,
  tone,
  label,
  caption,
  center,
}: {
  mode: RingMode;
  value: number;
  tone: RingTone;
  /** Under the number, e.g. "Excellent" or "Checking updates". */
  label: string;
  /** Small text under the label. */
  caption?: string;
  /** Replaces the number, e.g. with an icon. */
  center?: ReactNode;
}) {
  const clamped = Math.max(0, Math.min(100, value));
  const shown = useCountUp(Math.round(clamped));
  const [from, to] = STOPS[tone];
  const gradientId = `ring-${tone}`;

  return (
    <div
      className="relative flex-shrink-0"
      style={{ width: SIZE, height: SIZE }}
      role="img"
      aria-label={`${label}${mode === "idle" ? `: ${Math.round(clamped)} of 100` : ""}`}
    >
      {/* The light behind the ring. */}
      <div
        aria-hidden
        className="absolute inset-6 rounded-full animate-breathe blur-2xl"
        style={{ background: `radial-gradient(circle, color-mix(in oklab, ${from} 45%, transparent), transparent 70%)` }}
      />

      <svg
        aria-hidden
        className="absolute inset-0"
        width={SIZE}
        height={SIZE}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
      >
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" style={{ stopColor: from }} />
            <stop offset="100%" style={{ stopColor: to }} />
          </linearGradient>
          <linearGradient id={`${gradientId}-comet`} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" style={{ stopColor: to, stopOpacity: 0 }} />
            <stop offset="100%" style={{ stopColor: from, stopOpacity: 1 }} />
          </linearGradient>
        </defs>

        {/* Ticks around the outside, turning slowly — faster while working. */}
        <g
          className={mode === "busy" ? "animate-spin-fast" : "animate-spin-slow"}
          style={{
            transformBox: "fill-box",
            transformOrigin: "center",
            animationDuration: mode === "busy" ? "6s" : undefined,
          }}
        >
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={124}
            fill="none"
            stroke="var(--line-strong)"
            strokeWidth={2}
            strokeDasharray="2 9"
            strokeLinecap="round"
          />
        </g>

        {/* Track. */}
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={R}
          fill="none"
          stroke="var(--raised)"
          strokeWidth={14}
        />

        {/* Value. */}
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={R}
          fill="none"
          stroke={`url(#${gradientId})`}
          strokeWidth={14}
          strokeLinecap="round"
          strokeDasharray={C}
          strokeDashoffset={C * (1 - clamped / 100)}
          transform={`rotate(-90 ${SIZE / 2} ${SIZE / 2})`}
          style={{
            transition: "stroke-dashoffset 1.1s cubic-bezier(0.2, 0.7, 0.2, 1)",
            filter: `drop-shadow(0 0 10px color-mix(in oklab, ${from} 55%, transparent))`,
          }}
        />

        {/* The sweep that says "working". */}
        {mode === "busy" && (
          <g
            className="animate-spin-fast"
            style={{ transformBox: "fill-box", transformOrigin: "center" }}
          >
            <circle
              cx={SIZE / 2}
              cy={SIZE / 2}
              r={R}
              fill="none"
              stroke={`url(#${gradientId}-comet)`}
              strokeWidth={14}
              strokeLinecap="round"
              strokeDasharray={`${C * 0.22} ${C}`}
              opacity={0.9}
            />
          </g>
        )}

        {mode === "done" && (
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={R}
            fill="none"
            stroke={from}
            strokeWidth={3}
            className="animate-ripple"
            style={{ transformBox: "fill-box", transformOrigin: "center" }}
          />
        )}
      </svg>

      <div className="absolute inset-0 flex flex-col items-center justify-center text-center px-10">
        {center ?? (
          <div className="font-display text-[60px] leading-none font-bold tabular-nums text-gradient">
            {shown}
            {mode === "busy" && <span className="text-[26px] ml-0.5 align-top">%</span>}
          </div>
        )}
        <div className="mt-2 text-[15px] font-semibold text-fg font-display">{label}</div>
        {caption && <div className="mt-0.5 text-xs text-muted max-w-[150px] truncate">{caption}</div>}
      </div>
    </div>
  );
}
