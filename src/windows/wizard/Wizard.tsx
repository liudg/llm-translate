// One-click deployment wizard (DESIGN 5.1):
// Welcome(+Detect) → (Customize) → Download → Verify → Launch → SelfTest → Done
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
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
  X,
  Zap,
} from "lucide-react";
import {
  api,
  errorText,
  events,
  type Backend,
  type DetectResult,
  type DownloadError,
  type LaunchOutcome,
  type ModelView,
} from "../../lib/api";
import { backendName, bytes, eta, gb, mbToGb, speed } from "../../lib/format";
import { Badge, Button, Card, Progress, Spinner } from "../../components/ui";

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
  /** Deployed before this wizard run (re-deploy from settings). */
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
    // The window is pre-created hidden; when already deployed it only detects
    // once opened again (wizard-reset).
    api.getSettings().then((s) => {
      setWasDeployed(s.deployed);
      if (!s.deployed) runDetect();
    });
    const un = listen("wizard-reset", reset);
    return () => {
      un.then((f) => f());
    };
  }, [runDetect, reset]);

  // Refresh model list / free space when the target directory changes.
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
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 px-8 pt-7 pb-5">
        <div className="flex size-9 items-center justify-center rounded-lg bg-[var(--accent)] text-white dark:text-black">
          <Zap className="size-5" />
        </div>
        <div>
          <h1 className="text-lg font-semibold">一键部署本地翻译模型</h1>
          <p className="text-xs opacity-60">模型在本机运行，翻译内容不会离开这台电脑</p>
        </div>
      </header>

      <ol className="flex items-center gap-2 px-8 pb-5 text-xs">
        {STEPS.map((s, i) => (
          <li key={s.key} className="flex items-center gap-2">
            <span
              className={`flex size-5 items-center justify-center rounded-full text-[11px] ${
                i < idx || step === "done"
                  ? "bg-[var(--accent)] text-white dark:text-black"
                  : i === idx
                    ? "border-2 border-[var(--accent)] text-[var(--accent-text)]"
                    : "border border-black/20 opacity-60 dark:border-white/25"
              }`}
            >
              {i < idx || step === "done" ? <Check className="size-3" /> : i + 1}
            </span>
            <span className={i === idx ? "font-medium" : "opacity-60"}>{s.label}</span>
            {i < STEPS.length - 1 && <span className="h-px w-6 bg-black/15 dark:bg-white/15" />}
          </li>
        ))}
      </ol>

      <main className="min-h-0 flex-1 overflow-y-auto px-8 pb-6">
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
                // Leave the paused download: the .part file stays for a later resume.
                setPaused(false);
                setTransfer(null);
                setStep("welcome");
              } else {
                api.downloadControl(plan.modelId, "cancel");
              }
            }}
          />
        ) : step === "verify" ? (
          <Centered title="校验文件完整性" detail="正在计算 SHA256，确认模型文件完整无损">
            <Progress value={verify} />
          </Centered>
        ) : step === "launch" || step === "selftest" ? (
          <Centered
            title={step === "launch" ? "启动推理服务" : "自检"}
            detail={
              step === "launch"
                ? `正在以 ${backendName(fallback ? "cpu" : (plan?.backend ?? "cpu"))} 加载模型，首次启动可能需要十几秒`
                : "正在翻译「你好，世界」验证模型可用"
            }
          >
            <Progress value={0} indeterminate />
            {fallback && (
              <p className="mt-4 flex items-start gap-2 rounded-md bg-amber-500/10 p-3 text-left text-xs text-amber-800 dark:text-amber-300">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" />
                <span>GPU 启动失败，已自动改用 CPU。原因：{fallback}</span>
              </p>
            )}
          </Centered>
        ) : step === "done" && outcome ? (
          <Done outcome={outcome} model={model} firstDeploy={!wasDeployed} onFinish={(autostart) => api.finish(autostart)} />
        ) : (
          <Centered title="请稍候" detail="">
            <Spinner />
          </Centered>
        )}
      </main>
    </div>
  );
}

function Centered({ title, detail, children }: { title: string; detail: string; children: React.ReactNode }) {
  return (
    <div className="mx-auto flex max-w-md flex-col items-center pt-16 text-center">
      <h2 className="mb-1 text-base font-semibold">{title}</h2>
      <p className="mb-6 text-sm opacity-60">{detail}</p>
      <div className="w-full">{children}</div>
    </div>
  );
}

