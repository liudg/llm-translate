// Window-local singleton Edge TTS audio player with chunked prefetch and
// cross-window mutual exclusion via `tts-stop` events.
import { api, errorText, events, type TtsLang } from "./api";

export type TtsPhase = "idle" | "loading" | "playing";

export interface TtsSnapshot {
  activeKey: string | null;
  phase: TtsPhase;
  error: string | null;
  errorKey: string | null;
}

export interface SpeakOptions {
  lang?: TtsLang | null;
  voice?: string | null;
  rate?: number | null;
}

const SHORT_THRESHOLD = 260;
const PACK_TARGET = 240;
const HARD_LIMIT = 320;
const ERROR_DISMISS_MS = 4500;

function isSentenceEnd(c: string): boolean {
  return "。！？；.!?;…".includes(c);
}

function isClosingPunct(c: string): boolean {
  return "”\"’'）)".includes(c);
}

function splitParagraph(para: string): string[] {
  if (para.length <= SHORT_THRESHOLD) return [para];

  const sentences: string[] = [];
  let cur = "";
  for (let i = 0; i < para.length; i++) {
    const c = para[i];
    cur += c;
    const isAsciiEnd = c === "." || c === "!" || c === "?" || c === ";";
    const next = para[i + 1];
    if (isSentenceEnd(c) && (!isAsciiEnd || next === undefined || /\s/.test(next))) {
      while (i + 1 < para.length && (/\s/.test(para[i + 1]) || isClosingPunct(para[i + 1]))) {
        cur += para[++i];
      }
      sentences.push(cur);
      cur = "";
    }
  }
  if (cur.trim()) sentences.push(cur);

  const chunks: string[] = [];
  let buf = "";
  const flush = () => {
    const trimmed = buf.trim();
    if (trimmed) chunks.push(trimmed);
    buf = "";
  };

  for (const s of sentences) {
    const piece = s.trim();
    if (!piece) continue;
    if (buf && buf.length + 1 + piece.length > PACK_TARGET) {
      flush();
    }
    if (piece.length > HARD_LIMIT) {
      flush();
      let rest = piece;
      while (rest.length > HARD_LIMIT) {
        let cut = -1;
        for (const sep of ["，", ",", "、", " "]) {
          const idx = rest.lastIndexOf(sep, PACK_TARGET);
          if (idx > 60) {
            cut = idx + 1;
            break;
          }
        }
        if (cut < 0) cut = PACK_TARGET;
        const head = rest.slice(0, cut).trim();
        if (head) chunks.push(head);
        rest = rest.slice(cut).trim();
      }
      if (rest) chunks.push(rest);
      continue;
    }
    buf = buf ? `${buf} ${piece}` : piece;
  }
  flush();
  return chunks;
}

