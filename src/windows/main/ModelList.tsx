// Model management (DESIGN 5.8): editorial tier cards with live download progress,
// resume/cancel, self-tested model switch and safe delete confirmation.
import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, Download, HardDrive, Pause, Play, Sparkles, Trash2, X } from "lucide-react";
import {
  api,
  errorText,
  events,
  SELF_TEST_TEXT,
  type DownloadError,
  type LaunchOutcome,
  type ModelView,
} from "../../lib/api";
import { backendName, gb, speed } from "../../lib/format";
import { Badge, Button, Progress, Spinner } from "../../components/ui";
import { StatusChip } from "../../components/status";
import { useSettings } from "../../stores/settings";

interface Transfer {
  done: number;
  total: number;
  note: string;
  verifying: boolean;
  pausing: boolean;
}

type Message = { tone: "good" | "warn" | "error"; text: string };

function splitTierName(name: string): { tier: string | null; title: string } {
  const parts = name.split("·").map((s) => s.trim());
  if (parts.length >= 2) {
    return { tier: parts[0], title: parts.slice(1).join(" · ") };
  }
  return { tier: null, title: name };
}

export function ModelList({ onBusy }: { onBusy: (busy: boolean) => void }) {
  const settings = useSettings((s) => s.settings);
  const [models, setModels] = useState<ModelView[]>([]);
  const [transfers, setTransfers] = useState<Record<string, Transfer>>({});
  const [switching, setSwitching] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [message, setMessage] = useState<Message | null>(null);
  const dir = settings?.modelsDir;

  const refresh = useCallback(() => {
    if (dir) api.models(dir).then(setModels);
  }, [dir]);

  useEffect(refresh, [refresh, settings?.modelId]);
  useEffect(() => {
    const un = events.modelsChanged(refresh);
    return () => {
      un.then((f) => f());
    };
  }, [refresh]);
  useEffect(() => onBusy(switching !== null), [switching, onBusy]);

  useEffect(() => {
    const un = events.download((e) => {
      setTransfers((all) => {
        const t = all[e.modelId];
        if (!t) return all;
        let next: Transfer = t;
        if (e.phase === "probe") next = { ...t, note: "正在测试下载源…" };
        if (e.phase === "download")
          next = {
            ...t,
            done: e.done,
            total: e.total,
            verifying: false,
            note: `${speed(e.speedBps)} · ${e.source}`,
          };
        if (e.phase === "retry") next = { ...t, note: `${e.message}，${e.waitSecs} 秒后重试` };
        if (e.phase === "verify")
          next = { ...t, done: e.done, total: e.total, verifying: true, note: "正在校验文件完整性…" };
        return { ...all, [e.modelId]: next };
      });
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (!settings || !dir) return null;

  const download = async (m: ModelView) => {
    setMessage(null);
    setTransfers((all) => ({
      ...all,
      [m.id]: {
        done: m.downloadedBytes,
        total: m.size,
        note: "正在测试下载源…",
        verifying: false,
        pausing: false,
      },
    }));
    try {
      await api.download(m.id, dir);
      setMessage({ tone: "good", text: `${m.name} 下载完成，现在可以切换使用` });
    } catch (e) {
      const err = e as DownloadError;
      if (err?.kind !== "paused" && err?.kind !== "cancelled") {
        setMessage({ tone: "error", text: `${m.name} 下载失败：${errorText(e)}` });
      }
    } finally {
      setTransfers(({ [m.id]: _, ...rest }) => rest);
      refresh();
    }
  };

  const pause = (id: string) => {
    setTransfers((all) => (all[id] ? { ...all, [id]: { ...all[id], pausing: true } } : all));
    api.downloadControl(id, "pause");
  };

  const switchTo = async (m: ModelView) => {
    setMessage(null);
    setSwitching(m.id);
    try {
      const out: LaunchOutcome = await api.modelSwitch(m.id);
      setMessage(
        out.fallbackReason
          ? {
              tone: "warn",
              text: `已切换到 ${m.name}，但 GPU 启动失败，已自动改用 CPU。原因：${out.fallbackReason}`,
            }
          : {
              tone: "good",
              text: `已切换到 ${m.name}（${backendName(out.backend)}，耗时 ${(out.loadMs / 1000).toFixed(1)} 秒）· 自检“${SELF_TEST_TEXT}” → “${out.sample}”`,
            },
      );
    } catch (e) {
      setMessage({ tone: "error", text: `切换到 ${m.name} 失败：${errorText(e)}` });
    } finally {
      setSwitching(null);
      refresh();
    }
  };

  const remove = async (m: ModelView) => {
    setConfirmDelete(null);
    setMessage(null);
    try {
      await api.modelDelete(m.id);
    } catch (e) {
      setMessage({ tone: "error", text: errorText(e) });
    }
    refresh();
  };

  const locked = switching !== null;

  return (
    <div className="flex flex-col gap-2.5">
      {models.map((m) => {
        const current = settings.deployed && settings.modelId === m.id;
        const t = transfers[m.id];
        const partial = !m.installed && m.downloadedBytes > 0;
        const pct = m.size ? Math.floor((m.downloadedBytes / m.size) * 100) : 0;
        const livePct = t && t.total ? Math.min(100, Math.round((t.done / t.total) * 100)) : 0;
        const { tier, title } = splitTierName(m.name);

        return (
          <div
            key={m.id}
            className={`flex flex-col gap-3 rounded-xl border p-4 transition-colors ${
              current
                ? "border-[var(--accent)]/45 bg-[var(--bg-surface)] shadow-[var(--shadow-card)]"
                : "border-[var(--border)] bg-[var(--bg-surface)] shadow-[var(--shadow-card)]"
            }`}
          >
            <div className="flex flex-wrap items-start gap-x-4 gap-y-3">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-[var(--ink)]">{title}</span>
                  {tier && <Badge tone="neutral">{tier}</Badge>}
                  {current ? (
                    <Badge tone="accent">
                      <Sparkles className="size-3" />
                      当前使用
                    </Badge>
                  ) : m.installed ? (
                    <Badge tone="good">已就绪</Badge>
                  ) : partial && !t ? (
                    <Badge tone="warn">已下载 {pct}%</Badge>
                  ) : null}
                </div>

                <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-[var(--ink-secondary)] tabular-nums">
                  <span className="inline-flex items-center gap-1">
                    <HardDrive className="size-3 text-[var(--ink-muted)]" />
                    {gb(m.size)}
                  </span>
                  <span>建议显存 ≥ {m.recommendVramGB} GB</span>
                  <span>内存 ≥ {m.minRamGB} GB</span>
                </div>

                {current && <StatusChip className="mt-2" />}
              </div>

              <div className="flex shrink-0 items-center gap-1.5">
                {t ? (
                  <>
                    <Button
                      size="sm"
                      icon={<Pause className="size-3.5" />}
                      disabled={t.pausing || t.verifying}
                      onClick={() => pause(m.id)}
                    >
                      暂停
                    </Button>
                    <Button
                      size="sm"
                      variant="subtle"
                      icon={<X className="size-3.5" />}
                      onClick={() => api.downloadControl(m.id, "cancel")}
                    >
                      取消
                    </Button>
                  </>
                ) : switching === m.id ? (
                  <span className="inline-flex items-center gap-2 rounded-lg bg-[var(--accent-soft)] px-2.5 py-1.5 text-xs font-medium text-[var(--accent-text)]">
                    <Spinner className="size-3.5" />
                    正在加载并自检…
                  </span>
                ) : confirmDelete === m.id ? (
                  <div className="flex items-center gap-1.5">
                    <span className="text-xs text-[var(--ink-secondary)]">删除后需重新下载</span>
                    <Button size="sm" variant="danger" onClick={() => remove(m)}>
                      确认删除
                    </Button>
                    <Button size="sm" variant="subtle" onClick={() => setConfirmDelete(null)}>
                      取消
                    </Button>
                  </div>
                ) : current ? null : (
                  <>
                    {m.installed ? (
                      <Button size="sm" variant="primary" disabled={locked} onClick={() => switchTo(m)}>
                        切换使用
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="secondary"
                        icon={partial ? <Play className="size-3.5" /> : <Download className="size-3.5" />}
                        disabled={locked}
                        onClick={() => download(m)}
                      >
                        {partial ? "继续下载" : "下载模型"}
                      </Button>
                    )}
                    {(m.installed || partial) && (
                      <Button
                        size="sm"
                        variant="subtle"
                        icon={<Trash2 className="size-3.5" />}
                        disabled={locked}
                        onClick={() => setConfirmDelete(m.id)}
                        title="删除模型文件"
                      >
                        删除
                      </Button>
                    )}
                  </>
                )}
              </div>
            </div>

            {t && (
              <div className="flex flex-col gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface-alt)] px-3 py-2.5">
                <div className="flex items-center justify-between text-xs tabular-nums">
                  <span className="font-medium text-[var(--ink)]">{livePct}%</span>
                  <span className="text-[var(--ink-secondary)]">{t.pausing ? "正在暂停…" : t.note}</span>
                </div>
                <Progress value={t.total ? t.done / t.total : 0} />
                <div className="text-[11px] text-[var(--ink-muted)] tabular-nums">
                  {gb(t.done)} / {gb(t.total)}
                </div>
              </div>
            )}
          </div>
        );
      })}

      {message && (
        <div
          className={`flex items-start gap-2.5 rounded-xl border px-3.5 py-2.5 text-xs leading-relaxed ${
            message.tone === "good"
              ? "border-[var(--good)]/25 bg-[var(--good-soft)] text-[var(--good)]"
              : message.tone === "warn"
                ? "border-[var(--warn)]/25 bg-[var(--warn-soft)] text-[var(--warn)]"
                : "border-[var(--danger)]/25 bg-[var(--danger-soft)] text-[var(--danger)]"
          }`}
        >
          {message.tone === "good" ? (
            <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
          ) : (
            <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          )}
          <span className="selectable flex-1 break-all whitespace-pre-wrap">{message.text}</span>
          <button
            type="button"
            onClick={() => setMessage(null)}
            className="cursor-pointer opacity-70 hover:opacity-100"
            title="关闭"
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}
    </div>
  );
}
