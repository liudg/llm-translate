// Shortcut recorder (DESIGN 5.7). Click to record: all global shortcuts are
// paused meanwhile so pressing a bound combination records it instead of
// firing it. Esc cancels, Backspace / Delete unbinds. The backend registers the
// new combination before saving; if another program holds it the old binding
// stays and the error is shown.
import { useEffect, useRef, useState } from "react";
import { api, errorText, type HotkeyId, type HotkeyInfo } from "../../lib/api";
import { Badge } from "../../components/ui";

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

function Keys({ keys, dim = false }: { keys: string; dim?: boolean }) {
  return (
    <span className={`flex items-center gap-1 ${dim ? "opacity-60" : ""}`}>
      {keys.split("+").map((k, i) => (
        <kbd
          key={i}
          className="min-w-6 rounded border border-[var(--stroke)] bg-[var(--surface)] px-1.5 py-0.5 text-center text-xs"
        >
          {k}
        </kbd>
      ))}
    </span>
  );
}

export function HotkeyRecorder({ id, info }: { id: HotkeyId; info: HotkeyInfo | undefined }) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const recordingRef = useRef(false);

  const stop = () => {
    if (!recordingRef.current) return;
    recordingRef.current = false;
    setRecording(false);
    setHeld([]);
    api.hotkeyPause(false);
  };

  const start = () => {
    if (recordingRef.current || busy) return;
    recordingRef.current = true;
    setError(null);
    setHeld([]);
    setRecording(true);
    api.hotkeyPause(true);
  };

  const save = async (keys: string | null) => {
    setBusy(true);
    try {
      await api.hotkeySet(id, keys);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      stop();
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
        setError("快捷键需要包含 Ctrl、Alt 或 Win");
        return;
      }
      void save([...mods, key].join("+"));
    };
    const up = (e: KeyboardEvent) => {
      e.preventDefault();
      setHeld(modifiers(e));
    };
    // Switching away cancels: the other app would get the keys anyway.
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

  // Leaving the page (component unmount) must not leave shortcuts paused.
  useEffect(() => () => stop(), []);

  const keys = info?.keys ?? null;
  return (
    <div className="flex flex-col items-end gap-1">
      <div className="flex items-center gap-2">
        {!recording && keys && info && (info.registered ? <Badge tone="good">已生效</Badge> : <Badge tone="warn">未生效</Badge>)}
        <button
          onClick={recording ? stop : start}
          title={recording ? "Esc 取消 · Backspace 清除" : "点击后按下新的快捷键"}
          className={`flex h-8 min-w-36 items-center justify-center rounded-md border px-2 text-sm transition-colors ${
            recording
              ? "border-[var(--accent)] bg-[var(--accent)]/[0.08]"
              : "border-[var(--stroke)] bg-[var(--surface-strong)] hover:border-[var(--accent)]"
          }`}
        >
          {recording ? (
            held.length > 0 ? (
              <Keys keys={[...held, "…"].join("+")} dim />
            ) : (
              <span className="text-xs text-[var(--accent-text)]">请按下快捷键…</span>
            )
          ) : keys ? (
            <Keys keys={keys} />
          ) : (
            <span className="text-xs opacity-60">未绑定</span>
          )}
        </button>
      </div>
      {recording && <span className="text-[11px] opacity-60">Esc 取消 · Backspace 清除绑定</span>}
      {error && <span className="max-w-72 text-right text-xs text-red-600 dark:text-red-400">{error}</span>}
    </div>
  );
}
