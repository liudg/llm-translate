// Screenshot result window (DESIGN 5.6 step 6): the frozen region at its exact
// screen position with translations drawn over the original text as they
// stream in; a toolbar below (or above) it. One window, replaced by the next
// screenshot; Esc or the close button hides it.
import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Check, Copy, Eye, EyeOff, FileText, ImageIcon, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, events, type ShotParagraph, type ShotPayload } from "../../lib/api";
import { useCopy } from "../../lib/useCopy";
import { IconButton, Spinner } from "../../components/ui";
import { startInferenceSync, useInference } from "../../stores/inference";
import { drawParagraph } from "./render";

/** Toolbar strip in DIPs: a 4 px gap plus the 40 px bar (capture/mod.rs uses 44). */
const BAR_GAP = 4;
const BAR_HEIGHT = 40;

type Phase = "ocr" | "translating" | "done" | "error" | "cancelled";

interface Para extends ShotParagraph {
  result: string;
}

export default function Shot() {
  const [shot, setShot] = useState<ShotPayload | null>(null);
  const [paras, setParas] = useState<Para[]>([]);
  const [phase, setPhase] = useState<Phase>("ocr");
  const [error, setError] = useState<string | null>(null);
  const [original, setOriginal] = useState(false);
  const [done, setDone] = useState(0);
  const textCopy = useCopy();
  const sourceCopy = useCopy();
  const imageCopy = useCopy();
  const status = useInference((s) => s.status);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const baseRef = useRef<HTMLCanvasElement | null>(null);
  /** Highest shot id seen, and the id still on screen (0 once hidden). */
  const seen = useRef(0);
  const active = useRef(0);
  const translating = useRef(false);
  /** DPI scale of the monitor the shot came from. */
  const scaleRef = useRef(1);

  const draw = useCallback((list: Para[], showOriginal: boolean) => {
    const canvas = canvasRef.current;
    const base = baseRef.current;
    if (!canvas || !base) return;
    if (canvas.width !== base.width || canvas.height !== base.height) {
      canvas.width = base.width;
      canvas.height = base.height;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.drawImage(base, 0, 0);
    if (showOriginal) return;
    const scale = scaleRef.current;
    for (const p of list) {
      if (p.translatable && p.result) drawParagraph(ctx, p, p.result, scale, base.height);
    }
  }, []);

  const load = useCallback(
    async (p: ShotPayload) => {
      if (p.id <= seen.current) return;
      seen.current = p.id;
      active.current = p.id;
      const stale = () => active.current !== p.id;
      scaleRef.current = p.scale;
      setShot(p);
      setParas([]);
      setPhase("ocr");
      setError(null);
      setOriginal(false);
      setDone(0);

      let rgba: ArrayBuffer;
      try {
        rgba = await api.shotImage(p.id);
      } catch {
        return; // closed in the meantime
      }
      if (stale()) return;
      const base = document.createElement("canvas");
      base.width = p.width;
      base.height = p.height;
      base.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(rgba), p.width, p.height), 0, 0);
      baseRef.current = base;
      draw([], false);
      await api.shotShown(p.id);

      let found: ShotParagraph[];
      try {
        found = await api.shotOcr(p.id);
      } catch (e) {
        if (!stale()) {
          setError(errorText(e));
          setPhase("error");
        }
        return;
      }
      if (stale()) return;
      const list: Para[] = found.map((f) => ({ ...f, result: "" }));
      setParas(list);
      const todo = list.flatMap((f, i) => (f.translatable ? [i] : []));
      if (todo.length === 0) {
        setPhase("done");
        return;
      }
      setPhase("translating");
      translating.current = true;
      try {
        await api.translateBatch(
          todo.map((i) => list[i].text),
          null,
          (e) => {
            if (stale()) return;
            switch (e.type) {
              case "delta": {
                const at = todo[e.index];
                setParas((prev) => prev.map((q, i) => (i === at ? { ...q, result: q.result + e.text } : q)));
                break;
              }
              case "itemDone":
                setDone(e.index + 1);
                break;
              case "done":
                setPhase("done");
                break;
              case "error":
                setError(e.message);
                setPhase("error");
                break;
              case "cancelled":
                setPhase("cancelled");
                break;
            }
          },
        );
      } catch (e) {
        if (!stale()) {
          setError(errorText(e));
          setPhase("error");
        }
      }
      translating.current = false;
    },
    [draw],
  );

  useEffect(() => {
    startInferenceSync();
    api.shotCurrent().then((p) => p && load(p));
    const unNew = events.shotNew(load);
    const unHidden = events.shotHidden(() => {
      if (translating.current) api.cancelTranslate();
      translating.current = false;
      // Late events of the hidden shot are ignored from now on.
      active.current = 0;
      baseRef.current = null;
      setShot(null);
    });
    return () => {
      unNew.then((f) => f());
      unHidden.then((f) => f());
    };
  }, [load]);

  // Redraw at most once per frame while translations stream in.
  useEffect(() => {
    const id = requestAnimationFrame(() => draw(paras, original));
    return () => cancelAnimationFrame(id);
  }, [paras, original, draw, shot]);

  const translations = paras.filter((p) => p.translatable && p.result.trim()).map((p) => p.result.trim());
  const sources = paras.map((p) => p.text);
  const copyTranslation = () => {
    // Ctrl+C also lands here: never overwrite the clipboard with nothing.
    if (translations.length > 0) textCopy.copy(translations.join("\n"));
  };
  const copySource = () => sourceCopy.copy(sources.join("\n"));
  const copyImage = () => {
    const c = canvasRef.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx) return;
    const data = ctx.getImageData(0, 0, c.width, c.height);
    imageCopy.copy(() => api.shotCopyImage(new Uint8Array(data.data.buffer), c.width, c.height));
  };

  const keyRef = useRef({ copyTranslation, close: () => api.shotClose() });
  keyRef.current = { copyTranslation, close: () => api.shotClose() };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") keyRef.current.close();
      else if (e.ctrlKey && e.key.toLowerCase() === "c") keyRef.current.copyTranslation();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!shot) return null;

  const total = paras.filter((p) => p.translatable).length;
  let statusText: ReactNode;
  let busy = false;
  if (phase === "ocr") {
    busy = true;
    statusText = "正在识别文字…";
  } else if (phase === "translating") {
    busy = true;
    statusText = status.state === "loading" ? "正在加载模型…" : `正在翻译 ${done}/${total}`;
  } else if (phase === "error") {
    statusText = <span className="text-red-600 dark:text-red-400">{error}</span>;
  } else if (phase === "cancelled") {
    statusText = "已被其他翻译请求中断";
  } else if (paras.length === 0) {
    statusText = "未识别到文字";
  } else if (total === 0) {
    statusText = "没有需要翻译的文字";
  } else {
    statusText = `已翻译 ${total} 段`;
  }

  // Sizes in DIPs of the capture monitor. Dragged to a monitor with another
  // scale, Windows rescales the window and these stay proportional.
  const dpr = shot.scale;
  const imgStyle = {
    left: shot.imageX / dpr,
    top: shot.imageY / dpr,
    width: shot.width / dpr,
    height: shot.height / dpr,
  };
  const barStyle: CSSProperties =
    shot.toolbar === "below"
      ? { top: shot.height / dpr + BAR_GAP }
      : shot.toolbar === "above"
        ? { top: 0 }
        : { bottom: BAR_GAP, left: BAR_GAP, right: BAR_GAP };

  return (
    <div className="relative h-full w-full overflow-hidden">
      <canvas
        ref={canvasRef}
        className="absolute cursor-move outline outline-1 -outline-offset-1 outline-[var(--accent)]/60"
        style={imgStyle}
        onMouseDown={(e) => {
          if (e.button === 0) getCurrentWindow().startDragging();
        }}
      />
      <div
        data-tauri-drag-region
        className="absolute right-0 left-0 flex items-center gap-0.5 rounded-lg border border-black/10 bg-[#f9f9f9] px-1.5 text-sm text-[#1a1a1a] dark:border-white/10 dark:bg-[#2b2b2b] dark:text-[#f3f3f3]"
        style={{ height: BAR_HEIGHT, ...barStyle }}
      >
        <span data-tauri-drag-region className="flex min-w-0 flex-1 items-center gap-2 truncate px-1.5 text-xs">
          {busy && <Spinner className="pointer-events-none shrink-0 text-[var(--accent-text)]" />}
          <span data-tauri-drag-region className="truncate opacity-75" title={typeof statusText === "string" ? statusText : (error ?? "")}>
            {statusText}
          </span>
        </span>
        <IconButton
          title={original ? "显示译文" : "显示原图"}
          onClick={() => setOriginal((v) => !v)}
          disabled={translations.length === 0}
        >
          {original ? <EyeOff className="size-4 text-[var(--accent-text)]" /> : <Eye className="size-4" />}
        </IconButton>
        <IconButton title="复制全部译文（Ctrl+C）" onClick={copyTranslation} disabled={translations.length === 0}>
          {textCopy.copied ? <Check className="size-4" /> : <Copy className="size-4" />}
        </IconButton>
        <IconButton title="复制原文" onClick={copySource} disabled={sources.length === 0}>
          {sourceCopy.copied ? <Check className="size-4" /> : <FileText className="size-4" />}
        </IconButton>
        <IconButton title={original ? "复制原图" : "复制译图"} onClick={copyImage}>
          {imageCopy.copied ? <Check className="size-4" /> : <ImageIcon className="size-4" />}
        </IconButton>
        <IconButton title="关闭（Esc）" onClick={() => api.shotClose()}>
          <X className="size-4" />
        </IconButton>
      </div>
    </div>
  );
}
