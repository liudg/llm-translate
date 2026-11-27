// Translation Studio (DESIGN 5.4): unified editorial dual-sheet separated by
// the Living Ink Seam, with swap/back-translate, quick paste, bilingual copy,
// smart auto-scroll, in-app history drawer and full keyboard ergonomics.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeftRight,
  Check,
  ClipboardPaste,
  Clock,
  Columns2,
  Copy,
  CornerDownLeft,
  RotateCw,
  Square,
  X,
} from "lucide-react";
import { type HotkeyInfo, type OpenTranslatePayload, type TtsLang } from "../../lib/api";
import { useCopy } from "../../lib/useCopy";
import { useStreamAutoScroll } from "../../lib/useStreamAutoScroll";
import { DIR_LABEL, isImeComposing, useTranslator, type DirChoice } from "../../lib/useTranslator";
import { useTts } from "../../lib/useTts";
import { useHistory, type HistoryItem } from "../../stores/history";
import { useSettings } from "../../stores/settings";
import { Button, IconButton, Kbd, Segmented } from "../../components/ui";
import { SpeakButton } from "../../components/tts";
import { TranslationBody } from "../../components/translation";
import { HistoryDrawer } from "./HistoryDrawer";

const DEBOUNCE_MS = 500;
const MAX_CHARS = 20000;
const READING_TEXT_CLASS = "text-large leading-[1.78]";

export interface IncomingTranslate {
  payload: OpenTranslatePayload;
  seq: number;
}

