/**
 * The one place the front-end talks to the Rust side.
 *
 * Inside the Tauri window every call goes to the real command. In a plain
 * browser — `npm run dev` opened outside Tauri, the README screenshots, the
 * component tests — there is no IPC, so calls are answered by a demo data set
 * instead. The demo module is imported lazily, which keeps it out of the
 * bundle the app actually runs.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export const inTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(command, args);
  const { demoInvoke } = await import("./demo/demo");
  return demoInvoke<T>(command, args);
}
