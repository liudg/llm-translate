import { useInference } from "../stores/inference";
import { backendName } from "../lib/format";
import type { InferenceStatus } from "../lib/api";

export function statusView(status: InferenceStatus): [dot: string, text: string] {
  switch (status.state) {
    case "ready":
      return ["bg-green-500", `就绪 · ${backendName(status.backend)}`];
    case "loading":
      return ["bg-amber-500 animate-pulse", `加载模型中 · ${backendName(status.backend)}`];
    case "idle":
      return ["bg-neutral-400", "已释放，下次翻译时自动加载"];
    case "failed":
      return ["bg-red-500", status.message];
    case "unconfigured":
      return ["bg-neutral-400", "未部署"];
    default:
      return ["bg-neutral-400", "未启动"];
  }
}

export function StatusChip({ className = "" }: { className?: string }) {
  const status = useInference((s) => s.status);
  const [dot, text] = statusView(status);
  return (
    <span className={`flex min-w-0 items-center gap-2 text-xs opacity-80 ${className}`} title={text}>
      <span className={`size-2 shrink-0 rounded-full ${dot}`} />
      <span className="truncate">{text}</span>
    </span>
  );
}
