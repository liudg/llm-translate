// Settings (DESIGN 5.8): categorized editorial settings with visual theme cards,
// custom dropdowns, searchable process picker, model cards and tactile hotkey recorders.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
  Copy,
  Cpu,
  Download,
  FolderOpen,
  Info,
  Keyboard,
  MousePointerClick,
  Palette,
  Plus,
  RefreshCw,
  Search,
  Volume2,
  X,
} from "lucide-react";
import {
  api,
  errorText,
  type AppInfo,
  type FilterMode,
  type HardwareInfo,
  type HotkeyInfo,
  type SelectionPatch,
  type Theme,
  type TtsPatch,
  type VoiceCatalog,
  type WindowProcess,
} from "../../lib/api";
import { mbToGb } from "../../lib/format";
import { useCopy } from "../../lib/useCopy";
import { isImeComposing } from "../../lib/useTranslator";
import { useTts } from "../../lib/useTts";
import {
  Badge,
  BrandSeal,
  Button,
  Card,
  CustomSelect,
  Progress,
  Segmented,
  Spinner,
  Toggle,
  type SelectOption,
} from "../../components/ui";
import { SpeakButton } from "../../components/tts";
import { StatusChip } from "../../components/status";
import { useSettings } from "../../stores/settings";
import { downloadProgress, useUpdater } from "../../stores/updater";
import { HotkeyRecorder } from "./HotkeyRecorder";
import { ModelList } from "./ModelList";

export type SettingsSection = "general" | "hotkeys" | "selection" | "tts" | "models" | "about";

const IDLE_CHOICES = [0, 5, 10, 30, 60];

const NAV_ITEMS: { id: SettingsSection; label: string; icon: typeof Palette }[] = [
  { id: "general", label: "常规与外观", icon: Palette },
  { id: "hotkeys", label: "全局快捷键", icon: Keyboard },
  { id: "selection", label: "划词翻译", icon: MousePointerClick },
  { id: "tts", label: "语音朗读", icon: Volume2 },
  { id: "models", label: "模型与推理", icon: Cpu },
  { id: "about", label: "关于与更新", icon: Info },
];

function SectionHeader({ title }: { title: string }) {
  return (
    <div className="mb-4">
      <h2 className="text-large font-semibold tracking-tight text-[var(--ink)]">{title}</h2>
    </div>
  );
}

