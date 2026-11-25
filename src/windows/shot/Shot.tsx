// Screenshot result window (DESIGN 5.6): exact 1:1 frozen region with in-place
// streamed translation overlay + floating precision editorial HUD toolbar.
// Hold Space to peek original screenshot; Ctrl+C copies translation; Esc closes.
import { useCallback, useEffect, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { Check, Copy, FileText, GripVertical, ImageIcon, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, events, type ShotParagraph, type ShotPayload } from "../../lib/api";
import { useCopy } from "../../lib/useCopy";
import { useTts } from "../../lib/useTts";
import { IconButton, Spinner } from "../../components/ui";
import { SpeakButton } from "../../components/tts";
import { startInferenceSync, useInference } from "../../stores/inference";
import { drawParagraph } from "./render";

/** Toolbar strip in DIPs: 4 px gap + 40 px bar (matches TOOLBAR_HEIGHT = 44 in capture/mod.rs). */
const BAR_GAP = 4;
const BAR_HEIGHT = 40;
const BAR_MIN_WIDTH = 392;
const BAR_MAX_WIDTH = 552;

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
  const [peeking, setPeeking] = useState(false);
  const [done, setDone] = useState(0);

  const textCopy = useCopy();
  const sourceCopy = useCopy();
  const imageCopy = useCopy();
  const tts = useTts();
  const { stop: ttsStop, stopLocal: ttsStopLocal } = tts;
  const status = useInference((s) => s.status);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const baseRef = useRef<HTMLCanvasElement | null>(null);
  const seen = useRef(0);
  const active = useRef(0);
  const translating = useRef(false);
  const peekingRef = useRef(false);
  const scaleRef = useRef(1);

  const showOriginal = original !== peeking;

  const draw = useCallback((list: Para[], orig: boolean) => {
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
    if (orig) return;
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
      ttsStop();
      const stale = () => active.current !== p.id;
      scaleRef.current = p.scale;
      setShot(p);
      setParas([]);
      setPhase("ocr");
      setError(null);
      setOriginal(false);
      peekingRef.current = false;
      setPeeking(false);
      setDone(0);

      let rgba: ArrayBuffer;
      try {
        rgba = await api.shotImage(p.id);
      } catch {
        return;
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
    [draw, ttsStop],
  );

  useEffect(() => {
    startInferenceSync();
    api.shotCurrent().then((p) => p && load(p));
    const unNew = events.shotNew(load);
    const unHidden = events.shotHidden(() => {
      ttsStopLocal();
      if (translating.current) api.cancelTranslate();
      translating.current = false;
      active.current = 0;
      baseRef.current = null;
      peekingRef.current = false;
      setPeeking(false);
      setShot(null);
    });
    return () => {
      unNew.then((f) => f());
      unHidden.then((f) => f());
    };
  }, [load, ttsStopLocal]);

  useEffect(() => {
    const id = requestAnimationFrame(() => draw(paras, showOriginal));
    return () => cancelAnimationFrame(id);
  }, [paras, showOriginal, draw, shot]);

  const translations = paras.filter((p) => p.translatable && p.result.trim()).map((p) => p.result.trim());
  const sources = paras.map((p) => p.text);

  const copyTranslation = () => {
    if (translations.length === 0) return false;
    textCopy.copy(translations.join("\n"));
    return true;
  };
  const copySource = () => {
    if (sources.length > 0) sourceCopy.copy(sources.join("\n"));
  };
  const copyImage = () => {
    const c = canvasRef.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx) return;
    const data = ctx.getImageData(0, 0, c.width, c.height);
    imageCopy.copy(() => api.shotCopyImage(new Uint8Array(data.data.buffer), c.width, c.height));
  };

  const blurOnMouseClick = (e: MouseEvent<HTMLButtonElement>) => {
    if (e.detail > 0) e.currentTarget.blur();
  };

  const keyRef = useRef({ copyTranslation, copyImage, close: () => { ttsStop(); api.shotClose(); } });
  keyRef.current = { copyTranslation, copyImage, close: () => { ttsStop(); api.shotClose(); } };

  useEffect(() => {
    const onDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        keyRef.current.close();
      } else if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "c") {
        e.preventDefault();
        keyRef.current.copyImage();
      } else if (e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "c") {
        if (keyRef.current.copyTranslation()) {
          e.preventDefault();
        }
      } else if (e.code === "Space" && !e.ctrlKey && !e.altKey && !e.metaKey) {
        if (document.activeElement instanceof HTMLButtonElement) return;
        e.preventDefault();
        if (!e.repeat) {
          peekingRef.current = true;
          setPeeking(true);
        }
      }
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        if (!peekingRef.current && document.activeElement instanceof HTMLButtonElement) return;
        e.preventDefault();
        peekingRef.current = false;
        setPeeking(false);
      }
    };
    const onBlur = () => {
      peekingRef.current = false;
      setPeeking(false);
    };
    window.addEventListener("keydown", onDown, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  if (!shot) return null;

  const total = paras.filter((p) => p.translatable).length;
  let statusText: ReactNode;
  let busy = false;
  let progressRatio = 0;

  if (tts.error) {
    statusText = <span className="text-[var(--danger)]">{tts.error}</span>;
  } else if (phase === "ocr") {
    busy = true;
    statusText = "正在识别文字…";
  } else if (phase === "translating") {
    busy = true;
    progressRatio = total > 0 ? done / total : 0;
    statusText = status.state === "loading" ? "正在加载模型…" : `翻译中 ${done}/${total}`;
  } else if (phase === "error") {
    statusText = <span className="text-[var(--danger)]">{error}</span>;
  } else if (phase === "cancelled") {
    statusText = "已被其他请求中断";
  } else if (paras.length === 0) {
    statusText = "未识别到文字";
  } else if (total === 0) {
    statusText = "无须翻译的文字";
  } else {
    statusText = `已译 ${total} 段`;
  }

  const dpr = shot.cssScale;
  const imgLeft = shot.imageX / dpr;
  const imgWidth = shot.width / dpr;
  const imgHeight = shot.height / dpr;
  const imgStyle = {
    left: imgLeft,
    top: shot.imageY / dpr,
    width: imgWidth,
    height: imgHeight,
  };
  const winWidth = Math.max(imgWidth, BAR_MIN_WIDTH);
  const inside = shot.toolbar === "inside";
  const inset = inside ? BAR_GAP : 0;
  const availWidth = Math.max(0, winWidth - inset * 2);
  const minBarWidth = Math.min(BAR_MIN_WIDTH, availWidth);
  const rightEdge = Math.min(winWidth - inset, Math.max(inset + minBarWidth, imgLeft + imgWidth - inset));
  const rightOffset = Math.max(inset, winWidth - rightEdge);
  const barStyle: CSSProperties = {
    height: BAR_HEIGHT,
    right: rightOffset,
    minWidth: inside ? `min(${BAR_MIN_WIDTH}px, calc(100% - ${BAR_GAP * 2}px))` : `min(${BAR_MIN_WIDTH}px, 100%)`,
    maxWidth:
      rightOffset > inset
        ? Math.min(BAR_MAX_WIDTH, rightEdge - inset)
        : inside
          ? `min(${BAR_MAX_WIDTH}px, calc(100% - ${BAR_GAP * 2}px))`
          : `min(${BAR_MAX_WIDTH}px, 100%)`,
    ...(shot.toolbar === "below"
      ? { top: imgHeight + BAR_GAP }
      : shot.toolbar === "above"
        ? { top: 0 }
        : { bottom: BAR_GAP }),
  };

  const hasTranslations = translations.length > 0;
  const speakText = showOriginal ? sources.join("\n") : translations.join("\n");
  const speakKey = showOriginal ? "shot:source" : "shot:result";
  const canSpeak = showOriginal ? sources.length > 0 : hasTranslations && !busy;

  return (
    <div className="relative h-full w-full overflow-hidden">
      <canvas
        ref={canvasRef}
        className="absolute cursor-move outline outline-1 -outline-offset-1 outline-[var(--accent)]/75"
        style={imgStyle}
        onMouseDown={(e) => {
          if (e.button === 0) getCurrentWindow().startDragging();
        }}
      />

      {/* Floating Precision Editorial HUD Bar */}
      <div
        data-tauri-drag-region
        className="absolute flex w-max min-w-[392px] max-w-[552px] items-center gap-1 overflow-hidden rounded-[10px] border border-[var(--border-strong)] bg-[var(--bg-surface)]/96 px-1.5 text-xs text-[var(--ink)] shadow-[0_8px_22px_rgb(0_0_0/0.22)]"
        style={barStyle}
      >
        {/* Subtle drag handle & live status */}
        <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center gap-1.5 truncate pr-1 pl-0.5">
          <GripVertical className="pointer-events-none size-3.5 shrink-0 text-[var(--ink-muted)] opacity-70" />
          {busy ? (
            <Spinner className="pointer-events-none size-3.5 shrink-0 text-[var(--accent)]" />
          ) : (
            <span
              className={`pointer-events-none size-1.5 shrink-0 rounded-full ${
                phase === "error"
                  ? "bg-[var(--danger)]"
                  : phase === "done" && total > 0
                    ? "bg-[var(--good)]"
                    : "bg-[var(--ink-muted)]"
              }`}
            />
          )}
          <span
            data-tauri-drag-region
            className="truncate text-[12px] font-medium text-[var(--ink-secondary)] tabular-nums"
            title={typeof statusText === "string" ? statusText : (tts.error ?? error ?? "")}
          >
            {statusText}
          </span>
        </div>

        {/* Tactile 译文 / 原图 segmented toggle (also supports holding Space) */}
        <div
          title="切换显示译文或原图（按住空格键可临时透视对比）"
          className={`inline-flex shrink-0 items-center rounded-[7px] border border-[var(--border-subtle)] bg-[var(--bg-recessed)] p-[2px] ${
            !hasTranslations ? "opacity-45" : ""
          }`}
        >
          <button
            type="button"
            disabled={!hasTranslations}
            onClick={(e) => {
              blurOnMouseClick(e);
              setOriginal(false);
            }}
            className={`h-6 cursor-pointer rounded-[5px] px-2 text-[11px] font-medium transition-colors disabled:cursor-not-allowed ${
              !showOriginal
                ? "bg-[var(--bg-elevated)] text-[var(--ink)] shadow-[var(--shadow-xs)]"
                : "text-[var(--ink-secondary)] hover:text-[var(--ink)]"
            }`}
          >
            译文
          </button>
          <button
            type="button"
            disabled={!hasTranslations}
            onClick={(e) => {
              blurOnMouseClick(e);
              setOriginal(true);
            }}
            className={`h-6 cursor-pointer rounded-[5px] px-2 text-[11px] font-medium transition-colors disabled:cursor-not-allowed ${
              showOriginal
                ? "bg-[var(--bg-elevated)] text-[var(--accent-text)] shadow-[var(--shadow-xs)]"
                : "text-[var(--ink-secondary)] hover:text-[var(--ink)]"
            }`}
          >
            原图
          </button>
        </div>

        <span className="mx-0.5 h-4 w-px shrink-0 bg-[var(--border)]" />

        {/* Smart read-aloud: reads translation when viewing translation, source when viewing original */}
        <SpeakButton
          size="sm"
          phase={tts.phase}
          disabled={!canSpeak}
          title={showOriginal ? "朗读识别原文" : "朗读全部译文"}
          onClick={(e) => {
            blurOnMouseClick(e);
            if (tts.phase !== "idle") {
              ttsStop();
            } else if (speakText.trim()) {
              tts.play(speakKey, speakText);
            }
          }}
        />

        {/* Grouped copy & close actions */}
        <IconButton
          size="sm"
          title={textCopy.copied ? "已复制全部译文" : "复制全部译文（Ctrl+C）"}
          onClick={(e) => {
            blurOnMouseClick(e);
            copyTranslation();
          }}
          disabled={!hasTranslations}
        >
          {textCopy.copied ? <Check className="size-3.5 text-[var(--good)]" /> : <Copy className="size-3.5" />}
        </IconButton>

        <IconButton
          size="sm"
          title={sourceCopy.copied ? "已复制原文" : "复制识别原文"}
          onClick={(e) => {
            blurOnMouseClick(e);
            copySource();
          }}
          disabled={sources.length === 0}
        >
          {sourceCopy.copied ? <Check className="size-3.5 text-[var(--good)]" /> : <FileText className="size-3.5" />}
        </IconButton>

        <IconButton
          size="sm"
          title={imageCopy.copied ? "已复制图片" : `${showOriginal ? "复制原图" : "复制译图"}（Ctrl+Shift+C）`}
          onClick={(e) => {
            blurOnMouseClick(e);
            copyImage();
          }}
        >
          {imageCopy.copied ? <Check className="size-3.5 text-[var(--good)]" /> : <ImageIcon className="size-3.5" />}
        </IconButton>

        <IconButton
          size="sm"
          title="关闭（Esc）"
          onClick={(e) => {
            blurOnMouseClick(e);
            ttsStop();
            api.shotClose();
          }}
        >
          <X className="size-3.5" />
        </IconButton>

        {/* Precision bottom progress beam while OCR / translating */}
        {busy && (
          <div className="pointer-events-none absolute right-0 bottom-0 left-0 h-[2px] bg-[var(--accent-soft)]">
            {phase === "ocr" || total === 0 ? (
              <div className="progress-indeterminate h-full w-1/3 bg-[var(--accent)]" />
            ) : (
              <div
                className="h-full bg-[var(--accent)] transition-[width] duration-200"
                style={{ width: `${Math.min(100, Math.round(progressRatio * 100))}%` }}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}
