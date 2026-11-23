// Typed wrappers around the Rust commands and events.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Backend = "vulkan" | "cpu";
export type Direction = "zh2En" | "en2Zh";
export type Theme = "system" | "light" | "dark";

export interface Settings {
  deployed: boolean;
  modelId: string | null;
  modelsDir: string;
  backend: Backend;
  device: string | null;
  threads: number;
  idleReleaseMinutes: number;
  theme: Theme;
  selection: SelectionSettings;
  hotkeys: HotkeySettings;
  trayHintShown: boolean;
  ignoredUpdateVersion: string | null;
}

/** Display strings such as `Ctrl+Alt+Q`; null when unbound. */
export interface HotkeySettings {
  mini: string | null;
  shot: string | null;
  selection: string | null;
}

export interface SelectionSettings {
  enabled: boolean;
  drag: boolean;
  doubleClick: boolean;
  shiftClick: boolean;
  /** Process names such as `WINWORD.EXE`. */
  blacklist: string[];
}

export interface WindowProcess {
  exe: string;
  title: string;
}

/** One click on the selection icon. `text` is null when capture failed. */
export interface SelectionPayload {
  id: number;
  text: string | null;
  error: string | null;
}

export interface AppInfo {
  version: string;
  osBuild: number;
  effect: string;
  configDir: string;
  logsDir: string;
}

export interface AvailableUpdate {
  version: string;
  currentVersion: string;
  notes: string | null;
  pubDate: string | null;
}

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

export type UpdatePhase = "idle" | "checking" | "upToDate" | "available" | "downloading" | "installing" | "error";

export interface UpdateSnapshot {
  phase: UpdatePhase;
  manual: boolean;
  available: AvailableUpdate | null;
  progress: UpdateProgress | null;
  error: string | null;
}

export type HotkeyId = "mini" | "shot" | "selection";

export interface HotkeyInfo {
  id: HotkeyId;
  /** Null when unbound. */
  keys: string | null;
  registered: boolean;
  error: string | null;
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
  | { type: "start"; direction: Direction; mixed: boolean }
  | { type: "delta"; text: string }
  | { type: "done"; elapsedMs: number }
  | { type: "error"; message: string }
  | { type: "cancelled" };

/** A new screenshot for the result window; sizes are physical pixels. */
export interface ShotPayload {
  id: number;
  width: number;
  height: number;
  imageX: number;
  imageY: number;
  toolbar: "below" | "above" | "inside";
  scale: number;
}

/** An OCR paragraph; geometry in image pixels, colours as [r, g, b]. */
export interface ShotParagraph {
  rect: [number, number, number, number];
  lineHeight: number;
  linePitch: number;
  lines: number;
  text: string;
  translatable: boolean;
  bg: [number, number, number];
  fg: [number, number, number];
  /** Width and height (from the rect origin) the translation may grow into. */
  room: [number, number];
}

export type BatchEvent =
  | { type: "start" }
  | { type: "delta"; index: number; text: string }
  | { type: "itemDone"; index: number }
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
  finish: (autostart: boolean | null) => invoke<void>("deploy_finish", { autostart }),
  modelSwitch: (modelId: string) => invoke<LaunchOutcome>("model_switch", { modelId }),
  modelDelete: (modelId: string) => invoke<void>("model_delete", { modelId }),
  autostart: () => invoke<boolean>("autostart_enabled"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  hotkeyPause: (paused: boolean) => invoke<void>("hotkey_pause", { paused }),
  hotkeySet: (id: HotkeyId, keys: string | null) => invoke<void>("hotkey_set", { id, keys }),
  openWizard: () => invoke<void>("open_wizard"),
  openLogsDir: () => invoke<void>("open_logs_dir"),
  openModelsDir: () => invoke<void>("open_models_dir"),
  openMini: () => invoke<void>("open_mini"),
  appInfo: () => invoke<AppInfo>("app_info"),
  updateStatus: () => invoke<UpdateSnapshot>("update_status"),
  updateCheck: () => invoke<UpdateSnapshot>("update_check"),
  updateInstall: () => invoke<void>("update_install"),
  updateIgnore: (version: string) => invoke<void>("update_ignore", { version }),
  hotkeys: () => invoke<HotkeyInfo[]>("hotkey_status"),
  hardware: () => invoke<HardwareInfo>("hardware_info"),
  setTheme: (theme: Theme) => invoke<void>("set_theme", { theme }),
  setIdleRelease: (minutes: number) => invoke<void>("set_idle_release", { minutes }),
  setInference: (backend: Backend, device: string | null, threads: number) =>
    invoke<void>("set_inference", { backend, device, threads }),
  setSelection: (selection: SelectionSettings) => invoke<void>("set_selection", { selection }),
  windowProcesses: () => invoke<WindowProcess[]>("window_processes"),
  popupSetHeight: (height: number) => invoke<void>("popup_set_height", { height }),
  popupSetPinned: (pinned: boolean) => invoke<void>("popup_set_pinned", { pinned }),
  popupClose: () => invoke<void>("popup_close"),
  popupCurrent: () => invoke<SelectionPayload | null>("popup_current"),
  cancelTranslate: () => invoke<void>("translate_cancel"),
  translateBatch: (items: string[], direction: Direction | null, onEvent: (e: BatchEvent) => void) => {
    const ch = new Channel<BatchEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("translate_batch", { req: { items, direction }, onEvent: ch });
  },
  shotStart: () => invoke<void>("shot_start"),
  shotCurrent: () => invoke<ShotPayload | null>("shot_current"),
  /** Raw RGBA bytes of the screenshot. */
  shotImage: (id: number) => invoke<ArrayBuffer>("shot_image", { id }),
  shotOcr: (id: number) => invoke<ShotParagraph[]>("shot_ocr", { id }),
  shotShown: (id: number) => invoke<void>("shot_shown", { id }),
  shotClose: () => invoke<void>("shot_close"),
  shotCopyImage: (rgba: Uint8Array, width: number, height: number) =>
    invoke<void>("shot_copy_image", rgba, { headers: { "x-width": String(width), "x-height": String(height) } }),
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
  /** The wizard closed: downloaded / partial files may have changed. */
  modelsChanged: (cb: () => void) => listen("models-changed", () => cb()),
  status: (cb: (s: InferenceStatus) => void) => listen<InferenceStatus>("inference-status", (e) => cb(e.payload)),
  notice: (cb: (n: InferenceNotice) => void) => listen<InferenceNotice>("inference-notice", (e) => cb(e.payload)),
  settings: (cb: (s: Settings) => void) => listen<Settings>("settings-changed", (e) => cb(e.payload)),
  selection: (cb: (p: SelectionPayload) => void) =>
    listen<SelectionPayload>("selection-captured", (e) => cb(e.payload)),
  popupHidden: (cb: () => void) => listen("popup-hidden", () => cb()),
  shotNew: (cb: (p: ShotPayload) => void) => listen<ShotPayload>("shot-new", (e) => cb(e.payload)),
  shotHidden: (cb: () => void) => listen("shot-hidden", () => cb()),
  hotkeys: (cb: (h: HotkeyInfo[]) => void) => listen<HotkeyInfo[]>("hotkeys-changed", (e) => cb(e.payload)),
  /** Tray "settings": the main window switches page. */
  navigate: (cb: (page: string) => void) => listen<string>("navigate", (e) => cb(e.payload)),
  updateState: (cb: (s: UpdateSnapshot) => void) => listen<UpdateSnapshot>("update-state", (e) => cb(e.payload)),
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
