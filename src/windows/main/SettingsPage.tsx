// Settings (DESIGN 5.8). Live in M2: theme, model backend / threads / idle
// release, about. Startup, shortcut recording, selection and model management
// are placeholders until M3/M5.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { FolderOpen, RefreshCw } from "lucide-react";
import { api, errorText, type AppInfo, type HardwareInfo, type HotkeyInfo, type ModelView, type Theme } from "../../lib/api";
import { mbToGb } from "../../lib/format";
import { Badge, Button, Segmented, Select, Spinner, Toggle } from "../../components/ui";
import { StatusChip } from "../../components/status";
import { useSettings } from "../../stores/settings";

const LATER = "将在后续版本提供";
const IDLE_CHOICES = [0, 5, 10, 30, 60];

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1">
      <h2 className="mb-1 text-sm font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function Row({
  title,
  desc,
  children,
  muted = false,
}: {
  title: ReactNode;
  desc?: ReactNode;
  children?: ReactNode;
  muted?: boolean;
}) {
  return (
    <div className="flex min-h-[60px] flex-wrap items-center gap-x-4 gap-y-2 rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)] px-4 py-3">
      <div className={`min-w-0 flex-1 ${muted ? "opacity-55" : ""}`}>
        <div className="text-sm">{title}</div>
        {desc && <div className="mt-0.5 text-xs opacity-65">{desc}</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

/** Backend picker value: `cpu`, `vulkan:` (llama.cpp picks) or `vulkan:<device id>`. */
type BackendKey = string;

function backendKey(backend: "cpu" | "vulkan", device: string | null): BackendKey {
  return backend === "cpu" ? "cpu" : `vulkan:${device ?? ""}`;
}

function ModelGroup() {
  const settings = useSettings((s) => s.settings);
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [models, setModels] = useState<ModelView[]>([]);
  const [key, setKey] = useState<BackendKey | null>(null);
  const [threads, setThreads] = useState<number | null>(null);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.hardware().then(setHw).catch((e) => setError(errorText(e)));
  }, []);
  useEffect(() => {
    if (settings) api.models(settings.modelsDir).then(setModels);
  }, [settings?.modelsDir, settings?.modelId]);

  if (!settings) return null;
  const savedKey = backendKey(settings.backend, settings.device);
  const curKey = key ?? savedKey;
  const curThreads = threads ?? settings.threads;
  const dirty = curKey !== savedKey || (curKey === "cpu" && curThreads !== settings.threads);
  const model = models.find((m) => m.id === settings.modelId);
  const maxThreads = hw?.cpu.logicalCores ?? Math.max(settings.threads, 16);

  const apply = async () => {
    setApplying(true);
    setError(null);
    try {
      const [backend, device] = curKey === "cpu" ? (["cpu", null] as const) : (["vulkan", curKey.slice(7) || null] as const);
      await api.setInference(backend, device, curThreads);
      setKey(null);
      setThreads(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setApplying(false);
    }
  };

  return (
    <Group title="模型">
      <Row
        title={model?.name ?? settings.modelId ?? "未部署"}
        desc={
          <span className="flex flex-col gap-1">
            <StatusChip />
            <span className="selectable truncate" title={settings.modelsDir}>
              {settings.modelsDir}
            </span>
          </span>
        }
      >
        <Button icon={<FolderOpen className="size-4" />} onClick={() => api.openModelsDir()}>
          打开目录
        </Button>
        <Button icon={<RefreshCw className="size-4" />} onClick={() => api.openWizard()}>
          更换模型 / 重新部署
        </Button>
      </Row>

      <Row
        title="推理后端"
        desc={
          hw === null
            ? "正在检测显卡…"
            : hw.gpus.length === 0
              ? `未检测到可用的 Vulkan 显卡${hw.listDevicesError ? `（${hw.listDevicesError}）` : ""}`
              : "切换后会重新加载模型；GPU 多次失败会自动退回 CPU"
        }
      >
        {hw === null && <Spinner className="opacity-60" />}
        <Select value={curKey} onChange={(e) => setKey(e.target.value)} className="max-w-[420px]">
          <option value="cpu">CPU</option>
          {savedKey === "vulkan:" && <option value="vulkan:">GPU（自动选择）</option>}
          {hw?.gpus.map((g) => (
            <option key={g.id} value={`vulkan:${g.id}`}>
              GPU · {g.name}（{g.integrated ? "核显，" : ""}显存 {mbToGb(g.totalMb)}）
            </option>
          ))}
          {/* Saved device not in the list (driver change, detection still running). */}
          {savedKey.startsWith("vulkan:") &&
            savedKey !== "vulkan:" &&
            !hw?.gpus.some((g) => `vulkan:${g.id}` === savedKey) && (
              <option value={savedKey}>GPU · {settings.device}</option>
            )}
        </Select>
      </Row>

      {curKey === "cpu" && (
        <Row
          title="CPU 线程数"
          desc={hw ? `本机 ${hw.cpu.physicalCores} 个物理核心、${hw.cpu.logicalCores} 个逻辑核心；通常设为物理核心数最快` : undefined}
        >
          <Select value={curThreads} onChange={(e) => setThreads(Number(e.target.value))}>
            {Array.from({ length: maxThreads }, (_, i) => i + 1).map((n) => (
              <option key={n} value={n}>
                {n}
                {hw && n === hw.cpu.physicalCores ? "（推荐）" : ""}
              </option>
            ))}
          </Select>
        </Row>
      )}

      {(dirty || error) && (
        <div className="flex items-center justify-end gap-2 py-1">
          {error && <span className="mr-auto text-xs text-red-600 dark:text-red-400">{error}</span>}
          {dirty && (
            <>
              <Button
                onClick={() => {
                  setKey(null);
                  setThreads(null);
                }}
              >
                取消
              </Button>
              <Button variant="primary" disabled={applying} onClick={apply}>
                {applying ? <Spinner /> : null}
                应用并重新加载模型
              </Button>
            </>
          )}
        </div>
      )}

      <Row title="闲置释放模型" desc="闲置一段时间后关闭推理进程以释放内存 / 显存，下次翻译时自动重新加载（需几秒）">
        <Select value={settings.idleReleaseMinutes} onChange={(e) => api.setIdleRelease(Number(e.target.value))}>
          {IDLE_CHOICES.map((m) => (
            <option key={m} value={m}>
              {m === 0 ? "从不" : `${m} 分钟`}
            </option>
          ))}
        </Select>
      </Row>

      <Row title="模型管理" desc={`下载、切换、删除多个模型${LATER}；目前可通过"更换模型 / 重新部署"更换`} muted />
    </Group>
  );
}

export default function SettingsPage({ hotkeys, visible }: { hotkeys: HotkeyInfo[]; visible: boolean }) {
  const settings = useSettings((s) => s.settings);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    api.appInfo().then(setInfo);
  }, []);
  // Hardware detection runs llama-server; only start it once the page is opened.
  useEffect(() => {
    if (visible) setMounted(true);
  }, [visible]);

  const mini = useMemo(() => hotkeys.find((h) => h.id === "mini"), [hotkeys]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 p-5 pb-10">
        <h1 className="text-lg font-semibold">设置</h1>

        <Group title="常规">
          <Row title="主题" desc="浅色 / 深色，或跟随 Windows 设置">
            <Segmented<Theme>
              value={settings?.theme ?? "system"}
              onChange={(t) => api.setTheme(t)}
              options={[
                { value: "system", label: "跟随系统" },
                { value: "light", label: "浅色" },
                { value: "dark", label: "深色" },
              ]}
            />
          </Row>
          <Row title="开机自启" desc={LATER} muted>
            <Toggle checked={false} disabled />
          </Row>
        </Group>

        <Group title="快捷键">
          <Row title="输入翻译（迷你窗）" desc={mini?.error ?? "在任意位置按下即可呼出迷你翻译窗"}>
            <kbd className="rounded border border-[var(--stroke)] bg-[var(--surface)] px-2 py-0.5 text-xs">
              {mini?.keys ?? "Alt+Q"}
            </kbd>
            {mini && (mini.registered ? <Badge tone="good">已生效</Badge> : <Badge tone="warn">未生效</Badge>)}
          </Row>
          <Row title="截图翻译" desc={LATER} muted>
            <kbd className="rounded border border-[var(--stroke)] px-2 py-0.5 text-xs opacity-60">Alt+W</kbd>
          </Row>
          <Row title="划词翻译开关" desc={LATER} muted>
            <span className="text-xs opacity-60">未绑定</span>
          </Row>
          <Row title="自定义快捷键" desc={`录制快捷键与冲突检测${LATER}`} muted />
        </Group>

        <Group title="划词翻译">
          <Row title="划词翻译" desc={`选中文字后点击图标翻译；开关、黑名单与触发方式${LATER}`} muted>
            <Toggle checked={false} disabled />
          </Row>
        </Group>

        {mounted && <ModelGroup />}

        <Group title="关于">
          <Row
            title={`LLM Translate ${info?.version ?? ""}`}
            desc={info ? `Windows build ${info.osBuild} · 窗口效果 ${info.effect}` : undefined}
          />
          <Row
            title="设置文件"
            desc={
              <span className="selectable" title={info?.configDir}>
                {info ? `${info.configDir}\\settings.json` : ""}
              </span>
            }
          />
          <Row
            title="日志"
            desc={
              <span className="selectable" title={info?.logsDir}>
                {info?.logsDir}
              </span>
            }
          >
            <Button icon={<FolderOpen className="size-4" />} onClick={() => api.openLogsDir()}>
              打开日志目录
            </Button>
          </Row>
        </Group>
      </div>
    </div>
  );
}
