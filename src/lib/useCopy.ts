import { useRef, useState } from "react";

/** Copy text (or run another copy action) and flag `copied` for 1.5 s. */
export function useCopy() {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number>(undefined);
  const copy = async (what: string | (() => Promise<unknown>)) => {
    try {
      await (typeof what === "string" ? navigator.clipboard.writeText(what) : what());
    } catch (e) {
      console.error("copy failed", e);
      return;
    }
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1500);
  };
  return { copied, copy };
}
