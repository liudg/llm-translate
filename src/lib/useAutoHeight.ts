import { useEffect, useRef, type RefObject } from "react";

/** Report the element's height (rounded up) whenever it changes; flyout windows size themselves with it. */
export function useAutoHeight(ref: RefObject<HTMLElement | null>, onHeight: (h: number) => void) {
  const cb = useRef(onHeight);
  cb.current = onHeight;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h !== last) {
        last = h;
        cb.current(h);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
}
