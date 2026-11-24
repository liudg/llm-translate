// Mini window (DESIGN 5.4): Alt+Q flyout with one input. Enter translates,
// Shift+Enter adds a line, Esc hides; the backend hides it on focus loss.
// The window height follows the content.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { CornerDownLeft, Languages, Square } from "lucide-react";
import { NEXT_CHOICE, footerText, useTranslator, type DirChoice } from "../../lib/useTranslator";
import { useAutoHeight } from "../../lib/useAutoHeight";
import { IconButton } from "../../components/ui";
import { CopyResultButton, DirCycleButton, TranslationBody } from "../../components/translation";
import { startInferenceSync } from "../../stores/inference";

const WIDTH = 620;
const MAX_INPUT_PX = 132; // about 5 lines

export default function Mini() {
  const win = getCurrentWindow();
  const [text, setText] = useState("");
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop } = useTranslator();
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    startInferenceSync();
    const un = listen("mini-shown", () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    // Esc closes from anywhere (e.g. after clicking 复制译文), not just the input.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) {
        e.preventDefault();
        win.hide();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      un.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Grow the textarea with its content, up to MAX_INPUT_PX.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_PX)}px`;
  }, [text]);

  // Window height follows the content.
  useAutoHeight(rootRef, (h) => win.setSize(new LogicalSize(WIDTH, h)));

  const submit = () => {
    if (text.trim()) run(text, choice);
  };

  const cycleChoice = () => {
    const c = NEXT_CHOICE[choice];
    setChoice(c);
    if (state.source && text.trim()) run(text, c);
    inputRef.current?.focus();
  };

  const showResult = state.busy || state.result || state.error || state.stopped;

  return (
    <div ref={rootRef} className="flyout-frame flex flex-col">
      <div className="flex items-start gap-2 px-3 py-3">
        <div data-tauri-drag-region className="flex h-[38px] w-7 shrink-0 cursor-move items-center justify-center">
          <Languages className="pointer-events-none size-5 text-[var(--accent-text)]" />
        </div>
        <textarea
          ref={inputRef}
          autoFocus
          rows={1}
          maxLength={20000}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }}
          placeholder="输入中文或英文，回车翻译"
          spellCheck={false}
          className="min-h-[38px] flex-1 resize-none bg-transparent py-[7px] text-base leading-6 outline-none placeholder:opacity-50"
        />
        <DirCycleButton choice={choice} onClick={cycleChoice} className="mt-[5px]" />
        <IconButton title="翻译（Enter）" className="mt-[3px]" disabled={!text.trim()} onClick={submit}>
          <CornerDownLeft className="size-4" />
        </IconButton>
      </div>

      {showResult && (
        <div className="flex flex-col border-t border-[var(--stroke)]">
          <div className="selectable max-h-[320px] overflow-y-auto px-5 py-3 text-[15px] leading-relaxed whitespace-pre-wrap">
            <TranslationBody state={state} />
          </div>
          <div className="flex h-10 items-center gap-1 px-3 text-xs">
            <span className="truncate px-2 opacity-60">{footerText(state)}</span>
            <span className="ml-auto px-2 opacity-40">Esc 关闭</span>
            {state.busy && (
              <IconButton title="停止" onClick={stop}>
                <Square className="size-3.5" />
              </IconButton>
            )}
            <CopyResultButton text={state.result} />
          </div>
        </div>
      )}
    </div>
  );
}
