// Pieces shared by the translate page, the mini window and the selection popup.
import { Check, Copy } from "lucide-react";
import { DIR_SHORT, type DirChoice, type TranslatorState } from "../lib/useTranslator";
import { useCopy } from "../lib/useCopy";
import { useInference } from "../stores/inference";
import { Spinner } from "./ui";

/** The streamed result with its waiting, error and stopped states. */
export function TranslationBody({ state, placeholder }: { state: TranslatorState; placeholder?: string }) {
  const loading = useInference((s) => s.status.state === "loading");
  const gap = state.result ? "mt-1 " : "";
  return (
    <>
      {state.result && <span className={state.busy ? "stream-caret" : ""}>{state.result}</span>}
      {state.busy && !state.result && (
        <span className="flex items-center gap-2 text-sm opacity-60">
          <Spinner />
          {loading ? "正在加载模型…" : "翻译中…"}
        </span>
      )}
      {placeholder && !state.busy && !state.result && !state.error && !state.stopped && (
        <span className="text-sm opacity-40">{placeholder}</span>
      )}
      {state.error && <div className={`${gap}text-sm text-red-600 dark:text-red-400`}>{state.error}</div>}
      {state.stopped && (
        <div className={`${gap}text-sm opacity-60`}>{state.stopped === "user" ? "已停止" : "已被其他翻译请求中断"}</div>
      )}
    </>
  );
}

/** Accent "复制译文" button of the flyouts. */
export function CopyResultButton({ text }: { text: string }) {
  const { copied, copy } = useCopy();
  return (
    <button
      disabled={!text}
      onClick={() => copy(text)}
      className="inline-flex h-7 items-center gap-1.5 rounded-md bg-[var(--accent)] px-3 text-white hover:bg-[var(--accent-hover)] disabled:opacity-40 dark:text-black"
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      {copied ? "已复制" : "复制译文"}
    </button>
  );
}

/** Small button that shows the direction choice and cycles it. */
export function DirCycleButton({ choice, onClick, className = "" }: { choice: DirChoice; onClick: () => void; className?: string }) {
  return (
    <button
      onClick={onClick}
      title="切换翻译方向"
      className={`h-7 shrink-0 rounded-md border border-[var(--stroke)] bg-[var(--surface)] px-2 text-xs whitespace-nowrap hover:bg-black/5 dark:hover:bg-white/10 ${className}`}
    >
      {DIR_SHORT[choice]}
    </button>
  );
}
