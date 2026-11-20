// Main window (DESIGN 5.4 / 5.8): compact navigation rail + Translate / Settings pages.
import { useEffect, useState, type ReactNode } from "react";
import { AlertTriangle, Languages, Settings as SettingsIcon, X } from "lucide-react";
import { api, type HotkeyInfo } from "../../lib/api";
import { startInferenceSync, useInference } from "../../stores/inference";
import TranslatePage from "./TranslatePage";
import SettingsPage from "./SettingsPage";

type Page = "translate" | "settings";

function NavItem({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: ReactNode;
  label: string;
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
      {icon}
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

export default function Main() {
  const [page, setPage] = useState<Page>("translate");
  const [hotkeys, setHotkeys] = useState<HotkeyInfo[]>([]);
  const [hotkeyDismissed, setHotkeyDismissed] = useState(false);
  const notice = useInference((s) => s.notice);
  const dismissNotice = useInference((s) => s.dismissNotice);

  useEffect(() => {
    startInferenceSync();
    api.hotkeys().then(setHotkeys);
  }, []);

  const failedHotkeys = hotkeys.filter((h) => !h.registered);

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
          onClick={() => setPage("settings")}
        />
      </nav>

      <main className="flex min-w-0 flex-1 flex-col rounded-tl-lg border-t border-l border-[var(--stroke)] bg-[var(--surface)]">
        {(notice || (failedHotkeys.length > 0 && !hotkeyDismissed)) && (
          <div className="flex flex-col gap-2 px-5 pt-4">
            {notice && (
              <Banner onClose={dismissNotice}>
                GPU 后端多次失败，已切换到 CPU 并保存设置。原因：{notice.reason}
              </Banner>
            )}
            {failedHotkeys.length > 0 && !hotkeyDismissed && (
              <Banner onClose={() => setHotkeyDismissed(true)}>
                快捷键 {failedHotkeys.map((h) => h.keys).join("、")} 已被其他程序占用，迷你翻译窗只能从翻译页按钮打开。
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
