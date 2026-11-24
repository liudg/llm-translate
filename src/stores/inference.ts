import { create } from "zustand";
import { api, events, type InferenceNotice, type InferenceStatus } from "../lib/api";
import { follow, once } from "../lib/sync";

interface InferenceStore {
  status: InferenceStatus;
  notice: InferenceNotice | null;
  dismissNotice: () => void;
}

export const useInference = create<InferenceStore>((set) => ({
  status: { state: "stopped" },
  notice: null,
  dismissNotice: () => set({ notice: null }),
}));

/** Subscribe once per window to backend status events. */
export const startInferenceSync = once(() => {
  follow(events.status, api.inferenceStatus, (status) => useInference.setState({ status }));
  events.notice((notice) => useInference.setState({ notice }));
});
