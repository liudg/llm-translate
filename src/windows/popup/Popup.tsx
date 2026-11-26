// Selection popup (DESIGN 5.5): floating editorial dictionary card next to
// selected text. Hides on focus loss unless pinned; height tracks content.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowUpRight,
  Check,
  ChevronDown,
  Copy,
  Pin,
  PinOff,
  RotateCcw,
  ScanText,
  Square,
  X,
} from "lucide-react";
import { api, events, type OpenTranslatePayload, type SelectionPayload, type TtsLang } from "../../lib/api";
import { NEXT_CHOICE, footerText, isImeComposing, useTranslator, type DirChoice } from "../../lib/useTranslator";
import { useAutoHeight } from "../../lib/useAutoHeight";
import { useCopy } from "../../lib/useCopy";
import { useStreamAutoScroll } from "../../lib/useStreamAutoScroll";
import { useTts } from "../../lib/useTts";
import { BrandSeal, Button, IconButton } from "../../components/ui";
import { SpeakButton } from "../../components/tts";
import { CopyResultButton, DirCycleButton, TranslationBody } from "../../components/translation";
import { useHistory } from "../../stores/history";
import { startInferenceSync } from "../../stores/inference";
import { useSettings } from "../../stores/settings";

export default function Popup() {
  const [payload, setPayload] = useState<SelectionPayload | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop, clear, drop } = useTranslator();
  const tts = useTts();
  const { play: ttsPlay, stop: ttsStop, stopLocal: ttsStopLocal } = tts;
  const sourceCopy = useCopy();
  const rootRef = useRef<HTMLDivElement>(null);
  const resultRef = useRef<HTMLDivElement>(null);
  const seen = useRef(0);
  const wasBusy = useRef(false);
  const handedOffRef = useRef(false);
  const clearRef = useRef(clear);
  clearRef.current = clear;
  const dropRef = useRef(drop);
  dropRef.current = drop;

  const togglePin = useCallback(() => {
    setPinned((prev) => {
      const next = !prev;
      api.popupSetPinned(next);
      return next;
    });
  }, []);

  const keyRef = useRef({ result: "", togglePin });
  keyRef.current = { result: state.result, togglePin };

  useEffect(() => {
    startInferenceSync();
    const accept = (p: SelectionPayload) => {
      if (p.id <= seen.current) return;
      seen.current = p.id;
      handedOffRef.current = false;
      ttsStop();
      setPayload(p);
      setExpanded(false);
      setChoice("auto");
      if (p.text) {
        const text = p.text;
        run(text, "auto");
        const loaded = useSettings.getState().settings;
        if (loaded) {
          if (loaded.tts.popupAutoSpeak && text.trim()) {
            ttsPlay("popup:source", text);
          }
        } else if (text.trim()) {
          api
            .getSettings()
            .then((s) => {
              if (seen.current === p.id && s.tts.popupAutoSpeak) {
                ttsPlay("popup:source", text);
              }
            })
            .catch(() => {});
        }
      } else {
        clearRef.current();
      }
    };
    api.popupCurrent().then((p) => p && accept(p));
    const unSel = events.selection(accept);
    const unHide = events.popupHidden(() => {
      ttsStopLocal();
      setPinned(false);
      setPayload(null);
      setExpanded(false);
      if (handedOffRef.current) {
        handedOffRef.current = false;
        dropRef.current();
      } else {
        clearRef.current();
      }
    });
    const onKey = (e: KeyboardEvent) => {
      if (isImeComposing(e)) return;
      if (e.key === "Escape") {
        api.popupClose();
      } else if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "c" && keyRef.current.result) {
        e.preventDefault();
        navigator.clipboard.writeText(keyRef.current.result).catch(() => {});
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        keyRef.current.togglePin();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unSel.then((f) => f());
      unHide.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
  }, [run, ttsPlay, ttsStop, ttsStopLocal]);

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
        origin: "popup",
      });
    }
    wasBusy.current = state.busy;
  }, [state]);

  useStreamAutoScroll(resultRef, state.busy, state.result);
  useAutoHeight(rootRef, (h) => api.popupSetHeight(h), payload?.id ?? 0);

  const cycleChoice = () => {
    const c = NEXT_CHOICE[choice];
    setChoice(c);
    if (payload?.text) run(payload.text, c);
  };

  const openInMain = () => {
    if (!payload?.text) return;
    const hasFinishedResult =
      !state.busy &&
      !state.error &&
      !state.stopped &&
      Boolean(state.result.trim()) &&
      payload.text === state.source &&
      choice === state.choice;
    const handoff: OpenTranslatePayload = {
      text: payload.text,
      choice,
      ...(hasFinishedResult
        ? {
            result: state.result,
            direction: state.direction,
            mixed: state.mixed,
            elapsedMs: state.elapsedMs,
          }
        : {}),
    };
    handedOffRef.current = true;
    ttsStop();
    drop();
    api.openMainWithText(handoff);
    api.popupClose();
  };

  const switchToShot = () => {
    ttsStop();
    api.popupClose();
    api.shotStart();
  };

  const source = payload?.text ?? "";
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

  return (
    <div ref={rootRef} className="flyout-frame flex flex-col text-sm">
      {/* Top draggable header */}
      <div data-tauri-drag-region className="flex h-11 items-center gap-1.5 pr-1.5 pl-3">
        <BrandSeal size="xs" className="pointer-events-none" />

        {source ? (
          <button
            type="button"
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? "收起原文" : "展开原文"}
            className="flex min-w-0 flex-1 cursor-pointer items-center gap-1 rounded-md px-1.5 py-1 text-left text-xs text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
          >
            <span className="truncate">{source.replace(/\s+/g, " ")}</span>
            <ChevronDown
              className={`size-3.5 shrink-0 transition-transform duration-150 ${expanded ? "rotate-180" : ""}`}
            />
          </button>
        ) : (
          <span data-tauri-drag-region className="flex-1 px-1 text-xs font-medium text-[var(--ink-secondary)]">
            划词翻译
          </span>
        )}

        {source && (
          <SpeakButton
            size="sm"
            phase={tts.phaseOf("popup:source")}
            title="朗读原文"
            onClick={() => tts.toggle("popup:source", source, { lang: sourceLang })}
          />
        )}

        {source && <DirCycleButton choice={choice} onClick={cycleChoice} />}

        <IconButton
          size="sm"
          active={pinned}
          title={pinned ? "取消钉住（Alt+P）" : "钉住浮窗，失焦不关闭（Alt+P）"}
          onClick={togglePin}
        >
          {pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />}
        </IconButton>

        {source && (
          <IconButton size="sm" title="在主窗口展开编辑" onClick={openInMain}>
            <ArrowUpRight className="size-3.5" />
          </IconButton>
        )}

        <IconButton size="sm" title="关闭（Esc）" onClick={() => api.popupClose()}>
          <X className="size-3.5" />
        </IconButton>
      </div>

      {/* Expandable original source box */}
      {expanded && source && (
        <div className="mx-3 mb-2.5 flex flex-col gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-recessed)]/75 px-3 py-2">
          <div className="selectable max-h-28 overflow-y-auto text-[13px] leading-relaxed whitespace-pre-wrap text-[var(--ink-secondary)]">
            {source}
          </div>
          <div className="flex justify-end">
            <button
              type="button"
              onClick={() => sourceCopy.copy(source)}
              className="inline-flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--ink-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
            >
              {sourceCopy.copied ? (
                <Check className="size-3 text-[var(--good)]" />
              ) : (
                <Copy className="size-3" />
              )}
              <span>{sourceCopy.copied ? "已复制原文" : "复制原文"}</span>
            </button>
          </div>
        </div>
      )}

      {/* Living Ink Seam */}
      <div data-active={state.busy ? "true" : "false"} className="seam-beam-x h-px w-full" />

      {/* Translation reading body */}
      <div
        ref={resultRef}
        className="selectable max-h-[300px] min-h-12 overflow-y-auto bg-[var(--bg-flyout-result)] px-4 py-3.5 text-[15px] leading-[1.72] whitespace-pre-wrap text-[var(--ink)]"
        style={{ fontFamily: "var(--font-reading)" }}
      >
        {payload?.error && (
          <div className="flex flex-col gap-3 text-xs leading-relaxed">
            <p className="text-[var(--ink-secondary)]">{payload.error}</p>
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="primary" size="sm" icon={<ScanText className="size-3.5" />} onClick={switchToShot}>
                改用截图翻译
              </Button>
              {payload.blockApp && (
                <Button
                  variant="subtle"
                  size="sm"
                  onClick={() => api.popupBlockApp(payload.blockApp!)}
                >
                  不再在 {payload.blockApp} 显示
                </Button>
              )}
            </div>
          </div>
        )}
        <TranslationBody state={state} />
      </div>

      {/* Footer */}
      {source && (
        <div className="flex h-10 items-center gap-1 border-t border-[var(--border-subtle)] bg-[var(--bg-surface)]/75 px-2.5 text-xs">
          {tts.error ? (
            <span className="min-w-0 flex-1 truncate px-1 text-[var(--danger)]" title={tts.error}>
              {tts.error}
            </span>
          ) : (
            <span className="min-w-0 flex-1 truncate px-1 text-[var(--ink-muted)] tabular-nums">
              {footerText(state)}
            </span>
          )}
          <SpeakButton
            size="sm"
            phase={tts.phaseOf("popup:result")}
            disabled={!state.result.trim() || state.busy}
            title="朗读译文"
            onClick={() => tts.toggle("popup:result", state.result, { lang: targetLang })}
          />
          {state.busy ? (
            <IconButton size="sm" title="停止" onClick={stop}>
              <Square className="size-3.5" />
            </IconButton>
          ) : (
            <IconButton size="sm" title="重新翻译" onClick={() => run(source, choice)}>
              <RotateCcw className="size-3.5" />
            </IconButton>
          )}
          <CopyResultButton text={state.result} compact />
        </div>
      )}
    </div>
  );
}
