import { create } from "zustand";
import { api, errorText, events, type UpdateProgress, type UpdateSnapshot } from "../lib/api";
import { bytes } from "../lib/format";
import { follow, once } from "../lib/sync";

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

/** Subscribe once per window to backend updater events. */
export const startUpdaterSync = once(() =>
  follow(events.updateState, api.updateStatus, (snapshot) => useUpdater.setState({ snapshot })),
);

/** Installer download: fraction done (0 when the size is unknown) and its text. */
export function downloadProgress(prog: UpdateProgress | null): { ratio: number; text: string } {
  if (!prog) return { ratio: 0, text: "" };
  if (!prog.total) return { ratio: 0, text: bytes(prog.downloaded) };
  const ratio = prog.downloaded / prog.total;
  return {
    ratio,
    text: `${Math.min(100, Math.round(ratio * 100))}%（${bytes(prog.downloaded)} / ${bytes(prog.total)}）`,
  };
}
