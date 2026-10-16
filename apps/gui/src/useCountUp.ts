import { useEffect, useRef, useState } from "react";

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof requestAnimationFrame !== "undefined" &&
    !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * A number that counts up (or down) to its value instead of jumping. With
 * reduced motion, or where there is no animation frame (tests), it is simply
 * the value.
 */
export function useCountUp(target: number, durationMs = 900): number {
  const [value, setValue] = useState(target);
  // What is on screen right now, so a new target animates from there rather
  // than jumping back to where the previous animation started.
  const shown = useRef(target);
  const animate = typeof requestAnimationFrame !== "undefined" && !prefersReducedMotion();

  useEffect(() => {
    if (!animate) return;
    const start = shown.current;
    if (start === target) return;
    let frame = 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      const t = Math.min(1, (now - t0) / durationMs);
      const eased = 1 - Math.pow(1 - t, 3);
      const next = Math.round(start + (target - start) * eased);
      shown.current = next;
      setValue(next);
      if (t < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, durationMs, animate]);

  return animate ? value : target;
}
