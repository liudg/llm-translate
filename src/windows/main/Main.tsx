// M1 test panel: plain translate box + model status. Replaced by the real UI in M2.
import { useEffect, useRef, useState } from "react";
import { AlertTriangle, ArrowRightLeft, Copy, FileText, Languages, RefreshCw, Square, X } from "lucide-react";
import { api, type Direction, type Settings } from "../../lib/api";
import { backendName } from "../../lib/format";
import { Button, Card, Spinner } from "../../components/ui";
import { startInferenceSync, useInference } from "../../stores/inference";

type DirChoice = "auto" | Direction;

const DIR_LABEL: Record<Direction, string> = { zh2En: "中文 → 英语", en2Zh: "英语 → 中文" };

function StatusChip() {
  const status = useInference((s) => s.status);
  const [color, text] = (() => {
    switch (status.state) {
      case "ready":
        return ["bg-green-500", `就绪 · ${backendName(status.backend)}`];
      case "loading":
        return ["bg-amber-500 animate-pulse", `加载模型中 · ${backendName(status.backend)}`];
      case "idle":
        return ["bg-neutral-400", "已释放（下次翻译时自动加载）"];
      case "failed":
        return ["bg-red-500", status.message];
      case "unconfigured":
        return ["bg-neutral-400", "未部署"];
      default:
        return ["bg-neutral-400", "未启动"];
    }
  })();
  return (
    <span className="flex min-w-0 items-center gap-2 text-xs opacity-80" title={text}>
      <span className={`size-2 shrink-0 rounded-full ${color}`} />
      <span className="truncate">{text}</span>
    </span>
  );
}

export default function Main() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [source, setSource] = useState("");
  const [result, setResult] = useState("");
  const [dir, setDir] = useState<DirChoice>("auto");
  const [usedDir, setUsedDir] = useState<Direction | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState<number | null>(null);
  const status = useInference((s) => s.status);
  const notice = useInference((s) => s.notice);
  const dismissNotice = useInference((s) => s.dismissNotice);
  const reqId = useRef(0);

  useEffect(() => {
    startInferenceSync();
    api.getSettings().then(setSettings);
  }, []);

  // Settings change after a (re)deploy or a CPU fallback.
  useEffect(() => {
    api.getSettings().then(setSettings);
  }, [status.state]);

  const run = async () => {
    if (!source.trim()) return;
    const id = ++reqId.current;
    setBusy(true);
    setError(null);
    setResult("");
    setElapsed(null);
    await api.translate(source, dir === "auto" ? null : dir, (e) => {
      if (id !== reqId.current) return;
      switch (e.type) {
        case "start":
          setUsedDir(e.direction);
          break;
        case "delta":
          setResult((r) => r + e.text);
          break;
        case "done":
          setElapsed(e.elapsedMs);
          break;
        case "error":
          setError(e.message);
          break;
        case "cancelled":
          break;
      }
    });
    if (id === reqId.current) setBusy(false);
  };

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      <header className="flex items-center gap-3">
        <Languages className="size-5 text-[var(--accent-text)]" />
        <h1 className="font-semibold">LLM Translate</h1>
        <span className="text-xs opacity-50">M1 测试面板</span>
        <div className="ml-auto flex min-w-0 items-center gap-3">
          <StatusChip />
          <span className="shrink-0 text-xs opacity-50">{settings?.modelId}</span>
          <Button variant="subtle" icon={<FileText className="size-4" />} onClick={() => api.openLogsDir()}>
            日志
          </Button>
          <Button icon={<RefreshCw className="size-4" />} onClick={() => api.openWizard()}>
            重新部署
          </Button>
        </div>
      </header>

      {notice && (
        <div className="flex items-start gap-2 rounded-md bg-amber-500/10 p-2.5 text-xs text-amber-800 dark:text-amber-300">
          <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          <span className="flex-1">GPU 后端多次失败，已切换到 CPU 并保存设置。原因：{notice.reason}</span>
          <button onClick={dismissNotice} className="opacity-60 hover:opacity-100">
            <X className="size-4" />
          </button>
        </div>
      )}

      <div className="grid min-h-0 flex-1 grid-cols-2 gap-3">
        <Card className="flex min-h-0 flex-col">
          <textarea
            value={source}
            onChange={(e) => setSource(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && e.ctrlKey) run();
            }}
            placeholder="输入要翻译的中文或英文，Ctrl+Enter 翻译"
            className="min-h-0 flex-1 resize-none bg-transparent p-3 text-sm outline-none"
          />
        </Card>
        <Card className="flex min-h-0 flex-col">
          <div className="selectable min-h-0 flex-1 overflow-y-auto p-3 text-sm whitespace-pre-wrap">
            {result}
            {busy && !result && <Spinner className="opacity-50" />}
            {error && <div className="text-red-600 dark:text-red-400">{error}</div>}
          </div>
          <div className="flex h-9 items-center gap-2 border-t border-black/5 px-3 text-xs opacity-70 dark:border-white/5">
            {usedDir && <span>{DIR_LABEL[usedDir]}</span>}
            {elapsed !== null && <span>· {elapsed} ms</span>}
            <button
              className="ml-auto rounded p-1 hover:bg-black/5 disabled:opacity-40 dark:hover:bg-white/10"
              disabled={!result}
              title="复制译文"
              onClick={() => navigator.clipboard.writeText(result)}
            >
              <Copy className="size-4" />
            </button>
          </div>
        </Card>
      </div>

      <footer className="flex items-center gap-2">
        <ArrowRightLeft className="size-4 opacity-60" />
        <select
          value={dir}
          onChange={(e) => setDir(e.target.value as DirChoice)}
          className="h-8 rounded-md border border-black/10 bg-white/70 px-2 text-sm dark:border-white/10 dark:bg-neutral-800"
        >
          <option value="auto">自动检测方向</option>
          <option value="zh2En">{DIR_LABEL.zh2En}</option>
          <option value="en2Zh">{DIR_LABEL.en2Zh}</option>
        </select>
        <div className="ml-auto flex gap-2">
          {busy && (
            <Button icon={<Square className="size-3.5" />} onClick={() => api.cancelTranslate()}>
              停止
            </Button>
          )}
          <Button variant="primary" disabled={!source.trim()} onClick={run}>
            翻译
          </Button>
        </div>
      </footer>
    </div>
  );
}
