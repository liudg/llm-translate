import type { UnlistenFn } from "@tauri-apps/api/event";

/** Run `fn` the first time only (once per window). */
export function once(fn: () => void): () => void {
  let done = false;
  return () => {
    if (done) return;
    done = true;
    fn();
  };
}

/**
 * Follow a backend value: subscribe first, then fetch the current one; the
 * fetched snapshot only applies if no newer event came in meanwhile.
 */
export function follow<T>(
  listen: (cb: (v: T) => void) => Promise<UnlistenFn>,
  fetch: () => Promise<T>,
  apply: (v: T) => void,
) {
  let gotEvent = false;
  listen((v) => {
    gotEvent = true;
    apply(v);
  });
  fetch().then((v) => {
    if (!gotEvent) apply(v);
  });
}