export default function TranslatePage({
  hotkeys,
  visible,
  incoming,
}: {
  hotkeys: HotkeyInfo[];
  visible: boolean;
  incoming: IncomingTranslate | null;
}) {
  const [source, setSource] = useState("");
  const [choice, setChoice] = useState<DirChoice>("auto");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [composing, setComposing] = useState(false);

  const { state, run, stop, clear, hydrate } = useTranslator();
  const tts = useTts();
  const stopMainTts = tts.stopIfKeyPrefix;
  const { copied, copy } = useCopy();
  const { copied: copiedBilingual, copy: copyBilingual } = useCopy();
  const settings = useSettings((s) => s.settings);
  const historyItems = useHistory((s) => s.items);
  const recordHistory = useHistory((s) => s.record);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const resultRef = useRef<HTMLDivElement>(null);
  const timer = useRef<number>(undefined);
  const wasBusy = useRef(false);
  const handledSeq = useRef(0);

  useStreamAutoScroll(resultRef, state.busy, state.result);

  const miniKeys = hotkeys.find((h) => h.id === "mini" && h.registered)?.keys;
  const shotKeys = hotkeys.find((h) => h.id === "shot" && h.registered)?.keys;

  // Close history drawer automatically whenever leaving the translate page.
  useEffect(() => {
    if (!visible) setHistoryOpen(false);
  }, [visible]);

  // Receive structured handoff payload from Mini or Popup window.
  useEffect(() => {
    if (!incoming || incoming.seq <= handledSeq.current) return;
    handledSeq.current = incoming.seq;
    window.clearTimeout(timer.current);
    const { payload } = incoming;
    const clipped = payload.text.slice(0, MAX_CHARS);
    const nextChoice: DirChoice =
      payload.choice === "zh2En" || payload.choice === "en2Zh" || payload.choice === "auto"
        ? payload.choice
        : "auto";
    setSource(clipped);
    setChoice(nextChoice);

    const hasResult = Boolean(payload.result && payload.result.trim()) && clipped === payload.text;
    if (hasResult && payload.result) {
      wasBusy.current = false;
      hydrate({
        source: clipped,
        choice: nextChoice,
        result: payload.result,
        direction: payload.direction ?? null,
        mixed: Boolean(payload.mixed),
        elapsedMs: payload.elapsedMs ?? null,
      });
    } else if (clipped.trim()) {
      run(clipped, nextChoice);
    } else {
      clear();
    }
    window.setTimeout(() => inputRef.current?.focus(), 40);
  }, [incoming, run, hydrate, clear]);

  // Translate after typing pauses; skip when unchanged & clean or while IME is composing.
  useEffect(() => {
    window.clearTimeout(timer.current);
    if (!source.trim()) {
      if (state.source || state.busy) clear();
      return;
    }
    if (
      composing ||
      (source === state.source && choice === state.choice && !state.error && !state.stopped)
    ) {
      return;
    }
    timer.current = window.setTimeout(() => run(source, choice), DEBOUNCE_MS);
    return () => window.clearTimeout(timer.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, composing]);

  // Record finished clean translations into local history.
  useEffect(() => {
    if (wasBusy.current && !state.busy && state.result.trim() && !state.error && !state.stopped && state.direction) {
      recordHistory({
        source: state.source,
        result: state.result,
        direction: state.direction,
        mixed: state.mixed,
        elapsedMs: state.elapsedMs,
        origin: "main",
      });
    }
    wasBusy.current = state.busy;
  }, [state, recordHistory]);

  const runNow = useCallback(
    (c: DirChoice, text = source) => {
      window.clearTimeout(timer.current);
      if (text.trim()) run(text, c);
    },
    [run, source],
  );

  const changeChoice = useCallback(
    (c: DirChoice) => {
      setChoice(c);
      runNow(c);
    },
    [runNow],
  );

  const translateNow = useCallback(() => runNow(choice), [choice, runNow]);

  const swapTranslation = useCallback(() => {
    if (!state.result.trim() || state.busy) return;
    const nextSource = state.result.trim().slice(0, MAX_CHARS);
    const nextChoice: DirChoice =
      choice === "zh2En"
        ? "en2Zh"
        : choice === "en2Zh"
          ? "zh2En"
          : state.mixed
            ? "auto"
            : state.direction === "zh2En"
              ? "en2Zh"
              : state.direction === "en2Zh"
                ? "zh2En"
                : "auto";
    setSource(nextSource);
    setChoice(nextChoice);
    runNow(nextChoice, nextSource);
    inputRef.current?.focus();
  }, [choice, runNow, state.busy, state.direction, state.mixed, state.result]);

  const pasteClipboard = useCallback(async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (!text) return;
      const clipped = text.slice(0, MAX_CHARS);
      setSource(clipped);
      runNow(choice, clipped);
      inputRef.current?.focus();
    } catch {
      inputRef.current?.focus();
    }
  }, [choice, runNow]);

  const copyBoth = useCallback(() => {
    const src = (state.source || source).trim();
    const res = state.result.trim();
    if (!src || !res) return;
    copyBilingual(`${src}\n\n${res}`);
  }, [copyBilingual, source, state.result, state.source]);

  const restoreHistory = useCallback(
    (item: HistoryItem) => {
      window.clearTimeout(timer.current);
      const nextChoice: DirChoice = item.mixed ? "auto" : item.direction;
      setSource(item.source);
      setChoice(nextChoice);
      wasBusy.current = false;
      hydrate({
        source: item.source,
        choice: nextChoice,
        result: item.result,
        direction: item.direction,
        mixed: item.mixed,
        elapsedMs: item.elapsedMs,
      });
      inputRef.current?.focus();
    },
    [hydrate],
  );

  const clearAll = useCallback(() => {
    window.clearTimeout(timer.current);
    stopMainTts("main:");
    setSource("");
    clear();
    inputRef.current?.focus();
  }, [clear, stopMainTts]);

  // Focus textarea when page becomes visible and drawer is closed.
  useEffect(() => {
    if (visible && !historyOpen) {
      inputRef.current?.focus();
    }
  }, [visible, historyOpen]);

  // Studio-wide keyboard shortcuts (while history drawer is open, only Ctrl+H is allowed).
  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if (isImeComposing(e)) return;
      if (e.ctrlKey && !e.altKey && !e.metaKey) {
        const k = e.key.toLowerCase();
        if (!e.shiftKey && k === "h") {
          e.preventDefault();
          setHistoryOpen((v) => !v);
          return;
        }
        if (historyOpen) return;
        if (e.shiftKey && k === "c" && state.result) {
          e.preventDefault();
          copy(state.result);
        } else if (e.shiftKey && k === "s" && state.result && !state.busy) {
          e.preventDefault();
          swapTranslation();
        } else if (!e.shiftKey && k === "1") {
          e.preventDefault();
          changeChoice("auto");
        } else if (!e.shiftKey && k === "2") {
          e.preventDefault();
          changeChoice("zh2En");
        } else if (!e.shiftKey && k === "3") {
          e.preventDefault();
          changeChoice("en2Zh");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, historyOpen, state.result, state.busy, copy, swapTranslation, changeChoice]);

  const nearLimit = source.length > MAX_CHARS * 0.9;
  const pendingInput =
    Boolean(source.trim()) &&
    (source !== state.source || choice !== state.choice || Boolean(state.error) || Boolean(state.stopped)) &&
    !state.busy;

  // Null until auto mode has detected a direction.
  const targetLangLabel =
    choice === "zh2En"
      ? "英语"
      : choice === "en2Zh"
        ? "中文"
        : state.mixed
          ? "逐段互译"
          : state.direction === "zh2En"
            ? "英语"
            : state.direction === "en2Zh"
              ? "中文"
              : null;

  const canSwap = Boolean(state.result.trim()) && !state.busy;
  const canCopyBilingual = Boolean(state.result.trim()) && Boolean((state.source || source).trim());
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
    <div className="relative flex min-h-0 flex-1 flex-col px-4 pt-3 pb-4">
      {/* Unified Editorial Dual-Sheet ("双笺光缝") */}
      <div
        inert={historyOpen}
        className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-[var(--shadow-card)] min-[780px]:flex-row"
      >
        {/* Source Sheet */}
        <section className="relative flex min-h-0 flex-1 flex-col bg-[var(--bg-surface)]">
          {/* Sheet header: direction picker + source actions (labels fold away when the sheet is narrow) */}
          <div className="@container flex h-10 shrink-0 items-center gap-2 border-b border-[var(--border-subtle)] px-4">
            <Segmented<DirChoice>
              size="sm"
              value={choice}
              onChange={changeChoice}
              options={[
                { value: "auto", label: "自动检测", title: "自动判断语言方向（Ctrl+1），中英混排逐段翻译" },
                { value: "zh2En", label: "中 → 英", title: `${DIR_LABEL.zh2En}（Ctrl+2）` },
                { value: "en2Zh", label: "英 → 中", title: `${DIR_LABEL.en2Zh}（Ctrl+3）` },
              ]}
            />
            <div className="ml-auto flex items-center gap-1">
              <SpeakButton
                size="xs"
                phase={tts.phaseOf("main:source")}
                disabled={!source.trim()}
                title="朗读原文"
                onClick={() => tts.toggle("main:source", source, { lang: sourceLang })}
              />
              <button
                type="button"
                onClick={pasteClipboard}
                aria-label="粘贴"
                title="粘贴剪贴板文本并立即翻译（Ctrl+Shift+V）"
                className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md px-2 text-caption text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
              >
                <ClipboardPaste className="size-3" />
                <span className="hidden @min-[25rem]:inline">粘贴</span>
              </button>
              {source && (
                <button
                  type="button"
                  onClick={clearAll}
                  aria-label="清空"
                  title="清空原文"
                  className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md px-2 text-caption text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
                >
                  <X className="size-3" />
                  <span className="hidden @min-[25rem]:inline">清空</span>
                </button>
              )}
            </div>
          </div>

          {/* Textarea */}
          <div className="relative flex min-h-0 flex-1 flex-col">
            <textarea
              ref={inputRef}
              value={source}
              maxLength={MAX_CHARS}
              onChange={(e) => setSource(e.target.value)}
              onCompositionStart={() => setComposing(true)}
              onCompositionEnd={() => setComposing(false)}
              onBlur={() => setComposing(false)}
              onKeyDown={(e) => {
                if (isImeComposing(e)) return;
                if (e.key === "Enter" && e.ctrlKey && !e.altKey && !e.metaKey) {
                  e.preventDefault();
                  translateNow();
                } else if (
                  e.ctrlKey &&
                  e.shiftKey &&
                  !e.altKey &&
                  !e.metaKey &&
                  e.key.toLowerCase() === "v"
                ) {
                  e.preventDefault();
                  void pasteClipboard();
                }
              }}
              placeholder="输入或粘贴要翻译的内容…"
              spellCheck={false}
              className={`min-h-0 flex-1 resize-none bg-transparent px-5 py-4 text-[var(--ink)] outline-none placeholder:text-[var(--ink-muted)] ${READING_TEXT_CLASS}`}
              style={{ fontFamily: "var(--font-reading)" }}
            />

            {/* Serene empty-state shortcuts whisper */}
            {!source && (
              <div className="pointer-events-none absolute inset-x-5 bottom-3 flex flex-wrap items-center gap-x-3.5 gap-y-1.5 text-caption text-[var(--ink-muted)]">
                {miniKeys && (
                  <span className="inline-flex items-center gap-1.5">
                    <Kbd keys={miniKeys} compact />
                    <span>迷你窗</span>
                  </span>
                )}
                {shotKeys && (
                  <span className="inline-flex items-center gap-1.5">
                    <Kbd keys={shotKeys} compact />
                    <span>截图翻译</span>
                  </span>
                )}
                <span className="inline-flex items-center gap-1.5">
                  <span
                    className={`size-1.5 rounded-full ${
                      settings?.selection.enabled ? "bg-[var(--good)]" : "bg-[var(--ink-muted)]"
                    }`}
                  />
                  <span>{settings?.selection.enabled ? "划词翻译已开启" : "划词翻译已关闭"}</span>
                </span>
              </div>
            )}
          </div>

          {/* Source micro-footer */}
          <div className="flex h-10 shrink-0 items-center justify-between gap-2 border-t border-[var(--border-subtle)] px-4 text-caption">
            {tts.error && tts.errorKey === "main:source" ? (
              <span className="min-w-0 flex-1 truncate text-[var(--danger)]" title={tts.error}>
                {tts.error}
              </span>
            ) : (
              <span
                className={`tabular-nums ${
                  nearLimit ? "font-medium text-[var(--warn)]" : "text-[var(--ink-muted)]"
                }`}
              >
                {source.length.toLocaleString()} / {MAX_CHARS.toLocaleString()} 字
              </span>
            )}

            {source.trim() && (
              <button
                type="button"
                onClick={translateNow}
                title="立即翻译（Ctrl+Enter）"
                className={`inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-2.5 text-caption font-medium transition-colors ${
                  pendingInput
                    ? "bg-[var(--accent-soft)] text-[var(--accent-text)] hover:bg-[var(--accent)]/15"
                    : "text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
                }`}
              >
                <CornerDownLeft className="size-3.5" />
                <span>翻译</span>
                <Kbd keys="Ctrl+Enter" compact />
              </button>
            )}
          </div>
        </section>

        {/* The Living Ink Seam + Center Swap Medallion */}
        <div className="relative flex shrink-0 items-center justify-center">
          {/* Horizontal seam (narrow stacked mode) */}
          <div
            data-active={state.busy ? "true" : "false"}
            className="seam-beam-x h-px w-full min-[780px]:hidden"
          />
          {/* Vertical seam (wide dual-sheet mode) */}
          <div
            data-active={state.busy ? "true" : "false"}
            className="seam-beam-y hidden h-full w-px min-[780px]:block"
          />
          {/* Floating Swap / Back-translate medallion (always visible warm medallion) */}
          <button
            type="button"
            disabled={!canSwap}
            onClick={swapTranslation}
            aria-label="将译文填入原文并反向回译"
            title={
              canSwap
                ? "将译文填入原文并反向回译（Ctrl+Shift+S）"
                : "生成译文后可一键反向回译（Ctrl+Shift+S）"
            }
            className="absolute z-10 inline-flex size-7 cursor-pointer items-center justify-center rounded-full border border-[var(--border)] bg-[var(--bg-elevated)] text-[var(--ink-secondary)] shadow-[var(--shadow-xs)] transition-[color,border-color,transform,opacity] duration-150 hover:scale-105 hover:border-[var(--accent)]/50 hover:text-[var(--accent-text)] active:scale-95 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:scale-100 disabled:hover:border-[var(--border)] disabled:hover:text-[var(--ink-secondary)]"
          >
            <ArrowLeftRight className="size-3.5 rotate-90 transition-transform duration-150 min-[780px]:rotate-0" />
          </button>
        </div>

        {/* Translation Sheet */}
        <section className="flex min-h-0 flex-1 flex-col bg-[var(--bg-surface-alt)]">
          {/* Sheet header: target language + history */}
          <div className="flex h-10 shrink-0 items-center gap-2 border-b border-[var(--border-subtle)] px-4">
            <span className="text-caption font-semibold tracking-wide text-[var(--accent-text)]">译文</span>
            {targetLangLabel && (
              <span className="text-caption text-[var(--ink-secondary)]">· {targetLangLabel}</span>
            )}

            <div className="ml-auto flex items-center gap-2">
              {state.busy && (
                <span className="inline-flex items-center gap-1.5 text-caption font-medium text-[var(--accent-text)]">
                  <span className="size-1.5 animate-pulse rounded-full bg-[var(--accent)]" />
                  生成中
                </span>
              )}
              <button
                type="button"
                onClick={() => setHistoryOpen(true)}
                title="查看翻译历史（Ctrl+H）"
                className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md px-2 text-caption text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
              >
                <Clock className="size-3" />
                <span>历史</span>
                {historyItems.length > 0 && (
                  <span className="rounded-md bg-[var(--bg-recessed)] px-1 text-micro font-semibold text-[var(--ink-secondary)] tabular-nums">
                    {historyItems.length}
                  </span>
                )}
              </button>
            </div>
          </div>

          {/* Reading canvas */}
          <div
            ref={resultRef}
            className={`selectable min-h-0 flex-1 overflow-y-auto px-5 py-4 whitespace-pre-wrap text-[var(--ink)] ${READING_TEXT_CLASS}`}
            style={{ fontFamily: "var(--font-reading)" }}
          >
            <TranslationBody state={state} placeholder="译文将显示在这里" />
          </div>

          {/* Translation micro-footer */}
          <div className="flex h-10 shrink-0 items-center gap-1.5 border-t border-[var(--border-subtle)] px-3 text-caption">
            {/* Direction already shows in the sheet headers; the footer only reports read-aloud errors. */}
            {tts.error && tts.errorKey === "main:result" ? (
              <span className="min-w-0 flex-1 truncate px-1 text-[var(--danger)]" title={tts.error}>
                {tts.error}
              </span>
            ) : (
              <span className="flex-1" />
            )}

            <div className="flex shrink-0 items-center gap-1">
              <SpeakButton
                size="sm"
                phase={tts.phaseOf("main:result")}
                disabled={!state.result.trim() || state.busy}
                title="朗读译文"
                onClick={() => tts.toggle("main:result", state.result, { lang: targetLang })}
              />

              {state.busy ? (
                <Button variant="secondary" size="sm" icon={<Square className="size-3" />} onClick={stop}>
                  停止
                </Button>
              ) : (
                <IconButton
                  size="sm"
                  title="重新翻译"
                  disabled={!source.trim()}
                  onClick={translateNow}
                >
                  <RotateCw className="size-3.5" />
                </IconButton>
              )}

              <Button
                variant="subtle"
                size="sm"
                disabled={!canCopyBilingual}
                onClick={copyBoth}
                title="复制原文与译文双语对照"
                icon={
                  copiedBilingual ? (
                    <Check className="size-3.5 text-[var(--good)]" />
                  ) : (
                    <Columns2 className="size-3.5" />
                  )
                }
              >
                {copiedBilingual ? "已复制" : "双语"}
              </Button>

              <button
                type="button"
                disabled={!state.result}
                onClick={() => copy(state.result)}
                title="复制译文（Ctrl+Shift+C）"
                className={`inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-3 text-caption font-medium transition-[background-color,transform,opacity] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40 ${
                  copied
                    ? "bg-[var(--good)] text-white"
                    : "bg-[var(--accent)] text-[var(--ink-inverse)] hover:bg-[var(--accent-hover)]"
                }`}
              >
                {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
                <span>{copied ? "已复制" : "复制译文"}</span>
              </button>
            </div>
          </div>
        </section>
      </div>

      <HistoryDrawer
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        onRestore={restoreHistory}
      />
    </div>
  );
}
