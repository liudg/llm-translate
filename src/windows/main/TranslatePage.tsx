// Input-box translation (DESIGN 5.4): source / result panes, auto-translate with
// a 500 ms debounce (DESIGN 5.3), Ctrl+Enter to translate at once.
import { useEffect, useRef, useState } from "react";
import { useCopy } from "../../lib/useCopy";
import { Check, Copy, PanelTop, RotateCw, ScanText, Square, X } from "lucide-react";
import { api, type HotkeyInfo } from "../../lib/api";
import { DIR_LABEL, DIR_SHORT, useTranslator, directionText, type DirChoice } from "../../lib/useTranslator";
import { Button, IconButton, Segmented, Spinner } from "../../components/ui";
import { StatusChip } from "../../components/status";
import { useInference } from "../../stores/inference";

const DEBOUNCE_MS = 500;
const MAX_CHARS = 20000;

export default function TranslatePage({ hotkeys }: { hotkeys: HotkeyInfo[] }) {
  const [source, setSource] = useState("");
  const [choice, setChoice] = useState<DirChoice>("auto");
  const { state, run, stop, clear } = useTranslator();
  const { copied, copy } = useCopy();
  const status = useInference((s) => s.status);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const miniKeys = hotkeys.find((h) => h.id === "mini" && h.registered)?.keys;
  const shotKeys = hotkeys.find((h) => h.id === "shot" && h.registered)?.keys;

  // Translate after typing pauses; skip when nothing changed since the last request.
  useEffect(() => {
    if (!source.trim()) {
      if (state.source || state.busy) clear();
      return;
    }
    if (source === state.source && choice === state.choice) return;
    const t = window.setTimeout(() => run(source, choice), DEBOUNCE_MS);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source]);

  // Direction change: translate again at once.
  const changeChoice = (c: DirChoice) => {
    setChoice(c);
    if (source.trim()) run(source, c);
  };

  const translateNow = () => {
    if (source.trim()) run(source, choice);
  };

  useEffect(() => inputRef.current?.focus(), []);

  const waitingModel = state.busy && !state.result && status.state === "loading";
  const dirText = directionText(state);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-5">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="mr-1 text-lg font-semibold">翻译</h1>
        <Segmented<DirChoice>
          value={choice}
          onChange={changeChoice}
          options={[
            { value: "auto", label: DIR_SHORT.auto, title: "自动判断方向（中英混排时逐段判断）" },
            { value: "zh2En", label: DIR_SHORT.zh2En, title: DIR_LABEL.zh2En },
            { value: "en2Zh", label: DIR_SHORT.en2Zh, title: DIR_LABEL.en2Zh },
          ]}
        />
        <div className="ml-auto flex min-w-0 items-center gap-2">
          <StatusChip className="max-w-64" />
          <Button
            variant="subtle"
            icon={<PanelTop className="size-4" />}
            onClick={() => api.openMini()}
            title="打开迷你翻译窗"
          >
            迷你窗{miniKeys && <kbd className="ml-1 text-xs opacity-60">{miniKeys}</kbd>}
          </Button>
          <Button
            variant="subtle"
            icon={<ScanText className="size-4" />}
            onClick={() => api.shotStart()}
            title="截图翻译：框选屏幕区域，译文覆盖在原文位置"
          >
            截图{shotKeys && <kbd className="ml-1 text-xs opacity-60">{shotKeys}</kbd>}
          </Button>
        </div>
      </header>

      <div className="grid min-h-0 flex-1 grid-rows-2 gap-3 min-[860px]:grid-cols-2 min-[860px]:grid-rows-1">
        {/* Source */}
        <section className="flex min-h-0 flex-col rounded-lg border border-[var(--stroke)] bg-[var(--surface-strong)] shadow-sm focus-within:border-[var(--accent)]/60">
          <textarea
            ref={inputRef}
            value={source}
            maxLength={MAX_CHARS}
            onChange={(e) => setSource(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && e.ctrlKey) {
                e.preventDefault();
                translateNow();
              }
            }}
            placeholder="输入或粘贴中文 / 英文，停止输入后自动翻译（Ctrl+Enter 立即翻译）"
            className="min-h-0 flex-1 resize-none bg-transparent px-4 py-3 text-[15px] leading-relaxed outline-none placeholder:opacity-50"
            spellCheck={false}
          />
          <div className="flex h-10 shrink-0 items-center gap-1 px-2 text-xs opacity-70">
            <span className="px-2 tabular-nums">
              {source.length.toLocaleString()} / {MAX_CHARS.toLocaleString()}
            </span>
            <IconButton
              className="ml-auto"
              title="清空"
              disabled={!source}
              onClick={() => {
                setSource("");
                clear();
                inputRef.current?.focus();
              }}
            >
              <X className="size-4" />
            </IconButton>
          </div>
        </section>

        {/* Result */}
        <section className="flex min-h-0 flex-col rounded-lg border border-[var(--stroke)] bg-[var(--surface)] shadow-sm">
          <div className="selectable min-h-0 flex-1 overflow-y-auto px-4 py-3 text-[15px] leading-relaxed whitespace-pre-wrap">
            {state.result && <span className={state.busy ? "stream-caret" : ""}>{state.result}</span>}
            {state.busy && !state.result && (
              <span className="flex items-center gap-2 text-sm opacity-60">
                <Spinner />
                {waitingModel ? "正在加载模型…" : "翻译中…"}
              </span>
            )}
            {!state.busy && !state.result && !state.error && !state.stopped && (
              <span className="text-sm opacity-40">译文将显示在这里</span>
            )}
            {state.error && <div className="mt-1 text-sm text-red-600 dark:text-red-400">{state.error}</div>}
            {state.stopped && (
              <div className="mt-1 text-sm opacity-60">
                {state.stopped === "user" ? "已停止" : "已被其他翻译请求中断"}
              </div>
            )}
          </div>
          <div className="flex h-10 shrink-0 items-center gap-1 px-2 text-xs">
            <span className="truncate px-2 opacity-60">
              {dirText}
              {state.elapsedMs !== null && state.elapsedMs > 0 && ` · ${(state.elapsedMs / 1000).toFixed(2)} 秒`}
            </span>
            <div className="ml-auto flex items-center gap-1">
              {state.busy ? (
                <IconButton title="停止" onClick={stop}>
                  <Square className="size-3.5" />
                </IconButton>
              ) : (
                <IconButton title="重新翻译" disabled={!source.trim()} onClick={translateNow}>
                  <RotateCw className="size-4" />
                </IconButton>
              )}
              <IconButton title={copied ? "已复制" : "复制译文"} disabled={!state.result} onClick={() => copy(state.result)}>
                {copied ? <Check className="size-4 text-green-600 dark:text-green-400" /> : <Copy className="size-4" />}
              </IconButton>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
