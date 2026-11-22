// Main window (DESIGN 5.4 / 5.8): compact navigation rail + Translate / Settings pages.
import { useEffect, useState, type ReactNode } from "react";
import { AlertTriangle, ChevronDown, ChevronUp, Download, Languages, Settings as SettingsIcon, X } from "lucide-react";
import { api, events, type HotkeyInfo } from "../../lib/api";
import { bytes } from "../../lib/format";
import { Button, Progress, Spinner } from "../../components/ui";
import { startInferenceSync, useInference } from "../../stores/inference";
import { useSettings } from "../../stores/settings";
import { startUpdaterSync, useUpdater } from "../../stores/updater";
import TranslatePage from "./TranslatePage";
import SettingsPage from "./SettingsPage";

type Page = "translate" | "settings";

function NavItem({
  active,
  icon,
  label,
  dot = false,
  onClick,
}: {
  active: boolean;
  icon: ReactNode;
  label: string;
  dot?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`relative flex w-14 flex-col items-center gap-0.5 rounded-md py-2 text-[11px] transition-colors ${
        active ? "bg-black/[0.05] dark:bg-white/[0.07]" : "opacity-75 hover:bg-black/[0.04] dark:hover:bg-white/[0.05]"
      }`}
    >
      {active && <span className="absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-full bg-[var(--accent)]" />}
      <span className="relative">
        {icon}
        {dot && <span className="absolute -top-0.5 -right-1 size-2 rounded-full bg-[var(--accent)]" />}
      </span>
      {label}
    </button>
  );
}

function Banner({ children, onClose }: { children: ReactNode; onClose?: () => void }) {
  return (
    <div className="flex items-start gap-2 rounded-md border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-300">
      <AlertTriangle className="mt-px size-4 shrink-0" />
      <span className="flex-1">{children}</span>
      {onClose && (
        <button onClick={onClose} className="opacity-60 hover:opacity-100" title="关闭">
          <X className="size-4" />
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
  const ratio = prog?.total ? prog.downloaded / prog.total : 0;

  return (
    <div
      className={`flex flex-col gap-2 rounded-md border px-3 py-2 text-xs ${
        isError
          ? "border-amber-500/25 bg-amber-500/10 text-amber-800 dark:text-amber-300"
          : "border-[var(--accent)]/25 bg-[var(--accent)]/10"
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        {isDownloading || isInstalling ? (
          <Spinner className="size-4 shrink-0 text-[var(--accent-text)]" />
        ) : isError ? (
          <AlertTriangle className="size-4 shrink-0" />
        ) : (
          <Download className="size-4 shrink-0 text-[var(--accent-text)]" />
        )}

        <div className="min-w-0 flex-1">
          {isDownloading && (
            <span>
              正在下载新版本 v{avail.version}…{" "}
              {prog?.total
                ? `${Math.min(100, Math.round(ratio * 100))}%（${bytes(prog.downloaded)} / ${bytes(prog.total)}）`
                : prog
                  ? bytes(prog.downloaded)
                  : ""}
            </span>
          )}
          {isInstalling && <span>正在安装 v{avail.version}，完成后将自动重启…</span>}
          {isError && <span>更新 v{avail.version} 失败：{snapshot.error}</span>}
          {snapshot.phase === "available" && (
            <span>
              发现新版本 <strong>v{avail.version}</strong>（当前 v{avail.currentVersion}）
              {avail.notes && (
                <button
                  onClick={() => setShowNotes((v) => !v)}
                  className="ml-2 inline-flex items-center gap-0.5 text-[var(--accent-text)] hover:underline"
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
              <Button variant="primary" className="h-7 px-2.5 text-xs" onClick={install}>
                立即更新
              </Button>
              <Button variant="subtle" className="h-7 px-2 text-xs opacity-75" onClick={() => ignore(avail.version)}>
                忽略此版本
              </Button>
            </>
          )}
          {isError && (
            <Button variant="primary" className="h-7 px-2.5 text-xs" onClick={install}>
              重试
            </Button>
          )}
          {!isDownloading && !isInstalling && (
            <button
              onClick={() => dismissBanner(avail.version)}
              className="inline-flex size-6 items-center justify-center rounded opacity-60 hover:bg-black/5 hover:opacity-100 dark:hover:bg-white/10"
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
        <div className="selectable max-h-36 overflow-y-auto rounded bg-black/[0.04] px-2.5 py-2 leading-relaxed whitespace-pre-wrap opacity-85 dark:bg-white/[0.06]">
          {avail.notes}
        </div>
      )}
    </div>
  );
}

export default function Main() {
  const [page, setPage] = useState<Page>("translate");
  const [hotkeys, setHotkeys] = useState<HotkeyInfo[]>([]);
  const [hotkeyDismissed, setHotkeyDismissed] = useState(false);
  const notice = useInference((s) => s.notice);
  const dismissNotice = useInference((s) => s.dismissNotice);
  const settings = useSettings((s) => s.settings);
  const update = useUpdater((s) => s.snapshot);
  const dismissedVersion = useUpdater((s) => s.dismissedVersion);

  useEffect(() => {
    startInferenceSync();
    startUpdaterSync();
    api.hotkeys().then(setHotkeys);
    const unHotkeys = events.hotkeys((h) => {
      setHotkeys(h);
      setHotkeyDismissed(false);
    });
    const unNav = events.navigate((p) => (p === "settings" || p === "translate") && setPage(p));
    return () => {
      unHotkeys.then((f) => f());
      unNav.then((f) => f());
    };
  }, []);

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
    <div className="flex h-full">
      <nav className="flex w-[68px] shrink-0 flex-col items-center gap-1 py-3">
        <NavItem
          active={page === "translate"}
          icon={<Languages className="size-5" />}
          label="翻译"
          onClick={() => setPage("translate")}
        />
        <div className="flex-1" />
        <NavItem
          active={page === "settings"}
          icon={<SettingsIcon className="size-5" />}
          label="设置"
          dot={showSettingsDot}
          onClick={() => setPage("settings")}
        />
      </nav>

      <main className="flex min-w-0 flex-1 flex-col rounded-tl-lg border-t border-l border-[var(--stroke)] bg-[var(--surface)]">
        {hasBanners && (
          <div className="flex flex-col gap-2 px-5 pt-4">
            {showUpdateBanner && <UpdateBanner />}
            {notice && (
              <Banner onClose={dismissNotice}>
                GPU 后端多次失败，已切换到 CPU 并保存设置。原因：{notice.reason}
              </Banner>
            )}
            {failedHotkeys.length > 0 && !hotkeyDismissed && (
              <Banner onClose={() => setHotkeyDismissed(true)}>
                快捷键 {failedHotkeys.map((h) => h.keys).join("、")} 已被其他程序占用，可在
                <button className="mx-0.5 underline" onClick={() => setPage("settings")}>
                  设置
                </button>
                中更换；迷你窗和截图翻译也可以从翻译页按钮或托盘菜单打开。
              </Banner>
            )}
          </div>
        )}
        {/* Both pages stay mounted so the translate text survives a trip to settings. */}
        <div className={page === "translate" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
          <TranslatePage hotkeys={hotkeys} />
        </div>
        <div className={page === "settings" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
          <SettingsPage hotkeys={hotkeys} visible={page === "settings"} />
        </div>
      </main>
    </div>
  );
}
