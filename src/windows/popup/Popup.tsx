// Selection popup (DESIGN 5.5): shows the captured text (folded to one line)
// and streams its translation. Hides on focus loss unless pinned; the backend
// places it next to the selection and keeps it sized to this page.
import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Copy, Languages, Pin, PinOff, RotateCcw, Square, X } from "lucide-react";
import { api, events, type SelectionPayload } from "../../lib/api";
import { DIR_SHORT, directionText, useTranslator, type DirChoice } from "../../lib/useTranslator";
import { useCopy } from "../../lib/useCopy";
import { IconButton, Spinner } from "../../components/ui";
import { startInferenceSync, useInference } from "../../stores/inference";

const NEXT: Record<DirChoice, DirChoice> = { auto: "zh2En", zh2En: "en2Zh", en2Zh: "auto" };

export default function Popup() {
  const [payload, setPayload] = useState<SelectionPayload | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop, clear } = useTranslator();
  const { copied, copy } = useCopy();
  const status = useInference((s) => s.status);
  const rootRef = useRef<HTMLDivElement>(null);
  const seen = useRef(0);
  const busyRef = useRef(false);
  busyRef.current = state.busy;
  const clearRef = useRef(clear);
  clearRef.current = clear;

  useEffect(() => {
    startInferenceSync();
    const accept = (p: SelectionPayload) => {
      if (p.id <= seen.current) return;
      seen.current = p.id;
      setPayload(p);
      setExpanded(false);
      setChoice("auto");
      if (p.text) run(p.text, "auto");
      else clearRef.current(); // drop the previous result
    };
    api.popupCurrent().then((p) => p && accept(p));
    const unSel = events.selection(accept);
    const unHide = events.popupHidden(() => {
      setPinned(false);
      if (busyRef.current) stop();
      // Next capture opens clean instead of flashing this result first.
      setPayload(null);
      setExpanded(false);
      clearRef.current();
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") api.popupClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unSel.then((f) => f());
      unHide.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
  }, [run, stop]);

  // The window height follows the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h !== last) {
        last = h;
        api.popupSetHeight(h);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const togglePin = () => {
    const p = !pinned;
    setPinned(p);
    api.popupSetPinned(p);
  };

  const cycleChoice = () => {
    const c = NEXT[choice];
    setChoice(c);
    if (payload?.text) run(payload.text, c);
  };

  const source = payload?.text ?? "";
  const dirText = directionText(state);

  return (
    <div ref={rootRef} className="flyout-frame flex flex-col text-sm">
      <div data-tauri-drag-region className="flex h-11 items-center gap-1.5 pr-1.5 pl-3">
        <Languages className="pointer-events-none size-4 shrink-0 text-[var(--accent-text)]" />
        {source ? (
          <button
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? "收起原文" : "展开原文"}
            className="flex min-w-0 flex-1 items-center gap-1 rounded px-1 py-0.5 text-left opacity-70 hover:bg-black/5 hover:opacity-100 dark:hover:bg-white/10"
          >
            <span className="truncate">{source.replace(/\s+/g, " ")}</span>
            <ChevronDown className={`size-3.5 shrink-0 transition-transform ${expanded ? "rotate-180" : ""}`} />
          </button>
        ) : (
          <span data-tauri-drag-region className="flex-1 px-1 opacity-70">
            划词翻译
          </span>
        )}
        {source && (
          <button
            onClick={cycleChoice}
            title="切换翻译方向"
            className="h-7 shrink-0 rounded-md border border-[var(--stroke)] bg-[var(--surface)] px-2 text-xs whitespace-nowrap hover:bg-black/5 dark:hover:bg-white/10"
          >
            {DIR_SHORT[choice]}
          </button>
        )}
        <IconButton title={pinned ? "取消钉住" : "钉住（失焦不关闭）"} onClick={togglePin} className="size-7">
          {pinned ? <PinOff className="size-3.5 text-[var(--accent-text)]" /> : <Pin className="size-3.5" />}
        </IconButton>
        <IconButton title="关闭（Esc）" onClick={() => api.popupClose()} className="size-7">
          <X className="size-4" />
        </IconButton>
      </div>

      {expanded && source && (
        <div className="selectable mx-3 mb-2 max-h-28 overflow-y-auto rounded-md bg-black/[0.03] px-3 py-2 leading-relaxed whitespace-pre-wrap opacity-80 dark:bg-white/[0.04]">
          {source}
        </div>
      )}

      <div className="selectable max-h-[300px] min-h-10 overflow-y-auto border-t border-[var(--stroke)] px-4 py-3 text-[15px] leading-relaxed whitespace-pre-wrap">
        {payload?.error && <div className="text-sm opacity-75">{payload.error}</div>}
        {state.result && <span className={state.busy ? "stream-caret" : ""}>{state.result}</span>}
        {state.busy && !state.result && (
          <span className="flex items-center gap-2 text-sm opacity-60">
            <Spinner />
            {status.state === "loading" ? "正在加载模型…" : "翻译中…"}
          </span>
        )}
        {state.error && <div className="text-sm text-red-600 dark:text-red-400">{state.error}</div>}
        {state.stopped && (
          <div className="text-sm opacity-60">{state.stopped === "user" ? "已停止" : "已被其他翻译请求中断"}</div>
        )}
      </div>

      {source && (
        <div className="flex h-10 items-center gap-1 px-2 text-xs">
          <span className="min-w-0 flex-1 truncate px-2 opacity-60">
            {dirText}
            {state.elapsedMs !== null && state.elapsedMs > 0 && ` · ${(state.elapsedMs / 1000).toFixed(2)} 秒`}
          </span>
          {state.busy ? (
            <IconButton title="停止" onClick={stop} className="size-7">
              <Square className="size-3.5" />
            </IconButton>
          ) : (
            <IconButton title="重新翻译" onClick={() => run(source, choice)} className="size-7">
              <RotateCcw className="size-3.5" />
            </IconButton>
          )}
          <button
            disabled={!state.result}
            onClick={() => copy(state.result)}
            className="inline-flex h-7 items-center gap-1.5 rounded-md bg-[var(--accent)] px-3 text-white hover:bg-[var(--accent-hover)] disabled:opacity-40 dark:text-black"
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            {copied ? "已复制" : "复制译文"}
          </button>
        </div>
      )}
    </div>
  );
}
