// Mini window (DESIGN 5.4): Alt+Q editorial command capsule. Enter translates,
// Shift+Enter inserts a newline, Ctrl+Enter / Ctrl+Shift+C copies & hides,
// Ctrl+D cycles direction, Esc hides. Window height tracks content automatically.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PhysicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowUpRight, CornerDownLeft, RotateCw, Square, X } from "lucide-react";
import { api, events, type OpenTranslatePayload, type TtsLang } from "../../lib/api";
import {
  NEXT_CHOICE,
  footerText,
  isImeComposing,
  useTranslator,
  type DirChoice,
  type TranslatorState,
} from "../../lib/useTranslator";
import { useAutoHeight } from "../../lib/useAutoHeight";
import { useStreamAutoScroll } from "../../lib/useStreamAutoScroll";
import { useTts } from "../../lib/useTts";
import { BrandSeal, IconButton, Kbd } from "../../components/ui";
import { SpeakButton } from "../../components/tts";
import { CopyResultButton, DirCycleButton, TranslationBody } from "../../components/translation";
import { useHistory } from "../../stores/history";
import { startInferenceSync, useInference } from "../../stores/inference";

const WIDTH = 640;
const MAX_INPUT_PX = 132; // ~5 lines

export default function Mini() {
  const win = getCurrentWindow();
  const [text, setText] = useState("");
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop, clear, drop } = useTranslator();
  const tts = useTts();
  const { stop: ttsStop, stopLocal: ttsStopLocal } = tts;
  const status = useInference((s) => s.status);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const wasBusy = useRef(false);

  const submit = useCallback(() => {
    if (text.trim()) run(text, choice);
  }, [text, choice, run]);

  const cycleChoice = useCallback(() => {
    const c = NEXT_CHOICE[choice];
    setChoice(c);
    if (text.trim()) run(text, c);
    inputRef.current?.focus();
  }, [choice, text, run]);

  const hideWindow = useCallback(() => {
    ttsStop();
    if (state.busy) clear();
    win.hide();
  }, [state.busy, clear, win, ttsStop]);

  const copyAndHide = useCallback(async () => {
    if (!state.result) return;
    ttsStop();
    try {
      await navigator.clipboard.writeText(state.result);
    } catch {
      // Ignore clipboard failure.
    }
    win.hide();
  }, [state.result, win, ttsStop]);

  const openInMain = useCallback(() => {
    const usingCurrentInput = Boolean(text.trim());
    const payloadText = usingCurrentInput ? text : state.source;
    if (!payloadText.trim()) return;
    const payloadChoice = usingCurrentInput ? choice : state.choice;
    const hasFinishedResult =
      !state.busy &&
      !state.error &&
      !state.stopped &&
      Boolean(state.result.trim()) &&
      payloadText === state.source &&
      payloadChoice === state.choice;
    const payload: OpenTranslatePayload = {
      text: payloadText,
      choice: payloadChoice,
      ...(hasFinishedResult
        ? {
            result: state.result,
            direction: state.direction,
            mixed: state.mixed,
            elapsedMs: state.elapsedMs,
          }
        : {}),
    };
    ttsStop();
    drop();
    api.openMainWithText(payload);
    win.hide();
  }, [text, choice, state, drop, win, ttsStop]);

  const handlersRef = useRef({
    copyAndHide,
    cycleChoice,
    hideWindow,
    drop,
    shouldDropOnHide: false,
  });
  handlersRef.current = {
    copyAndHide,
    cycleChoice,
    hideWindow,
    drop,
    shouldDropOnHide: state.busy || state.stopped === "preempted",
  };

  useEffect(() => {
    startInferenceSync();
    const unShown = listen("mini-shown", () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    const unHidden = events.miniHidden(() => {
      ttsStopLocal();
      if (handlersRef.current.shouldDropOnHide) {
        handlersRef.current.drop();
      }
    });
    const onKey = (e: KeyboardEvent) => {
      if (isImeComposing(e)) return;
      if (e.key === "Escape") {
        e.preventDefault();
        handlersRef.current.hideWindow();
      } else if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "c") {
        e.preventDefault();
        handlersRef.current.copyAndHide();
      } else if (e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        handlersRef.current.cycleChoice();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unShown.then((f) => f());
      unHidden.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Record finished clean translations into shared local history.
  useEffect(() => {
    if (
      wasBusy.current &&
      !state.busy &&
      state.result.trim() &&
      !state.error &&
      !state.stopped &&
      state.direction
    ) {
      useHistory.getState().record({
        source: state.source,
        result: state.result,
        direction: state.direction,
        mixed: state.mixed,
        elapsedMs: state.elapsedMs,
        origin: "mini",
      });
    }
    wasBusy.current = state.busy;
  }, [state]);

  // Grow the textarea with its content, up to MAX_INPUT_PX.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_PX)}px`;
  }, [text]);

  // Window height follows content in exact physical pixels (including Windows text scale).
  useAutoHeight(rootRef, (h) => {
    const dpr = window.devicePixelRatio;
    win.setSize(new PhysicalSize(Math.round(WIDTH * dpr), Math.ceil(h * dpr)));
  });

  const showResult = Boolean(state.busy || state.result || state.error || state.stopped);
  const sourceLang: TtsLang | null =
    choice === "zh2En"
      ? "zh"
      : choice === "en2Zh"
        ? "en"
        : !state.mixed && state.direction === "zh2En"
          ? "zh"
          : !state.mixed && state.direction === "en2Zh"
            ? "en"
            : null;
  const targetLang: TtsLang | null =
    choice === "zh2En"
      ? "en"
      : choice === "en2Zh"
        ? "zh"
        : !state.mixed && state.direction === "zh2En"
          ? "en"
          : !state.mixed && state.direction === "en2Zh"
            ? "zh"
            : null;

  const statusDot =
    status.state === "ready"
      ? "bg-[var(--good)]"
      : status.state === "loading"
        ? "bg-[var(--warn)] animate-pulse"
        : status.state === "failed"
          ? "bg-[var(--danger)]"
          : "bg-[var(--ink-muted)]";

  return (
    <div ref={rootRef} className="flyout-frame flex flex-col">
      {/* Top command capsule */}
      <div className="flex items-start gap-2.5 px-3.5 py-3">
        <div
          data-tauri-drag-region
          title="按住此处可拖动窗口"
          className="relative mt-[5px] flex size-7 shrink-0 cursor-move items-center justify-center"
        >
          <BrandSeal size="sm" className="pointer-events-none" />
          <span
            className={`pointer-events-none absolute -right-0.5 -bottom-0.5 size-2 rounded-full ring-2 ring-[var(--bg-surface)] ${statusDot}`}
          />
        </div>

        <textarea
          ref={inputRef}
          autoFocus
          rows={1}
          maxLength={20000}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (isImeComposing(e)) return;
            if (e.key === "Enter") {
              if (e.ctrlKey && !e.altKey && !e.metaKey) {
                e.preventDefault();
                if (state.result && text === state.source && !state.busy) {
                  copyAndHide();
                } else {
                  submit();
                }
              } else if (!e.shiftKey && !e.altKey && !e.metaKey) {
                e.preventDefault();
                submit();
              }
            }
          }}
          placeholder="输入中文或英文，回车即时翻译…"
          spellCheck={false}
          className="min-h-[36px] flex-1 resize-none bg-transparent py-[6px] text-large leading-6 text-[var(--ink)] outline-none placeholder:text-[var(--ink-muted)]"
          style={{ fontFamily: "var(--font-reading)" }}
        />

        <div className="mt-[4px] flex shrink-0 items-center gap-1">
          {text.trim() && (
            <SpeakButton
              size="sm"
              phase={tts.phaseOf("mini:source")}
              title="朗读原文"
              onClick={() => tts.toggle("mini:source", text, { lang: sourceLang })}
            />
          )}
          {text && (
            <IconButton
              size="sm"
              title="清空"
              onClick={() => {
                ttsStop();
                setText("");
                clear();
                inputRef.current?.focus();
              }}
            >
              <X className="size-3.5" />
            </IconButton>
          )}
          <DirCycleButton choice={choice} onClick={cycleChoice} />
          <button
            type="button"
            disabled={!text.trim()}
            onClick={submit}
            title="翻译（Enter）"
            className="inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md bg-[var(--accent)] text-[var(--ink-inverse)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:bg-[var(--bg-recessed)] disabled:text-[var(--ink-muted)]"
          >
            <CornerDownLeft className="size-3.5" />
          </button>
        </div>
      </div>

      {!showResult && tts.error && (
        <div className="border-t border-[var(--border-subtle)] bg-[var(--danger-soft)] px-4 py-1.5 text-caption text-[var(--danger)]">
          {tts.error}
        </div>
      )}

      {/* Expandable result sheet */}
      {showResult && (
        <MiniResultSheet
          state={state}
          targetLang={targetLang}
          canRetry={Boolean(text.trim())}
          onOpenInMain={openInMain}
          onStop={stop}
          onRetry={submit}
        />
      )}
    </div>
  );
}