/** Split text into natural chunks so long passages start playing quickly via prefetch. */
export function splitForTts(text: string): string[] {
  const normalized = text.replace(/\r\n/g, "\n").trim();
  if (!normalized) return [];
  if (normalized.length <= SHORT_THRESHOLD) return [normalized];

  const out: string[] = [];
  for (const line of normalized.split(/\n+/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    out.push(...splitParagraph(trimmed));
  }
  return out;
}

type FetchResult = { ok: true; data: ArrayBuffer } | { ok: false; error: unknown };

class TtsPlayer {
  private snapshot: TtsSnapshot = { activeKey: null, phase: "idle", error: null, errorKey: null };
  private listeners = new Set<() => void>();
  private audio: HTMLAudioElement | null = null;
  private currentUrl: string | null = null;
  private abortAudio: (() => void) | null = null;
  private sessionId: string | null = null;
  private sessionSeq = 0;
  private errorTimer: number | undefined;
  private listening = false;

  private ensureListener() {
    if (this.listening) return;
    this.listening = true;
    events.ttsStop((ev) => {
      if (ev.exceptSessionId && ev.exceptSessionId === this.sessionId) return;
      this.stopLocal();
    });
  }

  private emit(next: TtsSnapshot) {
    this.snapshot = next;
    for (const cb of this.listeners) cb();
  }

  private clearErrorTimer() {
    if (this.errorTimer !== undefined) {
      window.clearTimeout(this.errorTimer);
      this.errorTimer = undefined;
    }
  }

  subscribe = (cb: () => void): (() => void) => {
    this.ensureListener();
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  };

  getSnapshot = (): TtsSnapshot => this.snapshot;

  private cleanupAudio() {
    const abort = this.abortAudio;
    this.abortAudio = null;
    if (abort) abort();

    if (this.audio) {
      this.audio.pause();
      this.audio.removeAttribute("src");
      this.audio.load();
    }
    if (this.currentUrl) {
      URL.revokeObjectURL(this.currentUrl);
      this.currentUrl = null;
    }
  }

  stopLocal() {
    this.sessionId = null;
    this.cleanupAudio();
    this.clearErrorTimer();
    if (this.snapshot.activeKey !== null || this.snapshot.phase !== "idle" || this.snapshot.error !== null) {
      this.emit({ activeKey: null, phase: "idle", error: null, errorKey: null });
    }
  }

  stop() {
    const sid = this.sessionId;
    this.stopLocal();
    if (sid) {
      api.ttsStop(sid).catch(() => {});
    }
  }

  stopIfKeyPrefix(prefix: string) {
    if (this.snapshot.activeKey?.startsWith(prefix)) {
      this.stop();
    }
  }

  private playBuffer(buf: ArrayBuffer): Promise<boolean> {
    return new Promise((resolve, reject) => {
      if (!this.audio) {
        this.audio = new Audio();
      }
      const audio = this.audio;
      const blob = new Blob([buf], { type: "audio/mpeg" });
      const url = URL.createObjectURL(blob);
      this.currentUrl = url;

      let settled = false;
      const detach = () => {
        audio.onended = null;
        audio.onerror = null;
        this.abortAudio = null;
        if (this.currentUrl === url) {
          URL.revokeObjectURL(url);
          this.currentUrl = null;
        }
      };

      this.abortAudio = () => {
        if (settled) return;
        settled = true;
        detach();
        resolve(false);
      };

      audio.onended = () => {
        if (settled) return;
        settled = true;
        detach();
        resolve(true);
      };

      audio.onerror = () => {
        if (settled) return;
        settled = true;
        detach();
        reject(new Error("音频解码或播放失败"));
      };

      audio.src = url;
      audio.play().catch((err) => {
        if (settled) return;
        settled = true;
        detach();
        reject(err);
      });
    });
  }

  async play(key: string, text: string, opts: SpeakOptions = {}) {
    this.ensureListener();
    const chunks = splitForTts(text);
    if (chunks.length === 0) return;

    this.cleanupAudio();
    this.clearErrorTimer();

    const sid = `${Date.now()}-${++this.sessionSeq}-${Math.random().toString(36).slice(2, 8)}`;
    this.sessionId = sid;
    this.emit({ activeKey: key, phase: "loading", error: null, errorKey: null });

    const fetchChunk = (chunkText: string): Promise<FetchResult> =>
      api
        .ttsSynthesize({
          sessionId: sid,
          text: chunkText,
          lang: opts.lang ?? null,
          voice: opts.voice ?? null,
          rate: opts.rate ?? null,
        })
        .then(
          (data): FetchResult => ({ ok: true, data }),
          (error): FetchResult => ({ ok: false, error }),
        );

    let pending: Promise<FetchResult> = fetchChunk(chunks[0]);

    for (let i = 0; i < chunks.length; i++) {
      const res = await pending;
      if (this.sessionId !== sid) return;

      if (!res.ok) {
        const msg = errorText(res.error);
        if (msg.includes("已停止")) {
          this.stopLocal();
          return;
        }
        this.sessionId = null;
        this.cleanupAudio();
        this.emit({ activeKey: null, phase: "idle", error: msg, errorKey: key });
        this.clearErrorTimer();
        this.errorTimer = window.setTimeout(() => {
          if (this.snapshot.error === msg) {
            this.emit({ ...this.snapshot, error: null, errorKey: null });
          }
        }, ERROR_DISMISS_MS);
        return;
      }

      // Prefetch the next chunk while the current one plays.
      if (i + 1 < chunks.length) {
        pending = fetchChunk(chunks[i + 1]);
      }

      this.emit({ activeKey: key, phase: "playing", error: null, errorKey: null });
      try {
        const completed = await this.playBuffer(res.data);
        if (!completed || this.sessionId !== sid) return;
      } catch (e) {
        if (this.sessionId !== sid) return;
        const msg = errorText(e);
        this.sessionId = null;
        this.cleanupAudio();
        this.emit({ activeKey: null, phase: "idle", error: msg, errorKey: key });
        this.clearErrorTimer();
        this.errorTimer = window.setTimeout(() => {
          if (this.snapshot.error === msg) {
            this.emit({ ...this.snapshot, error: null, errorKey: null });
          }
        }, ERROR_DISMISS_MS);
        return;
      }

      // If the next chunk is still synthesizing after this one finishes, show loading.
      if (i + 1 < chunks.length && this.sessionId === sid) {
        this.emit({ activeKey: key, phase: "loading", error: null, errorKey: null });
      }
    }

    if (this.sessionId === sid) {
      this.sessionId = null;
      this.emit({ activeKey: null, phase: "idle", error: null, errorKey: null });
    }
  }

  toggle(key: string, text: string, opts: SpeakOptions = {}) {
    if (this.snapshot.activeKey === key && this.snapshot.phase !== "idle") {
      this.stop();
    } else {
      void this.play(key, text, opts);
    }
  }
}

export const ttsPlayer = new TtsPlayer();
