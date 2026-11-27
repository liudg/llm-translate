// Shared streaming translation views used by Main, Mini and Popup.
import { AlertCircle, ArrowRight, Check, Copy } from "lucide-react";
import { DIR_SHORT, type DirChoice, type TranslatorState } from "../lib/useTranslator";
import { useCopy } from "../lib/useCopy";
import { useInference } from "../stores/inference";

/** The streamed result with its warm thinking shimmer, error and stopped states. */
export function TranslationBody({
  state,
  placeholder,
}: {
  state: TranslatorState;
  placeholder?: string;
}) {
  const waking = useInference((s) => s.status.state === "loading" || s.status.state === "idle");
  const gap = state.result ? "mt-3 " : "";

  return (
    <>
      {state.result && (
        <span className={state.busy ? "stream-caret" : ""}>{state.result}</span>
      )}

      {state.busy && !state.result && (
        <div className="flex flex-col gap-2.5 py-0.5 select-none">
          <div className="flex items-center gap-2 text-caption font-medium text-[var(--accent-text)]">
            <span className="size-1.5 animate-pulse rounded-full bg-[var(--accent)]" />
            <span>{waking ? "正在唤醒本地模型…" : "正在翻译…"}</span>
          </div>
          <div className="flex flex-col gap-2">
            <div className="thinking-line h-3.5 w-[86%] rounded-full" />
            <div className="thinking-line h-3.5 w-[64%] rounded-full" />
          </div>
        </div>
      )}

      {placeholder && !state.busy && !state.result && !state.error && !state.stopped && (
        <span className="text-large text-[var(--ink-muted)] select-none">{placeholder}</span>
      )}

      {state.error && (
        <div
          className={`${gap}flex items-start gap-2 rounded-lg border border-[var(--danger)]/20 bg-[var(--danger-soft)] px-3 py-2 text-caption leading-relaxed text-[var(--danger)]`}
        >
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span className="selectable break-all">{state.error}</span>
        </div>
      )}

      {state.stopped && (
        <div className={`${gap}text-caption text-[var(--ink-muted)] select-none`}>
          {state.stopped === "user" ? "已停止生成" : "已被其他窗口的新翻译请求中断"}
        </div>
      )}
    </>
  );
}

/** Accent "复制译文" button of the flyouts and studio footer. */
export function CopyResultButton({
  text,
  compact = false,
  className = "",
}: {
  text: string;
  compact?: boolean;
  className?: string;
}) {
  const { copied, copy } = useCopy();
  return (
    <button
      type="button"
      disabled={!text}
      onClick={() => copy(text)}
      className={`inline-flex cursor-pointer items-center gap-1.5 rounded-md font-medium whitespace-nowrap transition-[background-color,transform,opacity] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40 ${
        compact ? "h-7 px-2.5 text-caption" : "h-7 px-3 text-caption"
      } ${
        copied
          ? "bg-[var(--good)] text-white"
          : "bg-[var(--accent)] text-[var(--ink-inverse)] hover:bg-[var(--accent-hover)]"
      } ${className}`}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      {copied ? "已复制" : "复制译文"}
    </button>
  );
}

/** Tactile direction pill that cycles auto -> zh2En -> en2Zh. */
export function DirCycleButton({
  choice,
  onClick,
  className = "",
}: {
  choice: DirChoice;
  onClick: () => void;
  className?: string;
}) {
  const isLocked = choice !== "auto";
  return (
    <button
      type="button"
      onClick={onClick}
      title="切换翻译方向（自动 / 中→英 / 英→中）"
      className={`inline-flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-md border px-2 text-caption font-medium whitespace-nowrap transition-colors ${
        isLocked
          ? "border-[var(--accent)]/35 bg-[var(--accent-soft)] text-[var(--accent-text)]"
          : "border-[var(--border)] bg-[var(--bg-surface)] text-[var(--ink-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--ink)]"
      } ${className}`}
    >
      {choice === "auto" ? (
        DIR_SHORT.auto
      ) : (
        <span className="inline-flex items-center gap-0.5">
          <span>{choice === "zh2En" ? "中" : "英"}</span>
          <ArrowRight className="size-3 opacity-70" />
          <span>{choice === "zh2En" ? "英" : "中"}</span>
        </span>
      )}
    </button>
  );
}
