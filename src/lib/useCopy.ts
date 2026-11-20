import { useRef, useState } from "react";

/** Copy text and flag `copied` for 1.5 s. */
export function useCopy() {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number>(undefined);
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
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