function Row({ icon, label, value }: { icon: React.ReactNode; label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3 py-1.5 text-sm">
      <span className="opacity-60">{icon}</span>
      <span className="w-14 shrink-0 opacity-60">{label}</span>
      <span className="min-w-0 truncate">{value}</span>
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
      <Centered title="正在检测硬件" detail="CPU、内存、显卡与磁盘空间">
        <Progress value={0} indeterminate />
      </Centered>
    );
  }
  const hw = detect.hardware;
  const rec = detect.recommendation;
  const lowDisk = diskFree !== null && diskFree < needBytes(model);
  return (
    <div className="flex flex-col gap-4">
      <Card className="px-4 py-2">
        <Row icon={<Cpu className="size-4" />} label="CPU" value={`${hw.cpu.name}（${hw.cpu.physicalCores} 核）`} />
        <Row
          icon={<MemoryStick className="size-4" />}
          label="内存"
          value={`${mbToGb(hw.ramTotalMb)}（可用 ${mbToGb(hw.ramAvailableMb)}）`}
        />
        <Row
          icon={<MonitorCog className="size-4" />}
          label="显卡"
          value={
            hw.gpus.length
              ? hw.gpus
                  .map((g) => `${g.name}（${g.integrated ? "核显" : `可用显存 ${mbToGb(g.freeMb)}`}）`)
                  .join("；")
              : "未检测到可用于 Vulkan 的显卡"
          }
        />
        <Row
          icon={<HardDrive className="size-4" />}
          label="模型目录"
          value={
            <span className="selectable">
              {plan.dir}
              {diskFree !== null && <span className="opacity-60">（剩余 {gb(diskFree, 1)}）</span>}
            </span>
          }
        />
      </Card>

      <Card className="p-4">
        <div className="mb-2 text-xs opacity-60">推荐方案</div>
        <div className="flex items-center gap-2">
          <span className="font-semibold">{model?.name ?? rec.modelId}</span>
          {model && <Badge>{gb(model.size)}</Badge>}
          <Badge tone="accent">{backendName(plan.backend)}</Badge>
          {model?.installed && <Badge tone="good">已下载</Badge>}
          {model && !model.installed && model.downloadedBytes > 0 && (
            <Badge tone="warn">已下载 {bytes(model.downloadedBytes)}，可续传</Badge>
          )}
        </div>
        <p className="mt-1.5 text-sm opacity-70">{rec.reason}</p>
        {[...rec.warnings, ...(lowDisk ? ["模型目录所在磁盘空间不足，请在“自定义”中更换目录"] : [])].map((w) => (
          <p key={w} className="mt-2 flex items-center gap-1.5 text-xs text-amber-700 dark:text-amber-400">
            <AlertTriangle className="size-3.5" />
            {w}
          </p>
        ))}
      </Card>

      <div className="mt-2 flex justify-end gap-2">
        <Button icon={<Settings2 className="size-4" />} onClick={props.onCustomize}>
          自定义
        </Button>
        <Button variant="primary" disabled={lowDisk || !model} onClick={props.onStart}>
          开始部署
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
        <h3 className="mb-2 text-sm font-medium">模型</h3>
        <div className="flex flex-col gap-2">
          {models.map((m) => {
            const selected = m.id === plan.modelId;
            const vramShort = plan.backend === "vulkan" && gpu && gpu.freeMb / 1024 < m.recommendVramGB - 0.5;
            return (
              <button
                key={m.id}
                onClick={() => onChange({ ...plan, modelId: m.id })}
                className={`rounded-lg border p-3 text-left transition-colors ${
                  selected
                    ? "border-[var(--accent)] bg-[var(--accent)]/[0.07]"
                    : "border-black/10 bg-white/50 hover:bg-white/80 dark:border-white/10 dark:bg-white/[0.03] dark:hover:bg-white/[0.07]"
                }`}
              >
                <div className="flex items-center gap-2 text-sm">
                  <span className="font-medium">{m.name}</span>
                  <Badge>{gb(m.size)}</Badge>
                  {m.id === detect.recommendation.modelId && <Badge tone="accent">推荐</Badge>}
                  {m.installed && <Badge tone="good">已下载</Badge>}
                  {!m.installed && m.downloadedBytes > 0 && <Badge tone="warn">可续传</Badge>}
                </div>
                <div className="mt-1 text-xs opacity-60">
                  建议显存 {m.recommendVramGB} GB · 最低内存 {m.minRamGB} GB
                  {ramGb < m.minRamGB && <span className="text-amber-600"> · 内存不足</span>}
                  {vramShort && <span className="text-amber-600"> · 显存可能不足</span>}
                </div>
              </button>
            );
          })}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-sm font-medium">推理后端</h3>
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
        <h3 className="mb-2 text-sm font-medium">模型目录</h3>
        <div className="flex items-center gap-2">
          <div className="selectable min-w-0 flex-1 truncate rounded-md border border-black/10 bg-white/60 px-3 py-1.5 text-sm dark:border-white/10 dark:bg-white/[0.04]">
            {plan.dir}
          </div>
          <Button icon={<FolderOpen className="size-4" />} onClick={pickDir}>
            更改
          </Button>
        </div>
        <p className={`mt-1 text-xs ${lowDisk ? "text-amber-700 dark:text-amber-400" : "opacity-60"}`}>
          {diskFree !== null ? `剩余空间 ${gb(diskFree, 1)}` : "无法读取剩余空间"}
          {lowDisk && model && `，至少需要 ${gb(needBytes(model), 1)}`}
        </p>
      </section>

      <div className="mt-2 flex justify-between">
        <Button variant="subtle" icon={<ChevronLeft className="size-4" />} onClick={props.onBack}>
          返回
        </Button>
        <Button variant="primary" disabled={lowDisk || !model} onClick={props.onStart}>
          开始部署
        </Button>
      </div>
    </div>
  );
}

