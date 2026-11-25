import { listen, type EventCallback } from "@tauri-apps/api/event";
import { inTauri } from "./bridge";

/**
 * Outside Tauri (the browser demo, the component tests) events come from
 * `emitLocal` instead of the Rust side.
 */
const local = new Map<string, Set<EventCallback<unknown>>>();

function listenLocally<T>(event: string, handler: EventCallback<T>): () => void {
  const handlers = local.get(event) ?? new Set();
  local.set(event, handlers);
  handlers.add(handler as EventCallback<unknown>);
  return () => {
    handlers.delete(handler as EventCallback<unknown>);
  };
}

export function emitLocal(event: string, payload: unknown): void {
  for (const handler of local.get(event) ?? []) handler({ event, id: 0, payload });
}

/**
 * Subscribe to a Tauri event, returning a synchronous cleanup function.
 *
 * `listen` rejects when there is no Tauri IPC (a plain browser, or before the
 * webview finishes wiring up). Left unguarded that surfaces as an unhandled
 * rejection and, in `useEffect`, a cleanup that never runs.
 */
export function safeListen<T>(event: string, handler: EventCallback<T>): () => void {
  if (!inTauri) return listenLocally(event, handler);
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
