import { useInference } from "../stores/inference";
import { backendName } from "../lib/format";
import type { InferenceStatus } from "../lib/api";

/** Compact human-friendly model label for titlebar / status chips. */
export function shortModelName(modelId: string | null | undefined): string {
  if (!modelId) return "";
  if (modelId.includes("index-translate-9b")) return "Index 9B";
  if (modelId.includes("index-translate-2b")) return "Index 2B";
  if (modelId.includes("hy-mt2-7b")) return "Hy-MT2 7B";
  return modelId.replace(/-q\d.*$/i, "");
}

export function statusView(status: InferenceStatus): [dot: string, text: string, shortText: string] {
  switch (status.state) {
    case "ready": {
      const model = shortModelName(status.modelId);
      const dev = status.backend === "vulkan" ? "Vulkan" : "CPU";
      return [
        "bg-[var(--good)]",
        `就绪 · ${model ? `${model} · ` : ""}${backendName(status.backend)}`,
        model ? `${model} · ${dev}` : dev,
      ];
    }
    case "loading": {
      const model = shortModelName(status.modelId);
      return [
        "bg-[var(--warn)] animate-pulse",
        `正在加载 ${model || "模型"} · ${backendName(status.backend)}`,
        "加载模型中…",
      ];
    }
    case "idle":
      return ["bg-[var(--ink-muted)]", "模型已休眠，翻译时自动唤醒", "已休眠"];
    case "failed":
      return ["bg-[var(--danger)]", status.message, "推理异常"];
    case "unconfigured":
      return ["bg-[var(--ink-muted)]", "未部署模型", "未部署"];
    default:
      return ["bg-[var(--ink-muted)]", "推理服务未启动", "未启动"];
  }
}

/** Inline compact status indicator (used inside ModelList & ModelSection). */
export function StatusChip({ className = "" }: { className?: string }) {
  const status = useInference((s) => s.status);
  const [dot, text] = statusView(status);
  return (
    <span
      className={`inline-flex min-w-0 items-center gap-1.5 text-xs font-normal ${
        status.state === "failed" ? "text-[var(--danger)]" : "text-[var(--ink-secondary)]"
      } ${className}`}
      title={text}
    >
      <span className={`size-2 shrink-0 rounded-full ${dot}`} />
      <span className="truncate">{text}</span>
    </span>
  );
}

/** Clickable titlebar status pill showing active model & engine health.
 *  Folds into a compact dot button below 690px (keeping concise text when loading/failed). */
export function StatusPill({
  onClick,
  className = "",
}: {
  onClick?: () => void;
  className?: string;
}) {
  const status = useInference((s) => s.status);
  const [dot, fullText, shortText] = statusView(status);
  const urgent = status.state === "loading" || status.state === "failed";
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={fullText}
      title={`${fullText}${onClick ? "（点击查看模型设置）" : ""}`}
      className={`inline-flex h-7 max-w-[190px] cursor-pointer items-center justify-center gap-1.5 rounded-full border bg-[var(--bg-surface)] text-xs transition-colors hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)] hover:text-[var(--ink)] ${
        urgent ? "px-2.5" : "w-7 px-0 min-[690px]:w-auto min-[690px]:px-2.5"
      } ${
        status.state === "failed"
          ? "border-[var(--danger)]/35 text-[var(--danger)]"
          : "border-[var(--border)] text-[var(--ink-secondary)]"
      } ${className}`}
    >
      <span className={`size-2 shrink-0 rounded-full ${dot}`} />
      <span
        className={`truncate font-medium tabular-nums ${
          urgent ? "inline" : "hidden min-[690px]:inline"
        }`}
      >
        {shortText}
      </span>
    </button>
  );
}
