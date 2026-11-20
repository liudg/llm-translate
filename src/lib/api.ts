// Typed wrappers around the Rust commands and events.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Backend = "vulkan" | "cpu";
export type Direction = "zh2En" | "en2Zh";

export interface Settings {
  deployed: boolean;
  modelId: string | null;
  modelsDir: string;
  backend: Backend;
  device: string | null;
  threads: number;
  idleReleaseMinutes: number;
}

export interface GpuDevice {
  id: string;
  name: string;
  totalMb: number;
  freeMb: number;
  dedicatedMb: number | null;
  integrated: boolean;
}

export interface HardwareInfo {
  cpu: { name: string; physicalCores: number; logicalCores: number; avx2: boolean };
  ramTotalMb: number;
  ramAvailableMb: number;
  gpus: GpuDevice[];
  listDevicesError: string | null;
}

export interface Recommendation {
  modelId: string;
  backend: Backend;
  device: string | null;
  threads: number;
  reason: string;
  warnings: string[];
}

export interface DetectResult {
  hardware: HardwareInfo;
  recommendation: Recommendation;
  modelsDir: string;
}

export interface ModelView {
  id: string;
  name: string;
  file: string;
  size: number;
  minRamGB: number;
  recommendVramGB: number;
  installed: boolean;
  downloadedBytes: number;
}

export type DownloadProgress =
  | { phase: "probe"; source: string; speedBps: number | null; error: string | null }
  | { phase: "download"; source: string; done: number; total: number; speedBps: number }
  | { phase: "retry"; source: string; message: string; waitSecs: number }
  | { phase: "verify"; done: number; total: number };

export type DownloadEvent = DownloadProgress & { modelId: string };

export type DownloadErrorKind = "paused" | "cancelled" | "noSource" | "network" | "diskSpace" | "checksum" | "io";
export interface DownloadError {
  kind: DownloadErrorKind;
  message?: string;
}

export interface LaunchEvent {
  phase: "starting" | "selftest" | "fallback";
  backend: Backend;
  message: string | null;
}

export interface LaunchOutcome {
  backend: Backend;
  fellBack: boolean;
  fallbackReason: string | null;
  sampleSource: string;
  sample: string;
  loadMs: number;
  translateMs: number;
}

export type InferenceStatus =
  | { state: "unconfigured" }
  | { state: "stopped" }
  | { state: "loading"; backend: Backend; modelId: string }
  | { state: "ready"; backend: Backend; modelId: string; port: number }
  | { state: "idle" }
  | { state: "failed"; message: string };

export type InferenceNotice = { kind: "fellBackToCpu"; reason: string };

export type TranslateEvent =
  | { type: "start"; direction: Direction }
  | { type: "delta"; text: string }
  | { type: "done"; elapsedMs: number }
  | { type: "error"; message: string }
  | { type: "cancelled" };

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  inferenceStatus: () => invoke<InferenceStatus>("inference_status"),
  detect: () => invoke<DetectResult>("deploy_detect"),
  models: (dir: string) => invoke<ModelView[]>("deploy_models", { dir }),
  diskFree: (dir: string) => invoke<number | null>("disk_free_bytes", { dir }),
  download: (modelId: string, dir: string) => invoke<string>("download_model", { modelId, dir }),
  downloadControl: (modelId: string, action: "pause" | "cancel") =>
    invoke<void>("download_control", { modelId, action }),
  launch: (args: { modelId: string; dir: string; backend: Backend; device: string | null; threads: number }) =>
    invoke<LaunchOutcome>("deploy_launch", args),
  finish: () => invoke<void>("deploy_finish"),
  openWizard: () => invoke<void>("open_wizard"),
  openLogsDir: () => invoke<void>("open_logs_dir"),
  cancelTranslate: () => invoke<void>("translate_cancel"),
  translate: (text: string, direction: Direction | null, onEvent: (e: TranslateEvent) => void) => {
    const ch = new Channel<TranslateEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("translate", { req: { text, direction }, onEvent: ch });
  },
};

export const events = {
  download: (cb: (e: DownloadEvent) => void): Promise<UnlistenFn> =>
    listen<DownloadEvent>("download-progress", (e) => cb(e.payload)),
  launch: (cb: (e: LaunchEvent) => void) => listen<LaunchEvent>("deploy-launch", (e) => cb(e.payload)),
  status: (cb: (s: InferenceStatus) => void) => listen<InferenceStatus>("inference-status", (e) => cb(e.payload)),
  notice: (cb: (n: InferenceNotice) => void) => listen<InferenceNotice>("inference-notice", (e) => cb(e.payload)),
};

/** Tauri rejects with the serialized error value; normalize to text. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object") {
    const o = e as { message?: string; kind?: string };
    if (o.message) return o.message;
    if (o.kind) return o.kind;
  }
  return String(e);
}
