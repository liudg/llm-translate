// Settings (DESIGN 5.8): general (theme, start with Windows), shortcuts
// (recorded), selection translation, models (management, backend / threads /
// idle release), about.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, Download, FolderOpen, Plus, RefreshCw, X } from "lucide-react";
import {
  api,
  errorText,
  type AppInfo,
  type HardwareInfo,
  type HotkeyInfo,
  type SelectionSettings,
  type Theme,
  type WindowProcess,
} from "../../lib/api";
import { bytes, mbToGb } from "../../lib/format";
import { Badge, Button, Progress, Segmented, Select, Spinner, Toggle } from "../../components/ui";
import { useSettings } from "../../stores/settings";
import { useUpdater } from "../../stores/updater";
import { HotkeyRecorder } from "./HotkeyRecorder";
import { ModelList } from "./ModelList";
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
  const [switching, setSwitching] = useState(false);
  const onBusy = useCallback((b: boolean) => setSwitching(b), []);
  const [key, setKey] = useState<BackendKey | null>(null);
  const [threads, setThreads] = useState<number | null>(null);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.hardware().then(setHw).catch((e) => setError(errorText(e)));
  }, []);

  if (!settings) return null;
  const savedKey = backendKey(settings.backend, settings.device);
  const curKey = key ?? savedKey;
  const curThreads = threads ?? settings.threads;
  const dirty = curKey !== savedKey || (curKey === "cpu" && curThreads !== settings.threads);
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
      <ModelList onBusy={onBusy} />

      <Row
        title="模型目录"
        desc={
          <span className="selectable block truncate" title={settings.modelsDir}>
            {settings.modelsDir}
          </span>
        }
      >
        <Button icon={<FolderOpen className="size-4" />} onClick={() => api.openModelsDir()}>
          打开目录
        </Button>
        <Button icon={<RefreshCw className="size-4" />} disabled={switching} onClick={() => api.openWizard()}>
          重新部署
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
              <Button variant="primary" disabled={applying || switching} onClick={apply}>
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
    </Group>
  );
}

