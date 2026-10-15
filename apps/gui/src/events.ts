import { listen, type EventCallback } from "@tauri-apps/api/event";
import { inTauri } from "./bridge";

/**
 * Subscribe to a Tauri event, returning a synchronous cleanup function.
 *
 * `listen` rejects when there is no Tauri IPC (a plain browser, or before the
 * webview finishes wiring up). Left unguarded that surfaces as an unhandled
 * rejection and, in `useEffect`, a cleanup that never runs.
 */
export function safeListen<T>(event: string, handler: EventCallback<T>): () => void {
  // Outside Tauri (the browser demo, tests) there is nothing to listen to.
  if (!inTauri) return () => {};
  let unlisten: (() => void) | null = null;
  let cancelled = false;

  listen<T>(event, handler)
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch((e) => {
      console.warn(`[odysync] could not subscribe to "${event}":`, e);
    });

  return () => {
    cancelled = true;
    unlisten?.();
    unlisten = null;
  };
}