function SettingRow({
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
    <div className="flex min-h-[64px] flex-wrap items-center gap-x-4 gap-y-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-3 shadow-[var(--shadow-card)]">
      <div className={`min-w-0 flex-1 ${muted ? "opacity-50" : ""}`}>
        <div className="text-body font-medium text-[var(--ink)]">{title}</div>
        {desc && <div className="mt-0.5 text-caption leading-relaxed text-[var(--ink-secondary)]">{desc}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

/** Visual miniature theme preview cards (System / Light Paper / Dark Obsidian). */
function ThemePicker({ value, onChange }: { value: Theme; onChange: (t: Theme) => void }) {
  const choices: {
    id: Theme;
    label: string;
    preview: ReactNode;
  }[] = [
    {
      id: "system",
      label: "跟随系统",
      preview: (
        <div className="relative flex h-12 w-full overflow-hidden rounded-lg border border-[var(--border)]">
          <div className="flex flex-1 flex-col gap-1 bg-[#f4f1ea] p-2">
            <div className="h-1.5 w-6 rounded-full bg-[#b8431f]" />
            <div className="h-1.5 w-10 rounded-full bg-[#1b1917]/20" />
            <div className="h-1.5 w-7 rounded-full bg-[#1b1917]/12" />
          </div>
          <div className="flex flex-1 flex-col gap-1 bg-[#121110] p-2">
            <div className="h-1.5 w-6 rounded-full bg-[#e06d48]" />
            <div className="h-1.5 w-10 rounded-full bg-[#eeebe3]/25" />
            <div className="h-1.5 w-7 rounded-full bg-[#eeebe3]/14" />
          </div>
        </div>
      ),
    },
    {
      id: "light",
      label: "宣纸浅色",
      preview: (
        <div className="flex h-12 w-full flex-col gap-1.5 rounded-lg border border-[#e3ded3] bg-[#f4f1ea] p-2">
          <div className="flex items-center gap-1.5">
            <div className="size-2.5 rounded-md bg-[#b8431f]" />
            <div className="h-1.5 w-12 rounded-full bg-[#1b1917]/25" />
          </div>
          <div className="flex flex-1 gap-1.5">
            <div className="flex-1 rounded-md bg-[#fcfbf8] border border-[#e3ded3]" />
            <div className="flex-1 rounded-md bg-[#f8f5ee] border border-[#e3ded3]" />
          </div>
        </div>
      ),
    },
    {
      id: "dark",
      label: "墨砚深色",
      preview: (
        <div className="flex h-12 w-full flex-col gap-1.5 rounded-lg border border-[#2c2a26] bg-[#121110] p-2">
          <div className="flex items-center gap-1.5">
            <div className="size-2.5 rounded-md bg-[#e06d48]" />
            <div className="h-1.5 w-12 rounded-full bg-[#eeebe3]/30" />
          </div>
          <div className="flex flex-1 gap-1.5">
            <div className="flex-1 rounded-md bg-[#1b1a18] border border-[#2c2a26]" />
            <div className="flex-1 rounded-md bg-[#161513] border border-[#2c2a26]" />
          </div>
        </div>
      ),
    },
  ];

  return (
    <Card className="p-4">
      <div className="mb-3">
        <div className="text-body font-medium text-[var(--ink)]">外观主题</div>
      </div>
      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-3">
        {choices.map((c) => {
          const active = value === c.id;
          return (
            <button
              key={c.id}
              type="button"
              onClick={() => onChange(c.id)}
              className={`flex cursor-pointer flex-col gap-2 rounded-xl border p-2.5 text-left transition-all ${
                active
                  ? "border-[var(--accent)] bg-[var(--accent-soft)] ring-1 ring-[var(--accent)]/30"
                  : "border-[var(--border)] bg-[var(--bg-surface-alt)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)]"
              }`}
            >
              {c.preview}
              <div className="flex items-center justify-between px-0.5">
                <span className={`text-caption font-semibold ${active ? "text-[var(--accent-text)]" : "text-[var(--ink)]"}`}>
                  {c.label}
                </span>
                {active && (
                  <span className="inline-flex size-4 items-center justify-center rounded-full bg-[var(--accent)] text-[var(--ink-inverse)]">
                    <Check className="size-2.5" />
                  </span>
                )}
              </div>
            </button>
          );
        })}
      </div>
    </Card>
  );
}

/** Backend picker value: `cpu`, `vulkan:` (llama.cpp picks) or `vulkan:<device id>`. */
type BackendKey = string;

function backendKey(backend: "cpu" | "vulkan", device: string | null): BackendKey {
  return backend === "cpu" ? "cpu" : `vulkan:${device ?? ""}`;
}

function ModelSection() {
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

  const savedKey = settings ? backendKey(settings.backend, settings.device) : "cpu";
  const curKey = key ?? savedKey;
  const curThreads = threads ?? settings?.threads ?? 4;
  const maxThreads = hw?.cpu.logicalCores ?? Math.max(settings?.threads ?? 4, 16);

  const backendOptions = useMemo<SelectOption<BackendKey>[]>(() => {
    const list: SelectOption<BackendKey>[] = [
      {
        value: "cpu",
        label: "CPU 推理",
        desc: hw ? `${hw.cpu.name}（${hw.cpu.physicalCores} 核）` : "通用模式",
      },
    ];
    if (savedKey === "vulkan:") {
      list.push({ value: "vulkan:", label: "GPU · 自动选择", desc: "自动选择可用显卡" });
    }
    if (hw) {
      for (const g of hw.gpus) {
        list.push({
          value: `vulkan:${g.id}`,
          label: `GPU · ${g.name}`,
          desc: `显存 ${mbToGb(g.totalMb)}${g.freeMb > 0 ? `（可用 ${mbToGb(g.freeMb)}）` : ""}`,
          badge: g.integrated ? "核显" : "独显",
        });
      }
    }
    if (
      savedKey.startsWith("vulkan:") &&
      savedKey !== "vulkan:" &&
      !hw?.gpus.some((g) => `vulkan:${g.id}` === savedKey)
    ) {
      list.push({ value: savedKey, label: `GPU · ${settings?.device ?? ""}`, badge: "已保存" });
    }
    return list;
  }, [hw, savedKey, settings?.device]);

  const threadOptions = useMemo<SelectOption<number>[]>(
    () =>
      Array.from({ length: maxThreads }, (_, i) => i + 1).map((n) => ({
        value: n,
        label: `${n} 线程`,
        badge: hw && n === hw.cpu.physicalCores ? "推荐" : undefined,
      })),
    [hw, maxThreads],
  );

  const idleOptions = useMemo<SelectOption<number>[]>(
    () =>
      IDLE_CHOICES.map((m) => ({
        value: m,
        label: m === 0 ? "从不释放（常驻内存）" : `闲置 ${m} 分钟后释放`,
      })),
    [],
  );

  if (!settings) return null;
  const dirty = curKey !== savedKey || (curKey === "cpu" && curThreads !== settings.threads);

  const apply = async () => {
    setApplying(true);
    setError(null);
    try {
      const [backend, device] =
        curKey === "cpu" ? (["cpu", null] as const) : (["vulkan", curKey.slice(7) || null] as const);
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
    <div className="flex flex-col gap-4">
      <SectionHeader title="模型与推理" />

      <ModelList onBusy={onBusy} />

      <div className="mt-1 flex flex-col gap-2.5">
        <SettingRow
          title={
            <span className="flex flex-wrap items-center gap-2.5">
              <span>推理后端</span>
              <StatusChip />
            </span>
          }
          desc={
            hw === null
              ? "正在检测可用显卡…"
              : hw.gpus.length === 0
                ? `未检测到可用显卡${hw.listDevicesError ? `（${hw.listDevicesError}）` : ""}`
                : "切换后会重新加载模型"
          }
        >
          {hw === null && <Spinner className="size-3.5 text-[var(--ink-muted)]" />}
          <CustomSelect
            value={curKey}
            options={backendOptions}
            onChange={setKey}
            className="w-[260px]"
            menuClassName="w-[320px]"
          />
        </SettingRow>

        {curKey === "cpu" && (
          <SettingRow
            title="CPU 线程数"
            desc={
              hw
                ? `建议设为物理核心数（当前 ${hw.cpu.physicalCores} 核）`
                : "设置 CPU 推理使用的线程数"
            }
          >
            <CustomSelect
              value={curThreads}
              options={threadOptions}
              onChange={setThreads}
              className="w-[150px]"
            />
          </SettingRow>
        )}

        {(dirty || error) && (
          <div className="animate-panel-in flex flex-wrap items-center gap-2 rounded-xl border border-[var(--accent)]/30 bg-[var(--accent-soft)] px-4 py-2.5">
            {error ? (
              <span className="mr-auto text-caption text-[var(--danger)]">{error}</span>
            ) : (
              <span className="mr-auto text-caption font-medium text-[var(--accent-text)]">
                配置已更改，应用后生效
              </span>
            )}
            {dirty && (
              <>
                <Button
                  size="sm"
                  variant="subtle"
                  onClick={() => {
                    setKey(null);
                    setThreads(null);
                  }}
                >
                  放弃更改
                </Button>
                <Button size="sm" variant="primary" disabled={applying || switching} onClick={apply}>
                  {applying && <Spinner className="size-3.5" />}
                  应用并重新加载
                </Button>
              </>
            )}
          </div>
        )}

        <SettingRow
          title="闲置自动释放"
          desc="空闲一段时间后释放显存和内存，下次翻译时自动恢复"
        >
          <CustomSelect
            value={settings.idleReleaseMinutes}
            options={idleOptions}
            onChange={(m) => api.setIdleRelease(m)}
            className="w-[200px]"
          />
        </SettingRow>

        <SettingRow
          title="模型存储目录"
          desc={
            <span className="selectable block truncate font-mono text-caption" title={settings.modelsDir}>
              {settings.modelsDir}
            </span>
          }
        >
          <Button size="sm" icon={<FolderOpen className="size-3.5" />} onClick={() => api.openModelsDir()}>
            打开目录
          </Button>
          <Button
            size="sm"
            variant="subtle"
            icon={<RefreshCw className="size-3.5" />}
            disabled={switching}
            onClick={() => api.openWizard()}
          >
            重新部署
          </Button>
        </SettingRow>
      </div>
    </div>
  );
}

const FILTER_MODES: { value: FilterMode; label: string }[] = [
  { value: "blacklist", label: "黑名单模式" },
  { value: "whitelist", label: "白名单模式" },
];

const TTS_RATES: { value: string; label: string; rate: number }[] = [
  { value: "0.75", label: "0.75×", rate: 0.75 },
  { value: "0.9", label: "0.9×", rate: 0.9 },
  { value: "1", label: "1.0×", rate: 1.0 },
  { value: "1.25", label: "1.25×", rate: 1.25 },
  { value: "1.5", label: "1.5×", rate: 1.5 },
];

const PREVIEW_EN = "Hello! Welcome to LLM Translate. Clear pronunciation makes reading effortless.";
const PREVIEW_ZH = "你好，欢迎使用 LLM Translate，清晰的发音让阅读更轻松。";

function TtsSection({ active }: { active: boolean }) {
  const settings = useSettings((s) => s.settings);
  const [catalog, setCatalog] = useState<VoiceCatalog | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const tts = useTts();
  const stopPreview = tts.stopIfKeyPrefix;

  useEffect(() => {
    api.ttsVoices().then(setCatalog).catch((e) => setSaveError(errorText(e)));
  }, []);

  useEffect(() => {
    if (!active) stopPreview("preview:");
    return () => stopPreview("preview:");
  }, [active, stopPreview]);

  const enOptions = useMemo<SelectOption<string>[]>(
    () =>
      (catalog?.en ?? []).map((v) => ({
        value: v.id,
        label: v.label,
        desc: v.desc,
        badge: v.badge ?? undefined,
      })),
    [catalog],
  );

  const zhOptions = useMemo<SelectOption<string>[]>(
    () =>
      (catalog?.zh ?? []).map((v) => ({
        value: v.id,
        label: v.label,
        desc: v.desc,
        badge: v.badge ?? undefined,
      })),
    [catalog],
  );

  if (!settings) return null;
  const cfg = settings.tts;

  const patch = (p: TtsPatch) => {
    setSaveError(null);
    api.setTts(p).catch((e) => setSaveError(errorText(e)));
  };

  const closestRate = TTS_RATES.reduce((best, cur) =>
    Math.abs(cur.rate - cfg.rate) < Math.abs(best.rate - cfg.rate) ? cur : best,
  ).value;

  const previewError = tts.error && tts.errorKey?.startsWith("preview:") ? tts.error : null;

  return (
    <div className="flex flex-col gap-4">
      <SectionHeader title="语音朗读" />

      <div className="flex flex-col gap-2.5">
        <SettingRow title="英文音色">
          <CustomSelect
            value={cfg.enVoice}
            options={enOptions}
            disabled={!catalog}
            onChange={(v) => patch({ enVoice: v })}
            className="w-[188px]"
            menuClassName="w-[240px]"
          />
          <SpeakButton
            size="md"
            label="试听"
            phase={tts.phaseOf("preview:en")}
            title="试听当前英文音色"
            onClick={() =>
              tts.toggle("preview:en", PREVIEW_EN, { lang: "en", voice: cfg.enVoice, rate: cfg.rate })
            }
            className="border border-[var(--border)] bg-[var(--bg-surface-alt)]"
          />
        </SettingRow>

        <SettingRow title="中文音色">
          <CustomSelect
            value={cfg.zhVoice}
            options={zhOptions}
            disabled={!catalog}
            onChange={(v) => patch({ zhVoice: v })}
            className="w-[188px]"
            menuClassName="w-[240px]"
          />
          <SpeakButton
            size="md"
            label="试听"
            phase={tts.phaseOf("preview:zh")}
            title="试听当前中文音色"
            onClick={() =>
              tts.toggle("preview:zh", PREVIEW_ZH, { lang: "zh", voice: cfg.zhVoice, rate: cfg.rate })
            }
            className="border border-[var(--border)] bg-[var(--bg-surface-alt)]"
          />
        </SettingRow>

        <SettingRow title="朗读语速">
          <Segmented
            size="sm"
            value={closestRate}
            options={TTS_RATES.map((r) => ({ value: r.value, label: r.label }))}
            onChange={(v) => {
              const found = TTS_RATES.find((r) => r.value === v);
              if (found) patch({ rate: found.rate });
            }}
          />
        </SettingRow>

        <SettingRow title="划词自动朗读" desc="划词翻译时自动朗读原文">
          <Toggle checked={cfg.popupAutoSpeak} onChange={(v) => patch({ popupAutoSpeak: v })} />
        </SettingRow>
      </div>

      {(saveError || previewError) && (
        <div className="rounded-xl border border-[var(--danger)]/20 bg-[var(--danger-soft)] px-4 py-2.5 text-caption text-[var(--danger)]">
          {saveError ?? previewError}
        </div>
      )}
    </div>
  );
}

function SelectionSection() {
  const settings = useSettings((s) => s.settings);
  const [manual, setManual] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (!settings) return null;
  const sel = settings.selection;
  const isWhitelist = sel.mode === "whitelist";
  const listKey = isWhitelist ? "whitelist" : "blacklist";
  const activeList = sel[listKey];

  const patch = (p: SelectionPatch) => {
    setError(null);
    api.setSelection(p).catch((e) => setError(errorText(e)));
  };
  const has = (exe: string) => activeList.some((b) => b.trim().toLowerCase() === exe.trim().toLowerCase());
  const add = (exe: string) => {
    const name = exe.trim();
    if (!name || has(name)) return;
    patch({ add: { mode: sel.mode, exe: name } });
  };
  const remove = (exe: string) => {
    patch({ remove: { mode: sel.mode, exe } });
  };
  const addManual = () => {
    let name = manual.trim();
    if (!name) return;
    if (!/\.exe$/i.test(name)) name += ".exe";
    add(name);
    setManual("");
  };
  const off = !sel.enabled;

  return (
    <div className="flex flex-col gap-4">
      <SectionHeader title="划词翻译" />

      <SettingRow
        title="启用划词翻译"
        desc="拖动选中文字后在光标旁显示翻译浮标"
      >
        <Toggle checked={sel.enabled} onChange={(v) => patch({ enabled: v })} />
      </SettingRow>

      {/* Filter scope manager (blacklist / whitelist) */}
      <Card className={`flex flex-col gap-3.5 p-4 ${off ? "opacity-50" : ""}`}>
        <div className="flex flex-wrap items-start justify-between gap-2.5">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 text-body font-medium text-[var(--ink)]">
              <span>生效范围</span>
              <Badge>{activeList.length}</Badge>
            </div>
            <div className="mt-0.5 text-caption leading-relaxed text-[var(--ink-secondary)]">
              {isWhitelist
                ? "仅在以下应用中触发划词翻译；右键翻译浮标可快速移出"
                : "在所有应用中生效，以下应用除外；右键翻译浮标可快速屏蔽"}
            </div>
          </div>
          <Segmented
            size="sm"
            disabled={off}
            value={sel.mode}
            options={FILTER_MODES}
            onChange={(mode) => patch({ mode })}
          />
        </div>

        {activeList.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {activeList.map((b) => (
              <span
                key={b}
                className="inline-flex h-7 items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-alt)] pr-1 pl-2.5 font-mono text-caption text-[var(--ink)]"
              >
                {b}
                <button
                  type="button"
                  disabled={off}
                  title={isWhitelist ? "移出白名单" : "移出黑名单"}
                  onClick={() => remove(b)}
                  className="inline-flex size-5 cursor-pointer items-center justify-center rounded-md text-[var(--ink-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)] disabled:pointer-events-none"
                >
                  <X className="size-3" />
                </button>
              </span>
            ))}
          </div>
        ) : (
          <div className="rounded-lg border border-dashed border-[var(--border)] px-3 py-2.5 text-caption text-[var(--ink-muted)]">
            {isWhitelist
              ? "尚未添加应用，白名单为空时不会触发划词翻译"
              : "暂无屏蔽应用（所有支持文字选择的应用均会生效）"}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <ProcessPicker disabled={off} isAdded={has} onPick={add} onError={setError} />
          <input
            value={manual}
            disabled={off}
            onChange={(e) => setManual(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addManual()}
            placeholder="或输入程序名，如 notepad.exe"
            spellCheck={false}
            className="h-8 min-w-[180px] flex-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-alt)] px-2.5 text-caption text-[var(--ink)] placeholder:text-[var(--ink-muted)] focus:border-[var(--accent)] focus:bg-[var(--bg-elevated)] focus:ring-2 focus:ring-[var(--accent-ring)]"
          />
          <Button size="sm" icon={<Plus className="size-3.5" />} disabled={off || !manual.trim()} onClick={addManual}>
            添加
          </Button>
        </div>

        {error && <div className="text-caption text-[var(--danger)]">{error}</div>}
      </Card>
    </div>
  );
}

/** Searchable popover listing currently running desktop apps. */
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
  const [dropUp, setDropUp] = useState(false);
  const [procs, setProcs] = useState<WindowProcess[] | null>(null);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isImeComposing(e)) {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        ref.current?.querySelector("button")?.focus();
      }
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc, true);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc, true);
    };
  }, [open]);

  const toggle = () => {
    if (open) return setOpen(false);
    if (ref.current) {
      const rect = ref.current.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      const spaceAbove = rect.top - 46;
      setDropUp(spaceBelow < 300 && spaceAbove > spaceBelow);
    }
    setProcs(null);
    setQuery("");
    setOpen(true);
    window.setTimeout(() => searchRef.current?.focus(), 40);
    api
      .windowProcesses()
      .then(setProcs)
      .catch((e) => {
        setOpen(false);
        onError(errorText(e));
      });
  };

  const filtered = useMemo(() => {
    if (!procs) return null;
    const q = query.trim().toLowerCase();
    if (!q) return procs;
    return procs.filter(
      (p) => p.exe.toLowerCase().includes(q) || p.title.toLowerCase().includes(q),
    );
  }, [procs, query]);

  return (
    <div ref={ref} className="relative">
      <Button size="sm" disabled={disabled} onClick={toggle} className="w-[210px] justify-between">
        <span>从运行中的程序选择</span>
        <ChevronDown className={`size-3.5 text-[var(--ink-muted)] transition-transform ${open ? "rotate-180" : ""}`} />
      </Button>

      {open && (
        <div
          className={`animate-panel-in absolute left-0 z-30 flex w-[340px] flex-col overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-elevated)] shadow-[var(--shadow-popover)] ${
            dropUp ? "bottom-full mb-1.5" : "top-full mt-1.5"
          }`}
        >
          <div className="flex items-center gap-2 border-b border-[var(--border-subtle)] px-3 py-2">
            <Search className="size-3.5 shrink-0 text-[var(--ink-muted)]" />
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索应用或窗口标题…"
              className="min-w-0 flex-1 bg-transparent text-caption text-[var(--ink)] placeholder:text-[var(--ink-muted)]"
            />
          </div>

          <div className="max-h-60 overflow-y-auto p-1">
            {filtered === null && (
              <div className="flex items-center gap-2 px-3 py-3 text-caption text-[var(--ink-muted)]">
                <Spinner className="size-3.5" />
                正在读取活动窗口…
              </div>
            )}
            {filtered?.length === 0 && (
              <div className="px-3 py-3 text-caption text-[var(--ink-muted)]">没有匹配的可见窗口程序</div>
            )}
            {filtered?.map((p) => {
              const added = isAdded(p.exe);
              return (
                <button
                  key={p.exe}
                  type="button"
                  disabled={added}
                  onClick={() => {
                    onPick(p.exe);
                    setOpen(false);
                  }}
                  className="flex w-full cursor-pointer flex-col items-start rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-45"
                >
                  <div className="flex w-full items-center gap-2">
                    <span className="font-mono text-caption font-medium text-[var(--ink)]">{p.exe}</span>
                    {added && <Badge tone="neutral">已添加</Badge>}
                  </div>
                  <span className="mt-0.5 w-full truncate text-caption text-[var(--ink-muted)]">{p.title}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

function PathCopyButton({ path }: { path: string }) {
  const { copied, copy } = useCopy();
  return (
    <Button
      size="sm"
      variant="subtle"
      icon={copied ? <Check className="size-3.5 text-[var(--good)]" /> : <Copy className="size-3.5" />}
      onClick={() => copy(path)}
    >
      {copied ? "已复制" : "复制路径"}
    </Button>
  );
}

function AboutSection({ info }: { info: AppInfo | null }) {
  const snapshot = useUpdater((s) => s.snapshot);
  const check = useUpdater((s) => s.check);
  const install = useUpdater((s) => s.install);

  const avail = snapshot.available;
  const checking = snapshot.phase === "checking";
  const downloading = snapshot.phase === "downloading";
  const installing = snapshot.phase === "installing";
  const prog = snapshot.progress;
  const { ratio, text: progressText } = downloadProgress(prog);

  let statusHint: ReactNode = null;
  if (snapshot.phase === "upToDate" && snapshot.manual) {
    statusHint = <span className="font-medium text-[var(--good)]">已是最新版本</span>;
  } else if (snapshot.phase === "error" && snapshot.error) {
    statusHint = <span className="text-[var(--danger)]">{snapshot.error}</span>;
  } else if (downloading) {
    statusHint = (
      <span className="tabular-nums">
        正在下载 v{avail?.version}… {progressText}
      </span>
    );
  } else if (installing) {
    statusHint = <span>正在安装 v{avail?.version}，完成后将自动重启…</span>;
  }

  return (
    <div className="flex flex-col gap-4">
      <SectionHeader title="关于与更新" />

      {/* Brand Identity & Updater Hero Card */}
      <Card className="flex flex-col gap-3.5 p-5">
        <div className="flex flex-wrap items-center gap-4">
          <BrandSeal size="lg" />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-large font-semibold text-[var(--ink)]">LLM Translate</h3>
              {info && <Badge tone="neutral">v{info.version}</Badge>}
              {avail && <Badge tone="accent">新版本 v{avail.version}</Badge>}
            </div>
            {statusHint && (
              <div className="mt-1 text-caption text-[var(--ink-secondary)]">{statusHint}</div>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {avail ? (
              <>
                {!downloading && !installing && (
                  <Button size="sm" variant="subtle" icon={<RefreshCw className="size-3.5" />} onClick={check}>
                    重新检查
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="primary"
                  icon={downloading || installing ? <Spinner className="size-3.5" /> : <Download className="size-3.5" />}
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
              <Button
                size="sm"
                icon={checking ? <Spinner className="size-3.5" /> : <RefreshCw className="size-3.5" />}
                disabled={checking}
                onClick={check}
              >
                {checking ? "正在检查…" : "检查更新"}
              </Button>
            )}
          </div>
        </div>

        {downloading && <Progress value={ratio} indeterminate={!prog?.total} />}
        {installing && <Progress value={1} indeterminate />}

        {avail && (avail.notes || avail.pubDate) && (
          <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface-alt)] px-3.5 py-2.5 text-caption">
            <div className="mb-1 flex items-center justify-between text-[var(--ink-secondary)]">
              <span className="font-medium text-[var(--ink)]">v{avail.version} 更新内容</span>
              {avail.pubDate && <span className="tabular-nums">{avail.pubDate}</span>}
            </div>
            {avail.notes && (
              <div className="selectable max-h-40 overflow-y-auto leading-relaxed whitespace-pre-wrap text-[var(--ink-secondary)]">
                {avail.notes}
              </div>
            )}
          </div>
        )}
      </Card>

      <div className="flex flex-col gap-2.5">
        <SettingRow
          title="配置文件位置"
          desc={
            <span className="selectable block truncate font-mono text-caption" title={info?.configDir}>
              {info ? `${info.configDir}\\settings.json` : ""}
            </span>
          }
        >
          {info && <PathCopyButton path={`${info.configDir}\\settings.json`} />}
        </SettingRow>

        <SettingRow
          title="运行日志目录"
          desc={
            <span className="selectable block truncate font-mono text-caption" title={info?.logsDir}>
              {info?.logsDir}
            </span>
          }
        >
          <Button size="sm" icon={<FolderOpen className="size-3.5" />} onClick={() => api.openLogsDir()}>
            打开日志目录
          </Button>
        </SettingRow>
      </div>
    </div>
  );
}

export default function SettingsPage({
  hotkeys,
  visible,
  section,
  onSectionChange,
}: {
  hotkeys: HotkeyInfo[];
  visible: boolean;
  section: SettingsSection;
  onSectionChange: (s: SettingsSection) => void;
}) {
  const settings = useSettings((s) => s.settings);
  const update = useUpdater((s) => s.snapshot);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [modelsMounted, setModelsMounted] = useState(false);

  useEffect(() => {
    api.appInfo().then(setInfo);
  }, []);

  // Only trigger hardware detection once the Models section is actually visited.
  useEffect(() => {
    if (visible && section === "models") setModelsMounted(true);
  }, [visible, section]);

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartError, setAutostartError] = useState<string | null>(null);

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
  const hasFailedHotkeys = hotkeys.some((h) => h.keys && !h.registered);
  const hasUpdate = !!update.available && settings?.ignoredUpdateVersion !== update.available.version;

  return (
    <div className="flex min-h-0 flex-1 flex-col min-[720px]:flex-row">
      {/* Category Sidebar (horizontal pill bar on narrow viewports) */}
      <aside className="flex shrink-0 gap-1 overflow-x-auto border-b border-[var(--border-subtle)] px-3 py-2.5 min-[720px]:w-[186px] min-[720px]:flex-col min-[720px]:border-r min-[720px]:border-b-0 min-[720px]:px-3 min-[720px]:py-4">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const active = section === item.id;
          const dotTone =
            item.id === "hotkeys" && hasFailedHotkeys
              ? "bg-[var(--warn)]"
              : item.id === "about" && hasUpdate
                ? "bg-[var(--accent)]"
                : null;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onSectionChange(item.id)}
              className={`relative flex shrink-0 cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-body whitespace-nowrap transition-colors ${
                active
                  ? "bg-[var(--bg-surface)] font-semibold text-[var(--ink)] shadow-[var(--shadow-xs)]"
                  : "text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
              }`}
            >
              {active && (
                <span className="hidden min-[720px]:block absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-r-full bg-[var(--accent)]" />
              )}
              <Icon className={`size-4 shrink-0 ${active ? "text-[var(--accent-text)]" : "opacity-75"}`} />
              <span className="flex-1 text-left">{item.label}</span>
              {dotTone && <span className={`size-1.5 shrink-0 rounded-full ${dotTone}`} />}
            </button>
          );
        })}
      </aside>

      {/* Active Category Panel */}
      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5 pb-10">
        <div className="mx-auto max-w-[660px]">
          {section === "general" && (
            <div className="flex flex-col gap-4">
              <SectionHeader title="常规与外观" />
              <ThemePicker value={settings?.theme ?? "system"} onChange={(t) => api.setTheme(t)} />
              <SettingRow
                title="开机自动启动"
                desc={
                  autostartError ? (
                    <span className="text-[var(--danger)]">{autostartError}</span>
                  ) : (
                    "开机后在后台自动运行"
                  )
                }
              >
                <Toggle checked={autostart ?? false} disabled={autostart === null} onChange={toggleAutostart} />
              </SettingRow>
            </div>
          )}

          {section === "hotkeys" && (
            <div className="flex flex-col gap-3">
              <SectionHeader title="全局快捷键" />
              <SettingRow
                title="迷你翻译窗"
                desc={
                  hk("mini")?.error ? (
                    <span className="text-[var(--warn)]">{hk("mini")?.error}</span>
                  ) : (
                    "快速打开悬浮输入框"
                  )
                }
              >
                <HotkeyRecorder id="mini" info={hk("mini")} visible={visible && section === "hotkeys"} />
              </SettingRow>
              <SettingRow
                title="截图翻译"
                desc={
                  hk("shot")?.error ? (
                    <span className="text-[var(--warn)]">{hk("shot")?.error}</span>
                  ) : (
                    "框选屏幕区域直接翻译"
                  )
                }
              >
                <HotkeyRecorder id="shot" info={hk("shot")} visible={visible && section === "hotkeys"} />
              </SettingRow>
              <SettingRow
                title="划词翻译开关"
                desc={
                  hk("selection")?.error ? (
                    <span className="text-[var(--warn)]">{hk("selection")?.error}</span>
                  ) : (
                    "一键开启或暂停划词翻译"
                  )
                }
              >
                <HotkeyRecorder id="selection" info={hk("selection")} visible={visible && section === "hotkeys"} />
              </SettingRow>
            </div>
          )}

          {section === "selection" && <SelectionSection />}

          {section === "tts" && <TtsSection active={visible && section === "tts"} />}

          {/* Keep ModelSection mounted once visited so active downloads & hardware info survive tab switching. */}
          {modelsMounted && (
            <div className={section === "models" ? "block" : "hidden"}>
              <ModelSection />
            </div>
          )}

          {section === "about" && <AboutSection info={info} />}
        </div>
      </div>
    </div>
  );
}