function Choice({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md border px-3 py-1.5 text-sm transition-colors ${
        active
          ? "border-[var(--accent)] bg-[var(--accent)]/[0.08]"
          : "border-black/10 bg-white/50 hover:bg-white/80 dark:border-white/10 dark:bg-white/[0.03] dark:hover:bg-white/[0.07]"
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
  return (
    <Centered title={`下载 ${model?.name ?? "模型"}`} detail={paused ? "已暂停，可随时继续（断点续传）" : "自动选择最快的下载源"}>
      <Progress value={total ? done / total : 0} indeterminate={!transfer && !paused} />
      <div className="mt-2 flex justify-between text-xs opacity-70">
        <span>
          {bytes(done)} / {bytes(total)}
        </span>
        {transfer && !paused && (
          <span>
            {transfer.note ?? `${speed(transfer.speedBps)} · 剩余 ${eta(total - done, transfer.speedBps)} · ${transfer.source}`}
          </span>
        )}
        {!transfer && !paused && <span>正在测试下载源…</span>}
      </div>
      <div className="mt-6 flex justify-center gap-2">
        {paused ? (
          <Button variant="primary" icon={<Play className="size-4" />} onClick={props.onResume}>
            继续
          </Button>
        ) : (
          <Button icon={<Pause className="size-4" />} onClick={props.onPause} disabled={!transfer}>
            暂停
          </Button>
        )}
        <Button variant="subtle" icon={<X className="size-4" />} onClick={props.onCancel}>
          {paused ? "返回" : "取消"}
        </Button>
      </div>
    </Centered>
  );
}

function ErrorPanel({ message, onRetry, onCustomize }: { message: string; onRetry: () => void; onCustomize?: () => void }) {
  return (
    <div className="mx-auto flex max-w-lg flex-col items-center pt-12 text-center">
      <div className="mb-3 flex size-10 items-center justify-center rounded-full bg-red-500/15 text-red-600 dark:text-red-400">
        <AlertTriangle className="size-5" />
      </div>
      <h2 className="mb-2 text-base font-semibold">部署未完成</h2>
      <p className="selectable mb-6 text-sm break-all whitespace-pre-wrap opacity-80">{message}</p>
      <div className="flex gap-2">
        <Button variant="primary" icon={<RotateCcw className="size-4" />} onClick={onRetry}>
          重试
        </Button>
        {onCustomize && (
          <Button icon={<Settings2 className="size-4" />} onClick={onCustomize}>
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
  // First deployment: on by default. Re-deploy: show the current setting.
  const [autostart, setAutostart] = useState<boolean | null>(firstDeploy ? true : null);
  useEffect(() => {
    if (!firstDeploy) api.autostart().then(setAutostart);
  }, [firstDeploy]);
  return (
    <div className="mx-auto flex max-w-md flex-col items-center pt-10 text-center">
      <div className="mb-3 flex size-10 items-center justify-center rounded-full bg-green-600/15 text-green-700 dark:text-green-400">
        <Check className="size-5" />
      </div>
      <h2 className="mb-1 text-base font-semibold">部署完成</h2>
      <p className="mb-5 text-sm opacity-60">
        {model?.name} · {backendName(outcome.backend)} · 加载 {(outcome.loadMs / 1000).toFixed(1)} 秒
      </p>
      <Card className="w-full p-4 text-left text-sm">
        <div className="text-xs opacity-60">自检</div>
        <div className="mt-1">{outcome.sampleSource}</div>
        <div className="selectable mt-1 font-medium">{outcome.sample}</div>
        <div className="mt-1 text-xs opacity-60">耗时 {outcome.translateMs} ms</div>
      </Card>
      {outcome.fellBack && (
        <p className="mt-3 flex items-start gap-2 rounded-md bg-amber-500/10 p-3 text-left text-xs text-amber-800 dark:text-amber-300">
          <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          <span>
            GPU 启动失败，已改用 CPU 并保存该设置。原因：{outcome.fallbackReason}
            {model && model.size > 2e9 && "。该模型在 CPU 上较慢，建议重新部署并选择“轻量 · Index-Translate 2B”。"}
          </span>
        </p>
      )}
      <label className="mt-5 flex cursor-pointer items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="size-4 accent-[var(--accent)]"
          checked={autostart ?? false}
          disabled={autostart === null}
          onChange={(e) => setAutostart(e.target.checked)}
        />
        开机自动启动（驻留在系统托盘）
      </label>
      <Button variant="primary" className="mt-4" onClick={() => onFinish(autostart)}>
        开始使用
      </Button>
    </div>
  );
}
