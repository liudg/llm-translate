// Mini window (DESIGN 5.4): Alt+Q editorial command capsule. Enter translates,
// Shift+Enter inserts a newline, Ctrl+Enter / Ctrl+Shift+C copies & hides,
// Ctrl+D cycles direction, Esc hides. Window height tracks content automatically.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PhysicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowUpRight, CornerDownLeft, RotateCw, Square, X } from "lucide-react";
import { api, events, type OpenTranslatePayload } from "../../lib/api";
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
import { BrandSeal, IconButton, Kbd } from "../../components/ui";
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
    if (state.busy) clear();
    win.hide();
  }, [state.busy, clear, win]);

  const copyAndHide = useCallback(async () => {
    if (!state.result) return;
    try {
      await navigator.clipboard.writeText(state.result);
    } catch {
      // Ignore clipboard failure.
    }
    win.hide();
  }, [state.result, win]);

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
    drop();
    api.openMainWithText(payload);
    win.hide();
  }, [text, choice, state, drop, win]);

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
          className="min-h-[36px] flex-1 resize-none bg-transparent py-[6px] text-[15.5px] leading-6 text-[var(--ink)] outline-none placeholder:text-[var(--ink-muted)]"
          style={{ fontFamily: "var(--font-reading)" }}
        />

        <div className="mt-[4px] flex shrink-0 items-center gap-1">
          {text && (
            <IconButton
              size="sm"
              title="清空"
              onClick={() => {
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
            className="inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-[7px] bg-[var(--accent)] text-[var(--ink-inverse)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:bg-[var(--bg-recessed)] disabled:text-[var(--ink-muted)]"
          >
            <CornerDownLeft className="size-3.5" />
          </button>
        </div>
      </div>

      {/* Expandable result sheet */}
      {showResult && (
        <MiniResultSheet
          state={state}
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
  canRetry,
  onOpenInMain,
  onStop,
  onRetry,
}: {
  state: TranslatorState;
  canRetry: boolean;
  onOpenInMain: () => void;
  onStop: () => void;
  onRetry: () => void;
}) {
  const resultRef = useRef<HTMLDivElement>(null);
  useStreamAutoScroll(resultRef, state.busy, state.result);

  return (
    <div className="flex flex-col bg-[var(--bg-flyout-result)]">
      {/* Living Ink Seam */}
      <div data-active={state.busy ? "true" : "false"} className="seam-beam-x h-px w-full" />

      <div
        ref={resultRef}
        className="selectable max-h-[320px] overflow-y-auto px-5 py-3.5 text-[15.5px] leading-[1.74] whitespace-pre-wrap text-[var(--ink)]"
        style={{ fontFamily: "var(--font-reading)" }}
      >
        <TranslationBody state={state} />
      </div>

      <div className="flex h-10 items-center gap-1.5 border-t border-[var(--border-subtle)] px-3 text-xs">
        <span className="min-w-0 flex-1 truncate px-1 text-[var(--ink-muted)] tabular-nums">
          {footerText(state)}
        </span>

        {state.result && !state.busy && (
          <span className="hidden items-center gap-1 pr-1 text-[11px] text-[var(--ink-muted)] sm:inline-flex">
            <Kbd keys="Ctrl+Enter" compact />
            <span>复制并关闭</span>
          </span>
        )}

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
