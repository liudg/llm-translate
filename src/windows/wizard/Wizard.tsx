// One-click deployment wizard (DESIGN 5.1):
// Welcome(+Detect) → (Customize) → Download → Verify → Launch → SelfTest → Done
// Redesigned with frameless editorial chrome, 2×2 hardware spec grid, rich model
// tier cards, download telemetry cockpit and interactive shortcut cheat-sheet.
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ArrowRight,
  Check,
  ChevronLeft,
  Cpu,
  FolderOpen,
  HardDrive,
  MemoryStick,
  MonitorCog,
  Pause,
  Play,
  RotateCcw,
  Settings2,
  Sparkles,
  X,
} from "lucide-react";
import {
  api,
  errorText,
  events,
  SELF_TEST_TEXT,
  type Backend,
  type DetectResult,
  type DownloadError,
  type HotkeyInfo,
  type LaunchOutcome,
  type ModelView,
} from "../../lib/api";
import { backendName, bytes, eta, gb, mbToGb, speed } from "../../lib/format";
import { Badge, BrandSeal, Button, Card, Kbd, Progress, Spinner, Toggle } from "../../components/ui";
import { TitleBar } from "../../components/TitleBar";

type Step = "welcome" | "customize" | "download" | "verify" | "launch" | "selftest" | "done";

interface Plan {
  modelId: string;
  backend: Backend;
  device: string | null;
  threads: number;
  dir: string;
}

interface Transfer {
  done: number;
  total: number;
  speedBps: number;
  source: string;
  note: string | null;
}

const STEPS: { key: Step; label: string }[] = [
  { key: "welcome", label: "检测" },
  { key: "download", label: "下载" },
  { key: "verify", label: "校验" },
  { key: "launch", label: "启动" },
  { key: "selftest", label: "自检" },
  { key: "done", label: "完成" },
];

function stepIndex(s: Step) {
  return s === "customize" ? 0 : STEPS.findIndex((x) => x.key === s);
}