function SelectionGroup() {
  const settings = useSettings((s) => s.settings);
  const [manual, setManual] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (!settings) return null;
  const sel = settings.selection;

  const save = (patch: Partial<SelectionSettings>) => {
    setError(null);
    api.setSelection({ ...sel, ...patch }).catch((e) => setError(errorText(e)));
  };
  const has = (exe: string) => sel.blacklist.some((b) => b.toLowerCase() === exe.toLowerCase());
  const add = (exe: string) => {
    const name = exe.trim();
    if (!name || has(name)) return;
    save({ blacklist: [...sel.blacklist, name] });
  };
  const addManual = () => {
    let name = manual.trim();
    if (!name) return;
    if (!/\.exe$/i.test(name)) name += ".exe";
    add(name);
    setManual("");
  };
  const off = !sel.enabled;
  const triggers: { key: "drag" | "doubleClick" | "shiftClick"; title: string; desc: string }[] = [
    { key: "drag", title: "拖动选择", desc: "按住左键拖过文字后松开" },
    { key: "doubleClick", title: "双击选词", desc: "双击（三击选段落同样有效）" },
    { key: "shiftClick", title: "Shift + 单击", desc: "按住 Shift 单击扩展选区" },
  ];

  return (
    <Group title="划词翻译">
      <Row title="划词翻译" desc="选中文字后在光标旁显示图标，点击图标才取词并翻译；不会发送 Ctrl+C，终端里也安全">
        <Toggle checked={sel.enabled} onChange={(v) => save({ enabled: v })} />
      </Row>
      {triggers.map((t) => (
        <Row key={t.key} title={t.title} desc={t.desc} muted={off}>
          <Toggle checked={sel[t.key]} disabled={off} onChange={(v) => save({ [t.key]: v })} />
        </Row>
      ))}
      <div
        className={`flex flex-col gap-3 rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)] px-4 py-3 ${off ? "opacity-55" : ""}`}
      >
        <div>
          <div className="text-sm">黑名单</div>
          <div className="mt-0.5 text-xs opacity-65">在这些程序里选中文字时不显示图标（按进程名匹配）</div>
        </div>
        {sel.blacklist.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {sel.blacklist.map((b) => (
              <span
                key={b}
                className="inline-flex h-7 items-center gap-1 rounded-md border border-[var(--stroke)] bg-[var(--surface)] pr-1 pl-2.5 text-xs"
              >
                {b}
                <button
                  title="移除"
                  onClick={() => save({ blacklist: sel.blacklist.filter((x) => x !== b) })}
                  className="inline-flex size-5 items-center justify-center rounded hover:bg-black/5 dark:hover:bg-white/10"
                >
                  <X className="size-3" />
                </button>
              </span>
            ))}
          </div>
        ) : (
          <div className="text-xs opacity-50">暂无</div>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <ProcessPicker disabled={off} isAdded={has} onPick={add} onError={setError} />
          <input
            value={manual}
            disabled={off}
            onChange={(e) => setManual(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addManual()}
            placeholder="或输入进程名，如 notepad.exe"
            spellCheck={false}
            className="h-8 min-w-0 flex-1 rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)] px-2 text-sm outline-none focus:border-[var(--accent)]"
          />
          <Button icon={<Plus className="size-4" />} disabled={off || !manual.trim()} onClick={addManual}>
            添加
          </Button>
        </div>
        {error && <div className="text-xs text-red-600 dark:text-red-400">{error}</div>}
      </div>
    </Group>
  );
}

/** Button that lists processes with a visible window, fetched when opened. */
function ProcessPicker({
  disabled,
  isAdded,
  onPick,
  onError,
}: {
  disabled: boolean;
  isAdded: (exe: string) => boolean;
  onPick: (exe: string) => void;
  onError: (e: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [procs, setProcs] = useState<WindowProcess[] | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  const toggle = () => {
    if (open) return setOpen(false);
    setProcs(null);
    setOpen(true);
    api
      .windowProcesses()
      .then(setProcs)
      .catch((e) => {
        setOpen(false);
        onError(errorText(e));
      });
  };

  return (
    <div ref={ref} className="relative">
      <Button disabled={disabled} onClick={toggle} className="w-56 justify-between">
        <span>从正在运行的程序中选择</span>
        <ChevronDown className="size-4 opacity-70" />
      </Button>
      {open && (
        <div className="absolute top-full left-0 z-20 mt-1 max-h-72 w-96 overflow-y-auto rounded-lg border border-[var(--stroke)] bg-white p-1 shadow-lg dark:bg-[#2b2b2b]">
          {procs === null && (
            <div className="flex items-center gap-2 px-3 py-2 text-xs opacity-60">
              <Spinner className="size-3.5" />
              正在读取…
            </div>
          )}
          {procs?.length === 0 && <div className="px-3 py-2 text-xs opacity-60">没有找到有窗口的程序</div>}
          {procs?.map((p) => {
            const added = isAdded(p.exe);
            return (
              <button
                key={p.exe}
                disabled={added}
                onClick={() => {
                  onPick(p.exe);
                  setOpen(false);
                }}
                className="flex w-full flex-col items-start rounded-md px-3 py-1.5 text-left hover:bg-black/5 disabled:opacity-40 dark:hover:bg-white/10"
              >
                <span className="text-sm">
                  {p.exe}
                  {added && <span className="ml-2 text-xs opacity-70">已添加</span>}
                </span>
                <span className="w-full truncate text-xs opacity-60">{p.title}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function AboutVersionRow({ info }: { info: AppInfo | null }) {
  const snapshot = useUpdater((s) => s.snapshot);
  const check = useUpdater((s) => s.check);
  const install = useUpdater((s) => s.install);

  const avail = snapshot.available;
  const checking = snapshot.phase === "checking";
  const downloading = snapshot.phase === "downloading";
  const installing = snapshot.phase === "installing";
  const prog = snapshot.progress;
  const ratio = prog?.total ? prog.downloaded / prog.total : 0;

  let statusHint: ReactNode = null;
  if (snapshot.phase === "upToDate" && snapshot.manual) {
    statusHint = <span className="text-green-700 dark:text-green-400">已是最新版本</span>;
  } else if (snapshot.phase === "error" && snapshot.error) {
    statusHint = <span className="text-red-600 dark:text-red-400">{snapshot.error}</span>;
  } else if (downloading) {
    statusHint = (
      <span>
        正在下载 v{avail?.version}…{" "}
        {prog?.total
          ? `${Math.min(100, Math.round(ratio * 100))}%（${bytes(prog.downloaded)} / ${bytes(prog.total)}）`
          : prog
            ? bytes(prog.downloaded)
            : ""}
      </span>
    );
  } else if (installing) {
    statusHint = <span>正在安装 v{avail?.version}，完成后将自动重启…</span>;
  }

  return (
    <div className="flex flex-col gap-3 rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)] px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <span>LLM Translate {info?.version ?? ""}</span>
            {avail && <Badge tone="accent">新版本 v{avail.version}</Badge>}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2 text-xs opacity-65">
            {info && <span>Windows build {info.osBuild} · 窗口效果 {info.effect}</span>}
            {statusHint && (
              <>
                {info && <span>·</span>}
                <span className="opacity-100">{statusHint}</span>
              </>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {avail ? (
            <>
              {!downloading && !installing && (
                <Button variant="subtle" icon={<RefreshCw className="size-4" />} onClick={check}>
                  重新检查
                </Button>
              )}
              <Button
                variant="primary"
                icon={downloading || installing ? <Spinner /> : <Download className="size-4" />}
                disabled={downloading || installing}
                onClick={install}
              >
                {downloading
                  ? "下载中…"
                  : installing
                    ? "安装中…"
                    : snapshot.phase === "error"
                      ? "重试更新"
                      : `更新到 v${avail.version}`}
              </Button>
            </>
          ) : (
            <Button icon={checking ? <Spinner /> : <RefreshCw className="size-4" />} disabled={checking} onClick={check}>
              {checking ? "正在检查…" : "检查更新"}
            </Button>
          )}
        </div>
      </div>

      {downloading && <Progress value={ratio} indeterminate={!prog?.total} />}
      {installing && <Progress value={1} indeterminate />}

      {avail && (avail.notes || avail.pubDate) && (
        <div className="rounded-md border border-[var(--stroke)] bg-[var(--surface)] px-3 py-2 text-xs">
          <div className="mb-1 flex items-center justify-between opacity-70">
            <span className="font-medium">v{avail.version} 更新内容</span>
            {avail.pubDate && <span>{avail.pubDate}</span>}
          </div>
          {avail.notes && (
            <div className="selectable max-h-40 overflow-y-auto leading-relaxed whitespace-pre-wrap opacity-85">
              {avail.notes}
            </div>
          )}
        </div>
      )}
    </div>
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

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  // Re-read on every visit: it can also be turned off in Task Manager.
  useEffect(() => {
    if (visible) api.autostart().then(setAutostart);
  }, [visible]);
  const toggleAutostart = async (on: boolean) => {
    setAutostartError(null);
    try {
      await api.setAutostart(on);
    } catch (e) {
      setAutostartError(errorText(e));
    }
    setAutostart(await api.autostart());
  };

  const hk = (id: HotkeyInfo["id"]) => hotkeys.find((h) => h.id === id);

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
          <Row
            title="开机自启"
            desc={autostartError ?? "登录 Windows 后自动启动并驻留在系统托盘，不打开主窗口"}
          >
            <Toggle checked={autostart ?? false} disabled={autostart === null} onChange={toggleAutostart} />
          </Row>
        </Group>

        <Group title="快捷键">
          <Row title="输入翻译（迷你窗）" desc={hk("mini")?.error ?? "在任意位置按下即可呼出迷你翻译窗"}>
            <HotkeyRecorder id="mini" info={hk("mini")} visible={visible} />
          </Row>
          <Row title="截图翻译" desc={hk("shot")?.error ?? "框选屏幕区域，译文覆盖在原文位置"}>
            <HotkeyRecorder id="shot" info={hk("shot")} visible={visible} />
          </Row>
          <Row title="划词翻译开关" desc={hk("selection")?.error ?? "打开 / 关闭划词翻译；关闭时托盘图标变灰"}>
            <HotkeyRecorder id="selection" info={hk("selection")} visible={visible} />
          </Row>
        </Group>

        <SelectionGroup />

        {mounted && <ModelGroup />}

        <Group title="关于">
          <AboutVersionRow info={info} />
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
