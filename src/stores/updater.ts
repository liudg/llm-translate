import { create } from "zustand";
import { api, errorText, events, type UpdateSnapshot } from "../lib/api";

const INITIAL: UpdateSnapshot = {
  phase: "idle",
  manual: false,
  available: null,
  progress: null,
  error: null,
};

interface UpdaterStore {
  snapshot: UpdateSnapshot;
  /** Version whose banner was temporarily closed during this session. */
  dismissedVersion: string | null;
  check: () => Promise<void>;
  install: () => Promise<void>;
  ignore: (version: string) => Promise<void>;
  dismissBanner: (version: string) => void;
}

export const useUpdater = create<UpdaterStore>((set) => ({
  snapshot: INITIAL,
  dismissedVersion: null,
  check: async () => {
    try {
      const snapshot = await api.updateCheck();
      set({ snapshot });
    } catch (e) {
      set((s) => ({
        snapshot: {
          ...s.snapshot,
          phase: "error",
          manual: true,
          error: errorText(e),
        },
      }));
    }
  },
  install: async () => {
    // Re-open banner if the user started the update from Settings.
    set({ dismissedVersion: null });
    try {
      await api.updateInstall();
    } catch (e) {
      // Backend already emits an error snapshot; ensure we don't leave an unhandled rejection.
      set((s) =>
        s.snapshot.phase === "error"
          ? s
          : { snapshot: { ...s.snapshot, phase: "error", manual: true, error: errorText(e) } },
      );
    }
  },
  ignore: async (version: string) => {
    set({ dismissedVersion: version });
    try {
      await api.updateIgnore(version);
    } catch {
      // Non-fatal: already dismissed for this session.
    }
  },
  dismissBanner: (version: string) => set({ dismissedVersion: version }),
}));

let started = false;

/** Subscribe once per window to backend updater events. */
export function startUpdaterSync() {
  if (started) return;
  started = true;
  // Subscribe first; the snapshot only applies if no newer event came in.
  let gotEvent = false;
  events.updateState((snapshot) => {
    gotEvent = true;
    useUpdater.setState({ snapshot });
  });
  api.updateStatus().then((snapshot) => {
    if (!gotEvent) useUpdater.setState({ snapshot });
  });
}