export default function Wizard() {
  const [step, setStep] = useState<Step>("welcome");
  const [detect, setDetect] = useState<DetectResult | null>(null);
  const [detectError, setDetectError] = useState<string | null>(null);
  const [models, setModels] = useState<ModelView[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [diskFree, setDiskFree] = useState<number | null>(null);
  const [transfer, setTransfer] = useState<Transfer | null>(null);
  const [verify, setVerify] = useState(0);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fallback, setFallback] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<LaunchOutcome | null>(null);
  const [wasDeployed, setWasDeployed] = useState(true);
  const running = useRef(false);

  const runDetect = useCallback(async () => {
    setDetect(null);
    setDetectError(null);
    try {
      const d = await api.detect();
      setDetect(d);
      const r = d.recommendation;
      setPlan({ modelId: r.modelId, backend: r.backend, device: r.device, threads: r.threads, dir: d.modelsDir });
    } catch (e) {
      setDetectError(errorText(e));
    }
  }, []);

  const reset = useCallback(() => {
    if (running.current) return;
    setStep("welcome");
    setError(null);
    setFallback(null);
    setOutcome(null);
    setTransfer(null);
    setPaused(false);
    api.getSettings().then((s) => setWasDeployed(s.deployed));
    runDetect();
  }, [runDetect]);

  useEffect(() => {
    reset();
    const un = listen("wizard-reset", reset);
    return () => {
      un.then((f) => f());
    };
  }, [reset]);

  useEffect(() => {
    if (!plan) return;
    api.models(plan.dir).then(setModels).catch(() => setModels([]));
    api.diskFree(plan.dir).then(setDiskFree).catch(() => setDiskFree(null));
  }, [plan?.dir, step]);

  useEffect(() => {
    const unDl = events.download((e) => {
      if (e.phase === "download") {
        setStep("download");
        setTransfer({ done: e.done, total: e.total, speedBps: e.speedBps, source: e.source, note: null });
      } else if (e.phase === "retry") {
        setTransfer((t) => (t ? { ...t, speedBps: 0, note: `${e.message}，${e.waitSecs} 秒后重试` } : t));
      } else if (e.phase === "verify") {
        setStep("verify");
        setVerify(e.total ? e.done / e.total : 0);
      }
    });
    const unLaunch = events.launch((e) => {
      if (e.phase === "starting") setStep("launch");
      if (e.phase === "selftest") setStep("selftest");
      if (e.phase === "fallback") {
        setFallback(e.message);
        setStep("launch");
      }
    });
    return () => {
      unDl.then((f) => f());
      unLaunch.then((f) => f());
    };
  }, []);

  const deploy = useCallback(async (p: Plan) => {
    if (running.current) return;
    running.current = true;
    setError(null);
    setPaused(false);
    setFallback(null);
    setStep("download");
    try {
      try {
        await api.download(p.modelId, p.dir);
      } catch (e) {
        const err = e as DownloadError;
        if (err?.kind === "paused") {
          setPaused(true);
          return;
        }
        if (err?.kind === "cancelled") {
          setTransfer(null);
          setStep("welcome");
          return;
        }
        throw e;
      }
      setStep("launch");
      const out = await api.launch({
        modelId: p.modelId,
        dir: p.dir,
        backend: p.backend,
        device: p.device,
        threads: p.threads,
      });
      setOutcome(out);
      setStep("done");
    } catch (e) {
      setError(errorText(e));
    } finally {
      running.current = false;
    }
  }, []);

  const model = models.find((m) => m.id === plan?.modelId);
  const idx = stepIndex(step);

  return (
    <div className="app-shell flex h-full flex-col overflow-hidden">
      <TitleBar
        maximizable={false}
        left={
          <div data-tauri-drag-region className="flex items-center gap-2">
            <BrandSeal size="sm" className="pointer-events-none" />
            <span className="pointer-events-none text-body font-semibold tracking-tight text-[var(--ink)]">
              LLM Translate
            </span>
            <span className="pointer-events-none text-caption text-[var(--ink-muted)]">· 一键部署</span>
          </div>
        }
      />

      {/* Editorial hero & stepper header */}
      <div className="shrink-0 border-b border-[var(--border-subtle)] bg-[var(--bg-surface)]/55 px-7 pt-5 pb-4">
        <div className="mb-4 flex items-baseline justify-between gap-4">
          <div>
            <h1 className="text-large font-semibold tracking-tight text-[var(--ink)]">
              本地离线翻译引擎部署
            </h1>
            <p className="mt-0.5 text-caption text-[var(--ink-secondary)]">
              离线运行，翻译内容仅保存在本地
            </p>
          </div>
        </div>

        {/* 6-step connected progress rail */}
        <ol className="flex items-center gap-1.5 text-caption">
          {STEPS.map((s, i) => {
            const completed = i < idx || step === "done";
            const current = i === idx && step !== "done";
            return (
              <li key={s.key} className={`flex items-center gap-1.5 ${i < STEPS.length - 1 ? "flex-1" : ""}`}>
                <span
                  className={`flex size-5 shrink-0 items-center justify-center rounded-full text-micro font-semibold transition-colors ${
                    completed
                      ? "bg-[var(--accent)] text-[var(--ink-inverse)]"
                      : current
                        ? "border-2 border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent-text)]"
                        : "border border-[var(--border-strong)] text-[var(--ink-muted)]"
                  }`}
                >
                  {completed ? <Check className="size-3" /> : i + 1}
                </span>
                <span
                  className={`shrink-0 ${
                    current
                      ? "font-semibold text-[var(--ink)]"
                      : completed
                        ? "text-[var(--ink-secondary)]"
                        : "text-[var(--ink-muted)]"
                  }`}
                >
                  {s.label}
                </span>
                {i < STEPS.length - 1 && (
                  <span
                    className={`h-px flex-1 transition-colors ${
                      i < idx || step === "done" ? "bg-[var(--accent)]/60" : "bg-[var(--border)]"
                    }`}
                  />
                )}
              </li>
            );
          })}
        </ol>
      </div>

      {/* Step Content */}
      <main className="min-h-0 flex-1 overflow-y-auto px-7 py-5">
        {error ? (
          <ErrorPanel
            message={error}
            onRetry={() => plan && deploy(plan)}
            onCustomize={() => {
              setError(null);
              setStep("customize");
            }}
          />
        ) : step === "welcome" ? (
          <Welcome
            detect={detect}
            detectError={detectError}
            plan={plan}
            model={model}
            diskFree={diskFree}
            onRetryDetect={runDetect}
            onStart={() => plan && deploy(plan)}
            onCustomize={() => setStep("customize")}
          />
        ) : step === "customize" && plan && detect ? (
          <Customize
            detect={detect}
            models={models}
            plan={plan}
            diskFree={diskFree}
            onChange={setPlan}
            onBack={() => setStep("welcome")}
            onStart={() => deploy(plan)}
          />
        ) : step === "download" ? (
          <DownloadPanel
            model={model}
            transfer={transfer}
            paused={paused}
            onPause={() => plan && api.downloadControl(plan.modelId, "pause")}
            onResume={() => plan && deploy(plan)}
            onCancel={() => {
              if (!plan) return;
              if (paused) {
                setPaused(false);
                setTransfer(null);
                setStep("welcome");
              } else {
                api.downloadControl(plan.modelId, "cancel");
              }
            }}
          />
        ) : step === "verify" ? (
          <Centered
            title="校验模型文件"
            detail="正在检查文件完整性，请稍候…"
          >
            <div className="mb-2 text-large font-semibold text-[var(--ink)] tabular-nums">
              {Math.min(100, Math.round(verify * 100))}%
            </div>
            <Progress value={verify} />
          </Centered>
        ) : step === "launch" || step === "selftest" ? (
          <Centered
            title={step === "launch" ? "启动翻译引擎" : "测试翻译效果"}
            detail={
              step === "launch"
                ? `正在通过 ${backendName(fallback ? "cpu" : (plan?.backend ?? "cpu"))} 加载模型，请稍候…`
                : "正在进行首次翻译测试…"
            }
          >
            <Progress value={0} indeterminate />
            {fallback && (
              <div className="mt-5 flex items-start gap-2.5 rounded-xl border border-[var(--warn)]/25 bg-[var(--warn-soft)] p-3.5 text-left text-caption leading-relaxed text-[var(--warn)]">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" />
                <span>GPU 启动失败，已自动回退为 CPU 模式继续部署。原因：{fallback}</span>
              </div>
            )}
          </Centered>
        ) : step === "done" && outcome ? (
          <Done
            outcome={outcome}
            model={model}
            firstDeploy={!wasDeployed}
            onFinish={(autostart) => api.finish(autostart)}
          />
        ) : (
          <Centered title="请稍候" detail="正在准备部署环境">
            <div className="flex justify-center">
              <Spinner className="size-5 text-[var(--accent)]" />
            </div>
          </Centered>
        )}
      </main>
    </div>
  );
}

