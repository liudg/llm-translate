import { useEffect, useRef, type RefObject } from "react";

/**
 * Report the element's exact CSS height whenever it changes (or when monitor DPI /
 * Windows accessibility text size changes); flyout windows size themselves from it.
 */
export function useAutoHeight(ref: RefObject<HTMLElement | null>, onHeight: (h: number) => void) {
  const cb = useRef(onHeight);
  cb.current = onHeight;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    let last = 0;
    let rafId = 0;
    const report = (force = false) => {
      const h = el.getBoundingClientRect().height;
      if (h > 0 && (force || Math.abs(h - last) > 0.2)) {
        last = h;
        cb.current(h);
      }
    };

    const ro = new ResizeObserver(() => report(false));
    ro.observe(el);

    let mq: MediaQueryList | null = null;
    let onDprChange: (() => void) | null = null;

    const watchDpr = () => {
      if (mq && onDprChange) mq.removeEventListener("change", onDprChange);
      const dpr = window.devicePixelRatio || 1;
      mq = window.matchMedia(`(resolution: ${dpr}dppx)`);
      onDprChange = () => {
        watchDpr();
        window.cancelAnimationFrame(rafId);
        rafId = window.requestAnimationFrame(() => report(true));
      };
      mq.addEventListener("change", onDprChange, { once: true });
    };

    watchDpr();
    report(false);

    return () => {
      window.cancelAnimationFrame(rafId);
      ro.disconnect();
      if (mq && onDprChange) mq.removeEventListener("change", onDprChange);
    };
  }, [ref]);
}
