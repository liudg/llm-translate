// Streaming translation state shared by the main, mini and popup windows.
import { useCallback, useRef, useState } from "react";
import { api, type Direction } from "./api";

export type DirChoice = "auto" | Direction;

export const DIR_LABEL: Record<Direction, string> = { zh2En: "中文 → 英语", en2Zh: "英语 → 中文" };
export const DIR_SHORT: Record<DirChoice, string> = { auto: "自动", zh2En: "中 → 英", en2Zh: "英 → 中" };
/** Order of the direction toggle in the mini window and the popup. */
export const NEXT_CHOICE: Record<DirChoice, DirChoice> = { auto: "zh2En", zh2En: "en2Zh", en2Zh: "auto" };

/** Guard against WebView2 / Windows IME composition commits (including keyCode 229 / Process). */
export function isImeComposing(e: {
  nativeEvent?: { isComposing?: boolean };
  isComposing?: boolean;
  keyCode?: number;
  key?: string;
}): boolean {
  return Boolean(
    e.isComposing ||
      e.nativeEvent?.isComposing ||
      e.keyCode === 229 ||
      e.key === "Process",
  );
}

export interface TranslatorState {
  /** Text and direction of the latest request. */
  source: string;
  choice: DirChoice;
  result: string;
  busy: boolean;
  /** Overall direction the backend reported. */
  direction: Direction | null;
  /** Auto mode translated paragraphs in both directions. */
  mixed: boolean;
  elapsedMs: number | null;
  error: string | null;
  /** Ended early: by our stop button or by a request from another window. */
  stopped: "user" | "preempted" | null;
}

const EMPTY: TranslatorState = {
  source: "",
  choice: "auto",
  result: "",
  busy: false,
  direction: null,
  mixed: false,
  elapsedMs: null,
  error: null,
  stopped: null,
};

export function useTranslator() {
  const [state, setState] = useState<TranslatorState>(EMPTY);
  const reqId = useRef(0);
  const busyRef = useRef(false);
  const userStopped = useRef(false);

  const run = useCallback(async (source: string, choice: DirChoice) => {
    const id = ++reqId.current;
    busyRef.current = true;
    userStopped.current = false;
    setState({ ...EMPTY, source, choice, busy: true });
    const mine = (f: (s: TranslatorState) => TranslatorState) => {
      if (id === reqId.current) setState(f);
    };
    try {
      await api.translate(source, choice === "auto" ? null : choice, (e) => {
        switch (e.type) {
          case "start":
            mine((s) => ({ ...s, direction: e.direction, mixed: e.mixed }));
            break;
          case "delta":
            mine((s) => ({ ...s, result: s.result + e.text }));
            break;
          case "done":
            mine((s) => ({ ...s, elapsedMs: e.elapsedMs }));
            break;
          case "error":
            mine((s) => ({ ...s, error: e.message }));
            break;
          case "cancelled":
            mine((s) => ({ ...s, stopped: userStopped.current ? "user" : "preempted" }));
            break;
        }
      });
    } catch (e) {
      mine((s) => ({ ...s, error: String(e) }));
    }
    if (id === reqId.current) {
      busyRef.current = false;
      setState((s) => ({ ...s, busy: false }));
    }
  }, []);

  const stop = useCallback(() => {
    userStopped.current = true;
    api.cancelTranslate();
  }, []);

  /** Forget the current request (cancelling it if it is still running in this window). */
  const clear = useCallback(() => {
    if (reqId.current && busyRef.current) api.cancelTranslate();
    busyRef.current = false;
    reqId.current++;
    userStopped.current = false;
    setState(EMPTY);
  }, []);

  /** Abandon the local stream without calling backend cancel (used for handoff to Main). */
  const drop = useCallback(() => {
    busyRef.current = false;
    reqId.current++;
    userStopped.current = false;
    setState(EMPTY);
  }, []);

  /** Populate a completed translation directly without re-running inference. */
  const hydrate = useCallback(
    (init: {
      source: string;
      choice: DirChoice;
      result: string;
      direction: Direction | null;
      mixed: boolean;
      elapsedMs: number | null;
    }) => {
      busyRef.current = false;
      reqId.current++;
      userStopped.current = false;
      setState({
        source: init.source,
        choice: init.choice,
        result: init.result,
        busy: false,
        direction: init.direction,
        mixed: init.mixed,
        elapsedMs: init.elapsedMs,
        error: null,
        stopped: null,
      });
    },
    [],
  );

  return { state, run, stop, clear, drop, hydrate };
}

/** Footer label: the direction a request used and how long it took. */
export function footerText(s: TranslatorState): string {
  if (!s.direction) return "";
  const dir =
    s.choice !== "auto"
      ? DIR_LABEL[s.direction]
      : s.mixed
        ? "自动 · 中英混合，逐段翻译"
        : `自动 · ${DIR_LABEL[s.direction]}`;
  // Under 10 ms nothing was translated (copied through): no time.
  return s.elapsedMs !== null && s.elapsedMs >= 10 ? `${dir} · ${(s.elapsedMs / 1000).toFixed(2)} 秒` : dir;
}