function Centered({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mx-auto flex max-w-md flex-col items-center pt-12 text-center">
      <h2 className="text-large font-semibold text-[var(--ink)]">{title}</h2>
      {detail && <p className="mt-1 mb-6 text-caption leading-relaxed text-[var(--ink-secondary)]">{detail}</p>}
      <div className="w-full">{children}</div>
    </div>
  );
}

function SpecTile({
  icon,
  label,
  value,
  sub,
}: {
  icon: React.ReactNode;
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 shadow-[var(--shadow-card)]">
      <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[var(--bg-recessed)] text-[var(--ink-secondary)]">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-caption font-medium text-[var(--ink-muted)]">{label}</div>
        <div className="mt-0.5 truncate text-body font-medium text-[var(--ink)]">{value}</div>
        {sub && <div className="mt-0.5 truncate text-caption text-[var(--ink-secondary)] tabular-nums">{sub}</div>}
      </div>
    </div>
  );
}

function needBytes(model: ModelView | undefined) {
  if (!model || model.installed) return 0;
  return model.size - model.downloadedBytes + model.size * 0.2;
}

function Welcome(props: {
  detect: DetectResult | null;
  detectError: string | null;
  plan: Plan | null;
  model: ModelView | undefined;
  diskFree: number | null;
  onRetryDetect: () => void;
  onStart: () => void;
  onCustomize: () => void;
}) {
  const { detect, detectError, plan, model, diskFree } = props;
  if (detectError) {
    return <ErrorPanel message={`硬件检测失败：${detectError}`} onRetry={props.onRetryDetect} />;
  }
  if (!detect || !plan) {
    return (
      <Centered title="正在检测硬件" detail="正在匹配适合你电脑的翻译模型…">
        <Progress value={0} indeterminate />
      </Centered>
    );
  }
  const hw = detect.hardware;
  const rec = detect.recommendation;
  const lowDisk = diskFree !== null && diskFree < needBytes(model);
  const warnings = [
    ...rec.warnings,
    ...(lowDisk ? ["模型目录所在磁盘空间不足，请点击「自定义」更换存放位置"] : []),
  ];

  return (
    <div className="flex flex-col gap-4">
      {/* 2×2 Hardware Spec Grid */}
      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        <SpecTile
          icon={<Cpu className="size-4" />}
          label="处理器"
          value={hw.cpu.name}
          sub={`${hw.cpu.physicalCores} 核 ${hw.cpu.logicalCores} 线程`}
        />
        <SpecTile
          icon={<MemoryStick className="size-4" />}
          label="系统内存"
          value={mbToGb(hw.ramTotalMb)}
          sub={`当前可用 ${mbToGb(hw.ramAvailableMb)}`}
        />
        <SpecTile
          icon={<MonitorCog className="size-4" />}
          label="显卡"
          value={
            hw.gpus.length > 0 ? hw.gpus[0].name : "未检测到可用显卡"
          }
          sub={
            hw.gpus.length > 0
              ? hw.gpus
                  .map((g) => (g.integrated ? "核显" : `可用显存 ${mbToGb(g.freeMb)}`))
                  .join(" · ")
              : "将使用 CPU 运行"
          }
        />
        <SpecTile
          icon={<HardDrive className="size-4" />}
          label="模型目录"
          value={
            <span className="selectable font-mono text-caption" title={plan.dir}>
              {plan.dir}
            </span>
          }
          sub={diskFree !== null ? `磁盘剩余 ${gb(diskFree, 1)}` : "无法读取剩余空间"}
        />
      </div>

      {/* Recommended Plan Hero Card */}
      <div className="relative overflow-hidden rounded-xl border border-[var(--accent)]/35 bg-[var(--bg-surface)] p-4 shadow-[var(--shadow-card)]">
        <span className="absolute inset-y-0 left-0 w-1 bg-[var(--accent)]" />
        <div className="pl-2">
          <div className="mb-1.5 flex items-center gap-1.5 text-caption font-semibold text-[var(--accent-text)]">
            <Sparkles className="size-3.5" />
            <span>硬件匹配推荐方案</span>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-large font-semibold text-[var(--ink)]">
              {model?.name ?? rec.modelId}
            </span>
            {model && <Badge>{gb(model.size)}</Badge>}
            <Badge tone="accent">{backendName(plan.backend)}</Badge>
            {model?.installed && <Badge tone="good">已下载</Badge>}
            {model && !model.installed && model.downloadedBytes > 0 && (
              <Badge tone="warn">已下载 {bytes(model.downloadedBytes)}，支持断点续传</Badge>
            )}
          </div>
          <p className="mt-1.5 text-caption leading-relaxed text-[var(--ink-secondary)]">{rec.reason}</p>

          {warnings.map((w) => (
            <p
              key={w}
              className="mt-2.5 flex items-center gap-1.5 rounded-lg bg-[var(--warn-soft)] px-2.5 py-1.5 text-caption text-[var(--warn)]"
            >
              <AlertTriangle className="size-3.5 shrink-0" />
              <span>{w}</span>
            </p>
          ))}
        </div>
      </div>

      {/* Footer Actions */}
      <div className="flex items-center justify-end gap-2 pt-1">
        <Button icon={<Settings2 className="size-3.5" />} onClick={props.onCustomize}>
          自定义配置
        </Button>
        <Button variant="primary" disabled={lowDisk || !model} onClick={props.onStart}>
          <span>开始一键部署</span>
          <ArrowRight className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}

function Customize(props: {
  detect: DetectResult;
  models: ModelView[];
  plan: Plan;
  diskFree: number | null;
  onChange: (p: Plan) => void;
  onBack: () => void;
  onStart: () => void;
}) {
  const { detect, models, plan, onChange, diskFree } = props;
  const hw = detect.hardware;
  const ramGb = hw.ramTotalMb / 1024;
  const gpu = hw.gpus.find((g) => g.id === plan.device);
  const model = models.find((m) => m.id === plan.modelId);
  const lowDisk = diskFree !== null && diskFree < needBytes(model);

  const pickDir = async () => {
    const dir = await openDialog({ directory: true, defaultPath: plan.dir, title: "选择模型存放目录" });
    if (typeof dir === "string") onChange({ ...plan, dir });
  };

  return (
    <div className="flex flex-col gap-4">
      <section>
        <h3 className="mb-2 text-caption font-semibold tracking-wide text-[var(--ink-secondary)]">选择模型档位</h3>
        <div className="flex flex-col gap-2">
          {models.map((m) => {
            const selected = m.id === plan.modelId;
            const vramShort = plan.backend === "vulkan" && gpu && gpu.freeMb / 1024 < m.recommendVramGB - 0.5;
            return (
              <button
                key={m.id}
                type="button"
                onClick={() => onChange({ ...plan, modelId: m.id })}
                className={`flex cursor-pointer items-center gap-3 rounded-xl border p-3.5 text-left transition-all ${
                  selected
                    ? "border-[var(--accent)] bg-[var(--accent-soft)] ring-1 ring-[var(--accent)]/25"
                    : "border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)]"
                }`}
              >
                <span
                  className={`flex size-4 shrink-0 items-center justify-center rounded-full border ${
                    selected
                      ? "border-[var(--accent)] bg-[var(--accent)] text-[var(--ink-inverse)]"
                      : "border-[var(--border-strong)]"
                  }`}
                >
                  {selected && <Check className="size-2.5" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5 text-body">
                    <span className="font-semibold text-[var(--ink)]">{m.name}</span>
                    <Badge>{gb(m.size)}</Badge>
                    {m.id === detect.recommendation.modelId && <Badge tone="accent">推荐</Badge>}
                    {m.installed && <Badge tone="good">已下载</Badge>}
                    {!m.installed && m.downloadedBytes > 0 && <Badge tone="warn">可续传</Badge>}
                  </div>
                  <div className="mt-1 text-caption text-[var(--ink-secondary)] tabular-nums">
                    建议显存 {m.recommendVramGB} GB · 最低内存 {m.minRamGB} GB
                    {ramGb < m.minRamGB && <span className="ml-1.5 font-medium text-[var(--warn)]">· 内存偏紧</span>}
                    {vramShort && <span className="ml-1.5 font-medium text-[var(--warn)]">· 显存可能不足</span>}
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-caption font-semibold tracking-wide text-[var(--ink-secondary)]">推理后端</h3>
        <div className="flex flex-wrap gap-2">
          {hw.gpus.map((g) => (
            <Choice
              key={g.id}
              active={plan.backend === "vulkan" && plan.device === g.id}
              onClick={() => onChange({ ...plan, backend: "vulkan", device: g.id })}
            >
              GPU · {g.name}
              {g.integrated ? "（核显）" : `（${mbToGb(g.freeMb)}）`}
            </Choice>
          ))}
          <Choice active={plan.backend === "cpu"} onClick={() => onChange({ ...plan, backend: "cpu", device: null })}>
            CPU · {plan.threads} 线程
          </Choice>
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-caption font-semibold tracking-wide text-[var(--ink-secondary)]">模型存放目录</h3>
        <div className="flex items-center gap-2">
          <div className="selectable min-w-0 flex-1 truncate rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 font-mono text-caption text-[var(--ink)]">
            {plan.dir}
          </div>
          <Button size="sm" icon={<FolderOpen className="size-3.5" />} onClick={pickDir}>
            更改目录
          </Button>
        </div>
        <p className={`mt-1.5 text-caption tabular-nums ${lowDisk ? "text-[var(--warn)]" : "text-[var(--ink-muted)]"}`}>
          {diskFree !== null ? `磁盘剩余空间 ${gb(diskFree, 1)}` : "无法读取剩余空间"}
          {lowDisk && model && `，当前模型至少需要 ${gb(needBytes(model), 1)}`}
        </p>
      </section>

      <div className="flex items-center justify-between pt-2">
        <Button variant="subtle" icon={<ChevronLeft className="size-4" />} onClick={props.onBack}>
          返回推荐方案
        </Button>
        <Button variant="primary" disabled={lowDisk || !model} onClick={props.onStart}>
          <span>开始部署</span>
          <ArrowRight className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}

function Choice({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`cursor-pointer rounded-lg border px-3 py-1.5 text-caption font-medium transition-colors ${
        active
          ? "border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent-text)]"
          : "border-[var(--border)] bg-[var(--bg-surface)] text-[var(--ink-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--ink)]"
      }`}
    >
      {children}
    </button>
  );
}

function DownloadPanel(props: {
  model: ModelView | undefined;
  transfer: Transfer | null;
  paused: boolean;
  onPause: () => void;
  onResume: () => void;
  onCancel: () => void;
}) {
  const { model, transfer, paused } = props;
  const total = transfer?.total ?? model?.size ?? 0;
  const done = transfer?.done ?? model?.downloadedBytes ?? 0;
  const ratio = total ? done / total : 0;
  const pct = Math.min(100, Math.round(ratio * 100));

  return (
    <Centered
      title={`下载 ${model?.name ?? "模型"}`}
      detail={paused ? "下载已暂停，可随时继续" : "正在自动选择较快的下载源"}
    >
      <Card className="flex flex-col gap-3 p-5 text-left">
        <div className="flex items-baseline justify-between">
          <span className="text-large font-semibold text-[var(--ink)] tabular-nums">{pct}%</span>
          {transfer && !paused && <Badge tone="neutral">{transfer.source}</Badge>}
        </div>

        <Progress value={ratio} indeterminate={!transfer && !paused} />

        <div className="flex flex-wrap items-center justify-between gap-2 text-caption text-[var(--ink-secondary)] tabular-nums">
          <span>
            {bytes(done)} / {bytes(total)}
          </span>
          {transfer && !paused && (
            <span>
              {transfer.note ?? `${speed(transfer.speedBps)} · 预计剩余 ${eta(total - done, transfer.speedBps)}`}
            </span>
          )}
          {!transfer && !paused && <span>正在测试最快下载源…</span>}
          {paused && <span>已暂停</span>}
        </div>
      </Card>

      <div className="mt-5 flex justify-center gap-2">
        {paused ? (
          <Button variant="primary" icon={<Play className="size-3.5" />} onClick={props.onResume}>
            继续下载
          </Button>
        ) : (
          <Button icon={<Pause className="size-3.5" />} onClick={props.onPause} disabled={!transfer}>
            暂停
          </Button>
        )}
        <Button variant="subtle" icon={<X className="size-3.5" />} onClick={props.onCancel}>
          {paused ? "返回上一步" : "取消下载"}
        </Button>
      </div>
    </Centered>
  );
}

function ErrorPanel({
  message,
  onRetry,
  onCustomize,
}: {
  message: string;
  onRetry: () => void;
  onCustomize?: () => void;
}) {
  return (
    <div className="mx-auto flex max-w-lg flex-col items-center pt-10 text-center">
      <div className="mb-3 flex size-11 items-center justify-center rounded-full bg-[var(--danger-soft)] text-[var(--danger)]">
        <AlertTriangle className="size-5" />
      </div>
      <h2 className="text-large font-semibold text-[var(--ink)]">部署未能完成</h2>
      <p className="selectable mt-2 mb-6 rounded-xl border border-[var(--danger)]/20 bg-[var(--danger-soft)] px-4 py-3 text-left text-caption leading-relaxed break-all whitespace-pre-wrap text-[var(--danger)]">
        {message}
      </p>
      <div className="flex flex-wrap justify-center gap-2">
        <Button variant="primary" icon={<RotateCcw className="size-3.5" />} onClick={onRetry}>
          重试部署
        </Button>
        {onCustomize && (
          <Button icon={<Settings2 className="size-3.5" />} onClick={onCustomize}>
            更换模型或后端
          </Button>
        )}
        <Button variant="subtle" onClick={() => api.openLogsDir()}>
          查看日志
        </Button>
      </div>
    </div>
  );
}

function ShortcutTile({
  info,
  loaded,
  label,
}: {
  info: HotkeyInfo | undefined;
  loaded: boolean;
  label: string;
}) {
  const keys = info?.keys ?? null;
  const conflict = Boolean(keys && info && !info.registered);
  return (
    <div
      className={`rounded-lg border bg-[var(--bg-surface)] px-3 py-2 ${
        conflict ? "border-[var(--warn)]/35" : "border-[var(--border)]"
      }`}
    >
      <div className="flex min-h-[18px] items-center">
        {!loaded ? (
          <span className="inline-block h-[18px] w-12 rounded-md bg-[var(--bg-recessed)]" />
        ) : keys ? (
          <Kbd keys={keys} compact dim={conflict} />
        ) : (
          <span className="inline-flex h-[18px] items-center rounded-md bg-[var(--bg-recessed)] px-1.5 text-caption font-medium text-[var(--ink-muted)]">
            未绑定
          </span>
        )}
      </div>
      <div className="mt-1 text-caption text-[var(--ink-secondary)]">{label}</div>
      {conflict && (
        <div
          className="mt-0.5 text-caption leading-tight text-[var(--warn)]"
          title={info?.error ?? undefined}
        >
          未生效 · 可在设置中更换
        </div>
      )}
    </div>
  );
}

function Done({
  outcome,
  model,
  firstDeploy,
  onFinish,
}: {
  outcome: LaunchOutcome;
  model: ModelView | undefined;
  firstDeploy: boolean;
  onFinish: (autostart: boolean | null) => void;
}) {
  const [autostart, setAutostart] = useState<boolean | null>(firstDeploy ? true : null);
  const [hotkeys, setHotkeys] = useState<HotkeyInfo[] | null>(null);
  useEffect(() => {
    if (!firstDeploy) api.autostart().then(setAutostart);
  }, [firstDeploy]);
  useEffect(() => {
    let active = true;
    api
      .hotkeys()
      .then((list) => {
        if (active) setHotkeys(list);
      })
      .catch(() => {
        if (active) setHotkeys([]);
      });
    const un = events.hotkeys((list) => {
      if (active) setHotkeys(list);
    });
    return () => {
      active = false;
      un.then((f) => f());
    };
  }, []);

  const miniHotkey = hotkeys?.find((h) => h.id === "mini");
  const shotHotkey = hotkeys?.find((h) => h.id === "shot");

  return (
    <div className="mx-auto flex max-w-lg flex-col items-center pt-4 text-center">
      <div className="mb-2.5 flex size-11 items-center justify-center rounded-full bg-[var(--good-soft)] text-[var(--good)]">
        <Check className="size-5" />
      </div>
      <h2 className="text-large font-semibold text-[var(--ink)]">本地引擎部署完成</h2>
      <p className="mt-1 mb-4 text-caption text-[var(--ink-secondary)] tabular-nums">
        {model?.name} · {backendName(outcome.backend)} · 启动耗时 {(outcome.loadMs / 1000).toFixed(1)} 秒
      </p>

      {/* Bilingual self-test card */}
      <Card className="w-full overflow-hidden text-left">
        <div className="flex items-center justify-between border-b border-[var(--border-subtle)] bg-[var(--bg-surface-alt)] px-4 py-2 text-caption text-[var(--ink-muted)]">
          <span>试译结果</span>
          <span className="font-mono tabular-nums">{outcome.translateMs} ms</span>
        </div>
        <div className="grid grid-cols-2 divide-x divide-[var(--border-subtle)] text-body">
          <div className="px-4 py-3 text-[var(--ink-secondary)]">{SELF_TEST_TEXT}</div>
          <div className="selectable px-4 py-3 font-medium text-[var(--ink)]">{outcome.sample}</div>
        </div>
      </Card>

      {/* Quick-start shortcuts overview */}
      <div className="mt-3 grid w-full grid-cols-3 gap-2 text-left">
        <ShortcutTile info={miniHotkey} loaded={hotkeys !== null} label="随时呼出迷你窗" />
        <ShortcutTile info={shotHotkey} loaded={hotkeys !== null} label="框选截图覆盖译文" />
        <div className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2">
          <span className="inline-flex h-[18px] items-center rounded-md bg-[var(--accent)] px-1.5 text-caption font-semibold text-[var(--ink-inverse)]">
            译
          </span>
          <div className="mt-1 text-caption text-[var(--ink-secondary)]">选中文字点击浮标</div>
        </div>
      </div>

      {outcome.fallbackReason && (
        <div className="mt-3 flex w-full items-start gap-2 rounded-xl border border-[var(--warn)]/25 bg-[var(--warn-soft)] p-3 text-left text-caption leading-relaxed text-[var(--warn)]">
          <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          <span>
            GPU 启动失败，已改用 CPU 并保存。原因：{outcome.fallbackReason}
            {model && model.size > 2e9 && "。该模型在 CPU 上较慢，建议改用「轻量 · Index-Translate 2B」。"}
          </span>
        </div>
      )}

      <div className="mt-4 flex w-full items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2.5 text-left">
        <div>
          <div className="text-caption font-medium text-[var(--ink)]">开机自动启动</div>
          <div className="text-caption text-[var(--ink-muted)]">开机后在后台自动运行</div>
        </div>
        <Toggle
          checked={autostart ?? false}
          disabled={autostart === null}
          onChange={setAutostart}
        />
      </div>

      <Button variant="primary" className="mt-4 px-6" onClick={() => onFinish(autostart)}>
        <span>开始使用</span>
        <ArrowRight className="size-3.5" />
      </Button>
    </div>
  );
}
