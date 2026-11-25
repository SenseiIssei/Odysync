/**
 * One load at a time, however many callers ask for it.
 *
 * Two different wishes end up here. `refresh` means "I want current data" and
 * joins a load that is already running. `reload` means "something changed just
 * now": a load that began earlier may have read the old state, so its answer
 * is thrown away and exactly one more load runs, no matter how many reloads
 * arrive in the meantime.
 */
export interface LoadHooks<T> {
  /**
   * Fetch the data. Call `begin` when the fetch really starts, which may be
   * after a wait in a queue: a reload before that point needs no second load.
   */
  load: (begin: () => void) => Promise<T>;
  /** Called for every request, also for one that joins a running load. */
  onRequest: (quiet: boolean) => void;
  onValue: (value: T) => void;
  onError: (error: unknown) => void;
  onEnd: () => void;
}

export interface LoadOptions {
  /** A background poll: nobody asked, so nothing should look busy. */
  quiet?: boolean;
}

export interface Coalesced {
  refresh: (options?: LoadOptions) => Promise<void>;
  reload: () => Promise<void>;
}

export function coalesce<T>(hooks: LoadHooks<T>): Coalesced {
  let inFlight: Promise<void> | null = null;
  let stale = false;
  const begin = () => {
    stale = false;
  };

  const run = async () => {
    for (;;) {
      stale = false;
      try {
        const value = await hooks.load(begin);
        if (stale) continue;
        hooks.onValue(value);
      } catch (e) {
        if (stale) continue;
        hooks.onError(e);
      }
      // Nothing is awaited between reading `stale` and here, so a reload can
      // never slip in unnoticed.
      inFlight = null;
      hooks.onEnd();
      return;
    }
  };

  const request = (quiet: boolean) => {
    hooks.onRequest(quiet);
    inFlight ??= run();
    return inFlight;
  };

  return {
    refresh: (options) => request(options?.quiet ?? false),
    reload: () => {
      stale = inFlight !== null;
      return request(false);
    },
  };
}
