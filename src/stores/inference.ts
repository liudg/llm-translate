import { create } from "zustand";
import { api, events, type InferenceNotice, type InferenceStatus } from "../lib/api";

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

let started = false;

/** Subscribe once per window to backend status events. */
export function startInferenceSync() {
  if (started) return;
  started = true;
  api.inferenceStatus().then((status) => useInference.setState({ status }));
  events.status((status) => useInference.setState({ status }));
  events.notice((notice) => useInference.setState({ notice }));
}
