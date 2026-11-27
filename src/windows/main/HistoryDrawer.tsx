// Slide-over translation history drawer for the main studio.
import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Clock, Copy, FileText, Search, Trash2, X } from "lucide-react";
import { useHistory, type HistoryItem, type HistoryOrigin } from "../../stores/history";
import { useCopy } from "../../lib/useCopy";
import { isImeComposing } from "../../lib/useTranslator";
import { useTts } from "../../lib/useTts";
import { Badge, Button, IconButton } from "../../components/ui";
import { SpeakButton } from "../../components/tts";

const ORIGIN_LABEL: Partial<Record<HistoryOrigin, string>> = {
  mini: "迷你窗",
  popup: "划词",
};

function formatRelativeTime(ts: number): string {
  const diff = Math.max(0, Date.now() - ts);
  const sec = Math.floor(diff / 1000);
  if (sec < 45) return "刚刚";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} 分钟前`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr} 小时前`;
  const d = new Date(ts);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${mi}`;
}

function HighlightText({ text, query }: { text: string; query: string }) {
  const q = query.trim();
  if (!q) return <>{text}</>;
  const escaped = q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const parts = text.split(new RegExp(`(${escaped})`, "gi"));
  const lower = q.toLowerCase();
  return (
    <>
      {parts.map((part, i) =>
        part.toLowerCase() === lower ? (
          <mark
            key={i}
            className="rounded-md bg-[var(--accent-soft)] px-0.5 text-[var(--accent-text)]"
          >
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

function HistoryCard({
  item,
  query,
  onSelect,
  onDelete,
}: {
  item: HistoryItem;
  query: string;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const { copied: copiedResult, copy: copyResult } = useCopy();
  const { copied: copiedSource, copy: copySource } = useCopy();
  const tts = useTts();
  const dirLabel = item.mixed ? "中英混合" : item.direction === "zh2En" ? "中 → 英" : "英 → 中";
  const originLabel = item.origin ? ORIGIN_LABEL[item.origin] : undefined;
  const srcKey = `history:${item.id}:source`;
  const resKey = `history:${item.id}:result`;
  const srcPhase = tts.phaseOf(srcKey);
  const resPhase = tts.phaseOf(resKey);
  const srcLang = item.mixed ? null : item.direction === "zh2En" ? "zh" : "en";
  const resLang = item.mixed ? null : item.direction === "zh2En" ? "en" : "zh";

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className="group flex cursor-pointer flex-col gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 text-left transition-colors hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)]"
    >
      <div className="flex items-center gap-1.5">
        <Badge tone="neutral">{dirLabel}</Badge>
        {originLabel && <Badge tone="accent">{originLabel}</Badge>}
        <span className="text-caption text-[var(--ink-muted)] tabular-nums">
          {formatRelativeTime(item.timestamp)}
        </span>
        <div
          className="ml-auto flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100"
          onClick={(e) => e.stopPropagation()}
        >
          <IconButton
            size="sm"
            title={copiedSource ? "已复制原文" : "复制原文"}
            onClick={() => copySource(item.source)}
          >
            {copiedSource ? (
              <Check className="size-3.5 text-[var(--good)]" />
            ) : (
              <FileText className="size-3.5" />
            )}
          </IconButton>
          <IconButton
            size="sm"
            title={copiedResult ? "已复制译文" : "复制译文"}
            onClick={() => copyResult(item.result)}
          >
            {copiedResult ? (
              <Check className="size-3.5 text-[var(--good)]" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </IconButton>
          <IconButton size="sm" title="删除此条" onClick={onDelete}>
            <Trash2 className="size-3.5" />
          </IconButton>
        </div>
      </div>

      <div className="flex items-start gap-1.5">
        <p className="line-clamp-2 min-w-0 flex-1 text-caption leading-relaxed text-[var(--ink-secondary)]">
          <HighlightText text={item.source} query={query} />
        </p>
        <div
          className={`shrink-0 transition-opacity ${
            srcPhase !== "idle" ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-within:opacity-100"
          }`}
          onClick={(e) => e.stopPropagation()}
        >
          <SpeakButton
            size="xs"
            phase={srcPhase}
            title="朗读原文"
            onClick={() => tts.toggle(srcKey, item.source, { lang: srcLang })}
          />
        </div>
      </div>

      <div className="flex items-start gap-1.5">
        <p className="line-clamp-2 min-w-0 flex-1 text-body leading-relaxed font-medium text-[var(--ink)]">
          <HighlightText text={item.result} query={query} />
        </p>
        <div
          className={`shrink-0 transition-opacity ${
            resPhase !== "idle" ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-within:opacity-100"
          }`}
          onClick={(e) => e.stopPropagation()}
        >
          <SpeakButton
            size="xs"
            phase={resPhase}
            title="朗读译文"
            onClick={() => tts.toggle(resKey, item.result, { lang: resLang })}
          />
        </div>
      </div>
    </div>
  );
}

export function HistoryDrawer({
  open,
  onClose,
  onRestore,
}: {
  open: boolean;
  onClose: () => void;
  onRestore: (item: HistoryItem) => void;
}) {
  const items = useHistory((s) => s.items);
  const remove = useHistory((s) => s.remove);
  const clear = useHistory((s) => s.clear);
  const tts = useTts();
  const stopHistoryTts = tts.stopIfKeyPrefix;
  const [query, setQuery] = useState("");
  const [confirmClear, setConfirmClear] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) {
      setConfirmClear(false);
      stopHistoryTts("history:");
      return;
    }
    const id = window.setTimeout(() => searchRef.current?.focus(), 40);
    return () => window.clearTimeout(id);
  }, [open, stopHistoryTts]);

  // Three-tier Escape handling: confirmClear -> clear query -> close drawer.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || isImeComposing(e)) return;
      e.preventDefault();
      e.stopPropagation();
      if (confirmClear) {
        setConfirmClear(false);
      } else if (query) {
        setQuery("");
        searchRef.current?.focus();
      } else {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, onClose, confirmClear, query]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (it) => it.source.toLowerCase().includes(q) || it.result.toLowerCase().includes(q),
    );
  }, [items, query]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 top-[46px] z-30 flex justify-end">
      {/* Subtle scrim */}
      <div
        className="flex-1 bg-black/15 backdrop-blur-[1px] transition-opacity dark:bg-black/40"
        onClick={onClose}
      />

      {/* Drawer panel */}
      <aside
        role="dialog"
        aria-modal="true"
        aria-label="翻译历史"
        className="animate-drawer-in flex h-full w-[370px] max-w-[88vw] flex-col border-l border-[var(--border)] bg-[var(--bg-canvas)] shadow-[var(--shadow-popover)]"
      >
        <div className="flex h-12 shrink-0 items-center gap-2 border-b border-[var(--border-subtle)] px-4">
          <Clock className="size-4 text-[var(--accent-text)]" />
          <h2 className="text-large font-semibold text-[var(--ink)]">翻译历史</h2>
          {items.length > 0 && <Badge>{items.length}</Badge>}

          <div className="ml-auto flex items-center gap-1">
            {items.length > 0 &&
              (confirmClear ? (
                <div className="flex items-center gap-1">
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => {
                      clear();
                      setConfirmClear(false);
                    }}
                  >
                    确认清空
                  </Button>
                  <Button variant="subtle" size="sm" onClick={() => setConfirmClear(false)}>
                    取消
                  </Button>
                </div>
              ) : (
                <Button variant="subtle" size="sm" onClick={() => setConfirmClear(true)}>
                  清空
                </Button>
              ))}
            <IconButton size="sm" title="关闭（Esc）" onClick={onClose}>
              <X className="size-4" />
            </IconButton>
          </div>
        </div>

        {/* Search */}
        <div className="shrink-0 px-3 pt-3 pb-2">
          <div className="flex h-8 items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 focus-within:border-[var(--accent)] focus-within:ring-2 focus-within:ring-[var(--accent-ring)]">
            <Search className="size-3.5 shrink-0 text-[var(--ink-muted)]" />
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索原文或译文…"
              className="min-w-0 flex-1 bg-transparent text-caption text-[var(--ink)] placeholder:text-[var(--ink-muted)]"
            />
            {query && (
              <button
                type="button"
                aria-label="清除搜索"
                onClick={() => setQuery("")}
                className="cursor-pointer text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                <X className="size-3.5" />
              </button>
            )}
          </div>
          {tts.error && tts.errorKey?.startsWith("history:") && (
            <div className="mt-2 truncate rounded-lg border border-[var(--danger)]/20 bg-[var(--danger-soft)] px-2.5 py-1.5 text-caption text-[var(--danger)]">
              {tts.error}
            </div>
          )}
        </div>

        {/* List */}
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pt-1 pb-4">
          {filtered.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-16 text-center">
              <div className="mb-2.5 flex size-10 items-center justify-center rounded-full bg-[var(--bg-recessed)] text-[var(--ink-muted)]">
                <Clock className="size-4" />
              </div>
              <p className="text-caption font-medium text-[var(--ink-secondary)]">
                {items.length === 0 ? "暂无翻译记录" : "未找到匹配的历史记录"}
              </p>
              <p className="mt-1 text-caption text-[var(--ink-muted)]">
                {items.length === 0 ? "完成的翻译会自动保存在本地，方便随时回看" : "试试更换其他关键词"}
              </p>
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              {filtered.map((item) => (
                <HistoryCard
                  key={item.id}
                  item={item}
                  query={query}
                  onSelect={() => {
                    onRestore(item);
                    onClose();
                  }}
                  onDelete={() => remove(item.id)}
                />
              ))}
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}
