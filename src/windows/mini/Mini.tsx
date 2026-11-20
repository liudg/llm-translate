// Mini window (DESIGN 5.4): Alt+Q flyout with one input. Enter translates,
// Shift+Enter adds a line, Esc hides; the backend hides it on focus loss.
// The window height follows the content.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { Check, Copy, CornerDownLeft, Languages, Square } from "lucide-react";
import { DIR_SHORT, useTranslator, directionText, type DirChoice } from "../../lib/useTranslator";
import { useCopy } from "../../lib/useCopy";
import { IconButton, Spinner } from "../../components/ui";
import { startInferenceSync, useInference } from "../../stores/inference";

const WIDTH = 620;
const MAX_INPUT_PX = 132; // about 5 lines
const NEXT: Record<DirChoice, DirChoice> = { auto: "zh2En", zh2En: "en2Zh", en2Zh: "auto" };

export default function Mini() {
  const win = getCurrentWindow();
  const [text, setText] = useState("");
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop } = useTranslator();
  const { copied, copy } = useCopy();
  const status = useInference((s) => s.status);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    startInferenceSync();
    const un = listen("mini-shown", () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Grow the textarea with its content, up to MAX_INPUT_PX.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_PX)}px`;
  }, [text]);

  // Window height follows the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h !== last) {
        last = h;
        win.setSize(new LogicalSize(WIDTH, h));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [win]);

  const submit = () => {
    if (text.trim()) run(text, choice);
  };

  const cycleChoice = () => {
    const c = NEXT[choice];
    setChoice(c);
    if (state.source && text.trim()) run(text, c);
    inputRef.current?.focus();
  };

  const showResult = state.busy || state.result || state.error || state.stopped;
  const dirText = directionText(state);

  return (
    <div ref={rootRef} className="flex flex-col">
      <div className="flex items-start gap-2 px-3 py-3">
        <div data-tauri-drag-region className="flex h-[38px] w-7 shrink-0 cursor-move items-center justify-center">
          <Languages className="pointer-events-none size-5 text-[var(--accent-text)]" />
        </div>
        <textarea
          ref={inputRef}
          autoFocus
          rows={1}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              win.hide();
            }
          }}
          placeholder="输入中文或英文，回车翻译"
          spellCheck={false}
          className="min-h-[38px] flex-1 resize-none bg-transparent py-[7px] text-base leading-6 outline-none placeholder:opacity-50"
        />
        <button
          onClick={cycleChoice}
          title="切换翻译方向"
          className="mt-[5px] h-7 shrink-0 rounded-md border border-[var(--stroke)] bg-[var(--surface)] px-2 text-xs whitespace-nowrap hover:bg-black/5 dark:hover:bg-white/10"
        >
          {DIR_SHORT[choice]}
        </button>
        <IconButton title="翻译（Enter）" className="mt-[3px]" disabled={!text.trim()} onClick={submit}>
          <CornerDownLeft className="size-4" />
        </IconButton>
      </div>

      {showResult && (
        <div className="flex flex-col border-t border-[var(--stroke)]">
          <div className="selectable max-h-[320px] overflow-y-auto px-5 py-3 text-[15px] leading-relaxed whitespace-pre-wrap">
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
          <div className="flex h-10 items-center gap-1 px-3 text-xs">
            <span className="truncate px-2 opacity-60">
              {dirText}
              {state.elapsedMs !== null && state.elapsedMs > 0 && ` · ${(state.elapsedMs / 1000).toFixed(2)} 秒`}
            </span>
            <span className="ml-auto px-2 opacity-40">Esc 关闭</span>
            {state.busy && (
              <IconButton title="停止" onClick={stop}>
                <Square className="size-3.5" />
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
        </div>
      )}
    </div>
  );
}
