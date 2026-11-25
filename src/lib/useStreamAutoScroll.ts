import { useEffect, useRef, type RefObject } from "react";

const BOTTOM_THRESHOLD_PX = 36;

/**
 * Keep a scrollable streaming container pinned to the bottom while `active` is true,
 * pausing immediately when the user scrolls up (> 36px from bottom) and resuming when
 * they return near the bottom or when `dep` resets to empty.
 */
export function useStreamAutoScroll(
  ref: RefObject<HTMLElement | null>,
  active: boolean,
  dep: string,
) {
  const stickToBottom = useRef(true);
  const hasDep = dep.length > 0;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onScroll = () => {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      stickToBottom.current = dist <= BOTTOM_THRESHOLD_PX;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [ref, active, hasDep]);

  useEffect(() => {
    if (!dep) {
      stickToBottom.current = true;
      return;
    }
    if (!active || !stickToBottom.current) return;
    const el = ref.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [ref, active, dep]);
}
