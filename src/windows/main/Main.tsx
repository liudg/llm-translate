// Main window: unified frameless editorial title bar + Translate Studio & Settings.
import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  ChevronDown,
  ChevronUp,
  Download,
  Moon,
  PanelTop,
  ScanText,
  Sun,
  X,
} from "lucide-react";
import { api, events, type HotkeyInfo } from "../../lib/api";
import { useResolvedTheme } from "../../lib/theme";
import { BrandSeal, Button, Progress, Segmented, Spinner } from "../../components/ui";
import { TitleBar } from "../../components/TitleBar";
import { StatusPill } from "../../components/status";
import { startInferenceSync, useInference } from "../../stores/inference";
import { useSettings } from "../../stores/settings";
import { downloadProgress, startUpdaterSync, useUpdater } from "../../stores/updater";
import TranslatePage, { type IncomingTranslate } from "./TranslatePage";
import SettingsPage, { type SettingsSection } from "./SettingsPage";

type Page = "translate" | "settings";

function Banner({ children, onClose }: { children: ReactNode; onClose?: () => void }) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg border border-[var(--warn)]/25 bg-[var(--warn-soft)] px-3 py-2 text-caption text-[var(--warn)]">
      <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
      <span className="flex-1 leading-relaxed">{children}</span>
      {onClose && (
        <button
          type="button"
          onClick={onClose}
          className="cursor-pointer opacity-65 hover:opacity-100"
          title="关闭"
        >
          <X className="size-3.5" />
        </button>
      )}
    </div>
  );
}

