// Shortcut recorder (DESIGN 5.7): tactile keycap recorder with live held-key preview.
import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { api, errorText, type HotkeyId, type HotkeyInfo } from "../../lib/api";
import { Badge, Kbd } from "../../components/ui";

const NAMED: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  NumpadAdd: "NumAdd",
  NumpadSubtract: "NumSubtract",
  NumpadMultiply: "NumMultiply",
  NumpadDivide: "NumDivide",
  NumpadDecimal: "NumDecimal",
  Pause: "Pause",
  PrintScreen: "PrintScreen",
};

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

/** Key name for `KeyboardEvent.code`, in the form the backend parses. */
function keyName(code: string): string | null {
  let m = /^Key([A-Z])$/.exec(code);
  if (m) return m[1];
  m = /^Digit(\d)$/.exec(code);
  if (m) return m[1];
  m = /^Numpad(\d)$/.exec(code);
  if (m) return `Num${m[1]}`;
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  return NAMED[code] ?? null;
}

function modifiers(e: KeyboardEvent): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Win");
  return mods;
}

/** The recorder that is recording; starting another one ends it first. */
let active: { stop: (resume?: boolean) => void } | null = null;

/**
 * Pause / resume / set run strictly in the order the page issued them so a
 * resume never overtakes its pause.
 */
let queue: Promise<unknown> = Promise.resolve();
function inOrder<T>(f: () => Promise<T>): Promise<T> {
  const next = queue.then(f);
  queue = next.catch(() => {});
  return next;
}
const setPaused = (paused: boolean) => void inOrder(() => api.hotkeyPause(paused)).catch(() => {});

export function HotkeyRecorder({
  id,
  info,
  visible,
}: {
  id: HotkeyId;
  info: HotkeyInfo | undefined;
  /** The settings page is shown; it stays mounted while hidden. */
  visible: boolean;
}) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const recordingRef = useRef(false);
  const self = useRef({ stop: (_resume?: boolean) => {} });

  const stop = (resume = true) => {
    if (!recordingRef.current) return;
    recordingRef.current = false;
    if (active === self.current) active = null;
    setRecording(false);
    setHeld([]);
    if (resume) setPaused(false);
  };
  self.current.stop = stop;

  const start = () => {
    if (recordingRef.current || busy) return;
    const handover = active !== null && active !== self.current;
    if (handover) active!.stop(false);
    active = self.current;
    recordingRef.current = true;
    setError(null);
    setHeld([]);
    setRecording(true);
    if (!handover) setPaused(true);
  };

  const save = async (keys: string | null) => {
    setBusy(true);
    setError(null);
    try {
      await inOrder(() => api.hotkeySet(id, keys));
      setError(null);
      stop();
    } catch (e) {
      setError(errorText(e));
      setHeld([]);
    } finally {
      setBusy(false);
    }
  };

  const clearDirect = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await inOrder(() => api.hotkeySet(id, null));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!recording) return;
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat || busy) return;
      const mods = modifiers(e);
      if (MODIFIER_CODES.has(e.code)) {
        setHeld(mods);
        return;
      }
      if (mods.length === 0 && e.code === "Escape") {
        setError(null);
        return stop();
      }
      if (mods.length === 0 && (e.code === "Backspace" || e.code === "Delete")) return void save(null);
      const key = keyName(e.code);
      if (!key) {
        setError("不支持这个按键");
        return;
      }
      if (!mods.some((m) => m !== "Shift")) {
        setError("快捷键需要包含 Ctrl 或 Alt");
        return;
      }
      void save([...mods, key].join("+"));
    };
    const up = (e: KeyboardEvent) => {
      e.preventDefault();
      setHeld(modifiers(e));
    };
    const blur = () => stop();
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", blur);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording, busy]);

  useEffect(() => {
    if (!visible) stop();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);
  useEffect(() => () => stop(), []);

  const keys = info?.keys ?? null;

  return (
    <div className="flex flex-col items-end gap-1.5">
      <div className="flex items-center gap-2">
        {!recording &&
          keys &&
          info &&
          (info.registered ? <Badge tone="good">已生效</Badge> : <Badge tone="warn">未生效</Badge>)}

        <div className="group/rec relative flex items-center">
          <button
            type="button"
            disabled={busy}
            onClick={recording ? () => stop() : start}
            title={recording ? "Esc 取消 · Backspace 清除" : "点击后按下新的快捷键"}
            className={`flex h-9 min-w-[146px] cursor-pointer items-center justify-center rounded-lg border px-3 text-xs transition-all ${
              recording
                ? "border-[var(--accent)] bg-[var(--accent-soft)] ring-2 ring-[var(--accent-ring)]"
                : "border-[var(--border)] bg-[var(--bg-surface-alt)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)]"
            }`}
          >
            {recording ? (
              held.length > 0 ? (
                <Kbd keys={[...held, "…"].join("+")} dim />
              ) : (
                <span className="inline-flex items-center gap-1.5 font-medium text-[var(--accent-text)]">
                  <span className="size-1.5 animate-pulse rounded-full bg-[var(--accent)]" />
                  请按下快捷键…
                </span>
              )
            ) : keys ? (
              <Kbd keys={keys} />
            ) : (
              <span className="text-[var(--ink-muted)]">点击录制快捷键</span>
            )}
          </button>

          {/* Fixed right slot prevents horizontal layout shift when toggling recording mode */}
          <div className="ml-1 flex size-7 shrink-0 items-center justify-center">
            {!recording && keys && (
              <button
                type="button"
                disabled={busy}
                onClick={clearDirect}
                title="解除快捷键绑定"
                className="inline-flex size-7 cursor-pointer items-center justify-center rounded-md text-[var(--ink-muted)] opacity-0 transition-opacity group-hover/rec:opacity-100 hover:bg-[var(--bg-hover)] hover:text-[var(--ink)] focus:opacity-100"
              >
                <X className="size-3.5" />
              </button>
            )}
          </div>
        </div>
      </div>

      {recording && (
        <span className="text-[11px] text-[var(--ink-muted)]">
          Esc 取消 · Backspace 清除
        </span>
      )}
      {error && <span className="max-w-72 text-right text-xs text-[var(--danger)]">{error}</span>}
    </div>
  );
}