function MiniResultSheet({
  state,
  targetLang,
  canRetry,
  onOpenInMain,
  onStop,
  onRetry,
}: {
  state: TranslatorState;
  targetLang: TtsLang | null;
  canRetry: boolean;
  onOpenInMain: () => void;
  onStop: () => void;
  onRetry: () => void;
}) {
  const resultRef = useRef<HTMLDivElement>(null);
  const tts = useTts();
  useStreamAutoScroll(resultRef, state.busy, state.result);

  return (
    <div className="flex flex-col bg-[var(--bg-flyout-result)]">
      {/* Living Ink Seam */}
      <div data-active={state.busy ? "true" : "false"} className="seam-beam-x h-px w-full" />

      <div
        ref={resultRef}
        className="selectable max-h-[320px] overflow-y-auto px-5 py-3.5 text-large leading-[1.74] whitespace-pre-wrap text-[var(--ink)]"
        style={{ fontFamily: "var(--font-reading)" }}
      >
        <TranslationBody state={state} />
      </div>

      <div className="flex h-10 items-center gap-1.5 border-t border-[var(--border-subtle)] px-3 text-caption">
        {tts.error ? (
          <span className="min-w-0 flex-1 truncate px-1 text-[var(--danger)]" title={tts.error}>
            {tts.error}
          </span>
        ) : (
          <span className="min-w-0 flex-1 truncate px-1 text-[var(--ink-muted)] tabular-nums">
            {footerText(state)}
          </span>
        )}

        {state.result && !state.busy && !tts.error && (
          <span className="hidden items-center gap-1 pr-1 text-caption text-[var(--ink-muted)] sm:inline-flex">
            <Kbd keys="Ctrl+Enter" compact />
            <span>复制并关闭</span>
          </span>
        )}

        <SpeakButton
          size="sm"
          phase={tts.phaseOf("mini:result")}
          disabled={!state.result.trim() || state.busy}
          title="朗读译文"
          onClick={() => tts.toggle("mini:result", state.result, { lang: targetLang })}
        />

        <IconButton size="sm" title="在主窗口展开" onClick={onOpenInMain}>
          <ArrowUpRight className="size-3.5" />
        </IconButton>

        {state.busy ? (
          <IconButton size="sm" title="停止" onClick={onStop}>
            <Square className="size-3.5" />
          </IconButton>
        ) : (
          <IconButton size="sm" title="重新翻译" disabled={!canRetry} onClick={onRetry}>
            <RotateCw className="size-3.5" />
          </IconButton>
        )}

        <CopyResultButton text={state.result} compact />
      </div>
    </div>
  );
}