function UpdateBanner() {
  const snapshot = useUpdater((s) => s.snapshot);
  const install = useUpdater((s) => s.install);
  const ignore = useUpdater((s) => s.ignore);
  const dismissBanner = useUpdater((s) => s.dismissBanner);
  const [showNotes, setShowNotes] = useState(false);

  const avail = snapshot.available;
  if (!avail) return null;

  const isDownloading = snapshot.phase === "downloading";
  const isInstalling = snapshot.phase === "installing";
  const isError = snapshot.phase === "error";
  const prog = snapshot.progress;
  const { ratio, text: progressText } = downloadProgress(prog);

  return (
    <div
      className={`flex flex-col gap-2 rounded-lg border px-3 py-2 text-caption ${
        isError
          ? "border-[var(--danger)]/25 bg-[var(--danger-soft)] text-[var(--danger)]"
          : "border-[var(--accent)]/25 bg-[var(--accent-soft)] text-[var(--ink)]"
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        {isDownloading || isInstalling ? (
          <Spinner className="size-3.5 shrink-0 text-[var(--accent-text)]" />
        ) : isError ? (
          <AlertTriangle className="size-3.5 shrink-0" />
        ) : (
          <Download className="size-3.5 shrink-0 text-[var(--accent-text)]" />
        )}

        <div className="min-w-0 flex-1">
          {isDownloading && (
            <span>
              正在下载新版本 v{avail.version}… <span className="tabular-nums">{progressText}</span>
            </span>
          )}
          {isInstalling && <span>正在安装 v{avail.version}，完成后将自动重启…</span>}
          {isError && <span>更新 v{avail.version} 失败：{snapshot.error}</span>}
          {snapshot.phase === "available" && (
            <span>
              发现新版本 <strong className="font-semibold">v{avail.version}</strong>
              <span className="text-[var(--ink-secondary)]">（当前 v{avail.currentVersion}）</span>
              {avail.notes && (
                <button
                  type="button"
                  onClick={() => setShowNotes((v) => !v)}
                  className="ml-2 inline-flex cursor-pointer items-center gap-0.5 font-medium text-[var(--accent-text)] hover:underline"
                >
                  更新说明
                  {showNotes ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
                </button>
              )}
            </span>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {snapshot.phase === "available" && (
            <>
              <Button variant="primary" size="sm" onClick={install}>
                立即更新
              </Button>
              <Button variant="subtle" size="sm" onClick={() => ignore(avail.version)}>
                忽略此版本
              </Button>
            </>
          )}
          {isError && (
            <Button variant="primary" size="sm" onClick={install}>
              重试
            </Button>
          )}
          {!isDownloading && !isInstalling && (
            <button
              type="button"
              onClick={() => dismissBanner(avail.version)}
              className="inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
              title="暂时关闭"
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>
      </div>

      {isDownloading && <Progress value={ratio} indeterminate={!prog?.total} />}
      {isInstalling && <Progress value={1} indeterminate />}
      {showNotes && avail.notes && snapshot.phase === "available" && (
        <div className="selectable max-h-36 overflow-y-auto rounded-md border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 leading-relaxed whitespace-pre-wrap text-[var(--ink-secondary)]">
          {avail.notes}
        </div>
      )}
    </div>
  );
}

export default function Main() {
  const [page, setPage] = useState<Page>("translate");
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
  const [incoming, setIncoming] = useState<IncomingTranslate | null>(null);
  const [hotkeys, setHotkeys] = useState<HotkeyInfo[]>([]);
  const [hotkeyDismissed, setHotkeyDismissed] = useState(false);

  const notice = useInference((s) => s.notice);
  const dismissNotice = useInference((s) => s.dismissNotice);
  const settings = useSettings((s) => s.settings);
  const update = useUpdater((s) => s.snapshot);
  const dismissedVersion = useUpdater((s) => s.dismissedVersion);

  const openSettingsAt = useCallback((section: SettingsSection) => {
    setSettingsSection(section);
    setPage("settings");
  }, []);

  useEffect(() => {
    startInferenceSync();
    startUpdaterSync();
    api.hotkeys().then(setHotkeys);
    const unHotkeys = events.hotkeys((h) => {
      setHotkeys(h);
      setHotkeyDismissed(false);
    });
    const unNav = events.navigate((p) => {
      if (p === "settings" || p === "translate") setPage(p);
    });
    const unOpenTranslate = events.openTranslate((payload) => {
      setPage("translate");
      setIncoming((prev) => ({ payload, seq: (prev?.seq ?? 0) + 1 }));
    });
    return () => {
      unHotkeys.then((f) => f());
      unNav.then((f) => f());
      unOpenTranslate.then((f) => f());
    };
  }, []);

  // Ctrl+, toggles between Translate and Settings.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key === ",") {
        e.preventDefault();
        setPage((p) => (p === "settings" ? "translate" : "settings"));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const resolvedTheme = useResolvedTheme();
  const isDark = resolvedTheme === "dark";

  const toggleLightDark = () => {
    api.setTheme(isDark ? "light" : "dark");
  };

  const miniKeys = hotkeys.find((h) => h.id === "mini" && h.registered)?.keys;
  const shotKeys = hotkeys.find((h) => h.id === "shot" && h.registered)?.keys;

  // Bound but not registered (taken by another program at startup).
  const failedHotkeys = hotkeys.filter((h) => h.keys && !h.registered);
  const avail = update.available;
  const updateIgnored = !!avail && settings?.ignoredUpdateVersion === avail.version;
  const updateDismissed = !!avail && dismissedVersion === avail.version;
  const showUpdateBanner =
    update.phase === "downloading" ||
    update.phase === "installing" ||
    (update.phase === "error" && !!avail && !updateDismissed) ||
    (update.phase === "available" && !!avail && !updateIgnored && !updateDismissed);
  const showSettingsDot = !!avail && !updateIgnored;
  const hasBanners = showUpdateBanner || !!notice || (failedHotkeys.length > 0 && !hotkeyDismissed);

  return (
    <div className="app-shell flex h-full flex-col overflow-hidden">
      <TitleBar
        left={
          <>
            <div data-tauri-drag-region className="flex items-center gap-2 pr-1">
              <BrandSeal size="sm" className="pointer-events-none" />
              <span className="pointer-events-none hidden text-body font-semibold tracking-tight text-[var(--ink)] min-[820px]:inline">
                LLM Translate
              </span>
            </div>
            <Segmented<Page>
              size="sm"
              value={page}
              onChange={setPage}
              options={[
                { value: "translate", label: "翻译", title: "文本翻译（Ctrl+,）" },
                { value: "settings", label: "设置", title: "偏好与模型设置（Ctrl+,）", dot: showSettingsDot },
              ]}
            />
          </>
        }
        right={
          <>
            <StatusPill onClick={() => openSettingsAt("models")} />
            <span className="mx-0.5 h-4 w-px bg-[var(--border)]" />
            <Button
              variant="subtle"
              size="sm"
              icon={<PanelTop className="size-3.5" />}
              onClick={() => api.openMini()}
              title={`打开迷你翻译窗${miniKeys ? `（${miniKeys}）` : ""}`}
            >
              <span className="hidden min-[760px]:inline">迷你窗</span>
            </Button>
            <Button
              variant="subtle"
              size="sm"
              icon={<ScanText className="size-3.5" />}
              onClick={() => api.shotStart()}
              title={`截图翻译：框选屏幕区域覆盖译文${shotKeys ? `（${shotKeys}）` : ""}`}
            >
              <span className="hidden min-[760px]:inline">截图</span>
            </Button>
            <button
              type="button"
              onClick={toggleLightDark}
              title={isDark ? "切换到宣纸浅色" : "切换到墨砚深色"}
              className="inline-flex size-7 cursor-pointer items-center justify-center rounded-md text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
            >
              {isDark ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
            </button>
          </>
        }
      />

      <main className="flex min-h-0 flex-1 flex-col">
        {hasBanners && (
          <div className="flex shrink-0 flex-col gap-2 px-4 pt-3">
            {showUpdateBanner && <UpdateBanner />}
            {notice && (
              <Banner onClose={dismissNotice}>
                GPU 后端多次失败，已自动切换到 CPU 并保存设置。原因：{notice.reason}
              </Banner>
            )}
            {failedHotkeys.length > 0 && !hotkeyDismissed && (
              <Banner onClose={() => setHotkeyDismissed(true)}>
                快捷键 {failedHotkeys.map((h) => h.keys).join("、")} 已被其他程序占用，可前往
                <button
                  type="button"
                  className="mx-1 cursor-pointer font-semibold underline"
                  onClick={() => openSettingsAt("hotkeys")}
                >
                  快捷键设置
                </button>
                更换；迷你窗和截图翻译也可以从顶栏或托盘菜单打开。
              </Banner>
            )}
          </div>
        )}

        {/* Both pages stay mounted so the translate text survives a trip to settings. */}
        <div className={page === "translate" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
          <TranslatePage hotkeys={hotkeys} visible={page === "translate"} incoming={incoming} />
        </div>
        <div className={page === "settings" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
          <SettingsPage
            hotkeys={hotkeys}
            visible={page === "settings"}
            section={settingsSection}
            onSectionChange={setSettingsSection}
          />
        </div>
      </main>
    </div>
  );
}
