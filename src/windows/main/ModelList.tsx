// Model management (DESIGN 5.8): every model in models.json with its download
// state. Download (pause / resume / cancel), switch (launch + self-test with the
// saved backend, previous model restored on failure) and delete.
import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, Download, Pause, Play, Trash2, X } from "lucide-react";
import { api, errorText, events, type DownloadError, type LaunchOutcome, type ModelView } from "../../lib/api";
import { backendName } from "../../lib/format";
import { Badge, Button, Progress, Spinner } from "../../components/ui";
import { StatusChip } from "../../components/status";
import { useSettings } from "../../stores/settings";

function gb(bytes: number) {
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function mbps(bps: number) {
  return `${(bps / 1024 ** 2).toFixed(1)} MB/s`;
}

/** Live state of a download started on this page. */
interface Transfer {
  done: number;
  total: number;
  /** Text under the bar: speed / source, retry notice, verifying. */
  note: string;
  verifying: boolean;
  pausing: boolean;
}

type Message = { tone: "good" | "warn" | "error"; text: string };

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
  useEffect(() => onBusy(switching !== null), [switching, onBusy]);

  useEffect(() => {
    const un = events.download((e) => {
      setTransfers((all) => {
        const t = all[e.modelId];
        if (!t) return all; // started elsewhere (wizard)
        let next: Transfer = t;
        if (e.phase === "probe") next = { ...t, note: "正在测试下载源…" };
        if (e.phase === "download")
          next = { ...t, done: e.done, total: e.total, verifying: false, note: `${mbps(e.speedBps)} · ${e.source}` };
        if (e.phase === "retry") next = { ...t, note: `${e.message}，${e.waitSecs} 秒后重试` };
        if (e.phase === "verify") next = { ...t, done: e.done, total: e.total, verifying: true, note: "正在校验文件…" };
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
      [m.id]: { done: m.downloadedBytes, total: m.size, note: "正在测试下载源…", verifying: false, pausing: false },
    }));
    try {
      await api.download(m.id, dir);
      setMessage({ tone: "good", text: `${m.name} 下载完成，可以切换使用` });
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
        out.fellBack
          ? {
              tone: "warn",
              text: `已切换到 ${m.name}，但 GPU 启动失败，已改用 CPU（速度较慢）。原因：${out.fallbackReason}`,
            }
          : {
              tone: "good",
              text: `已切换到 ${m.name} · ${backendName(out.backend)} · 加载 ${(out.loadMs / 1000).toFixed(1)} 秒；自检“${out.sampleSource}” → “${out.sample}”`,
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
    <div className="flex flex-col divide-y divide-[var(--stroke)] rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)]">
      {models.map((m) => {
        const current = settings.deployed && settings.modelId === m.id;
        const t = transfers[m.id];
        const partial = !m.installed && m.downloadedBytes > 0;
        const pct = m.size ? Math.floor((m.downloadedBytes / m.size) * 100) : 0;
        return (
          <div key={m.id} className="flex flex-col gap-2 px-4 py-3">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-sm">
                  {m.name}
                  {current ? (
                    <Badge tone="accent">使用中</Badge>
                  ) : m.installed ? (
                    <Badge tone="good">已下载</Badge>
                  ) : partial && !t ? (
                    <Badge tone="warn">已下载 {pct}%</Badge>
                  ) : null}
                </div>
                <div className="mt-0.5 text-xs opacity-65">
                  {gb(m.size)} · 建议显存 ≥ {m.recommendVramGB} GB · 内存 ≥ {m.minRamGB} GB
                </div>
                {current && <StatusChip className="mt-1" />}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {t ? (
                  <>
                    <Button
                      icon={<Pause className="size-4" />}
                      disabled={t.pausing || t.verifying}
                      onClick={() => pause(m.id)}
                    >
                      暂停
                    </Button>
                    <Button variant="subtle" icon={<X className="size-4" />} onClick={() => api.downloadControl(m.id, "cancel")}>
                      取消
                    </Button>
                  </>
                ) : switching === m.id ? (
                  <span className="flex items-center gap-2 text-xs opacity-75">
                    <Spinner />
                    正在加载并自检…
                  </span>
                ) : confirmDelete === m.id ? (
                  <>
                    <span className="text-xs opacity-75">删除后需重新下载</span>
                    <Button
                      className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-500 dark:hover:bg-red-600"
                      onClick={() => remove(m)}
                    >
                      确认删除
                    </Button>
                    <Button variant="subtle" onClick={() => setConfirmDelete(null)}>
                      取消
                    </Button>
                  </>
                ) : current ? null : (
                  <>
                    {m.installed ? (
                      <Button variant="primary" disabled={locked} onClick={() => switchTo(m)}>
                        使用
                      </Button>
                    ) : (
                      <Button
                        icon={partial ? <Play className="size-4" /> : <Download className="size-4" />}
                        disabled={locked}
                        onClick={() => download(m)}
                      >
                        {partial ? "继续下载" : "下载"}
                      </Button>
                    )}
                    {(m.installed || partial) && (
                      <Button
                        variant="subtle"
                        icon={<Trash2 className="size-4" />}
                        disabled={locked}
                        onClick={() => setConfirmDelete(m.id)}
                      >
                        删除
                      </Button>
                    )}
                  </>
                )}
              </div>
            </div>
            {t && (
              <div className="flex flex-col gap-1">
                <Progress value={t.total ? t.done / t.total : 0} />
                <div className="flex justify-between text-xs opacity-65">
                  <span>
                    {gb(t.done)} / {gb(t.total)}
                  </span>
                  <span>{t.pausing ? "正在暂停…" : t.note}</span>
                </div>
              </div>
            )}
          </div>
        );
      })}
      {message && (
        <div
          className={`flex items-start gap-2 px-4 py-2.5 text-xs ${
            message.tone === "good"
              ? "text-green-700 dark:text-green-400"
              : message.tone === "warn"
                ? "text-amber-700 dark:text-amber-400"
                : "text-red-600 dark:text-red-400"
          }`}
        >
          {message.tone === "good" ? (
            <CheckCircle2 className="mt-px size-4 shrink-0" />
          ) : (
            <AlertTriangle className="mt-px size-4 shrink-0" />
          )}
          <span className="selectable flex-1 break-all whitespace-pre-wrap">{message.text}</span>
          <button onClick={() => setMessage(null)} className="opacity-60 hover:opacity-100" title="关闭">
            <X className="size-4" />
          </button>
        </div>
      )}
    </div>
  );
}
