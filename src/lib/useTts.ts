// React hook binding to the window-local singleton TTS player.
import { useCallback, useSyncExternalStore } from "react";
import { ttsPlayer, type SpeakOptions, type TtsPhase, type TtsSnapshot } from "./tts";

export function useTts(): TtsSnapshot & {
  play: (key: string, text: string, opts?: SpeakOptions) => void;
  toggle: (key: string, text: string, opts?: SpeakOptions) => void;
  stop: () => void;
  stopLocal: () => void;
  stopIfKeyPrefix: (prefix: string) => void;
  phaseOf: (key: string) => TtsPhase;
} {
  const snap = useSyncExternalStore(ttsPlayer.subscribe, ttsPlayer.getSnapshot);

  const play = useCallback((key: string, text: string, opts?: SpeakOptions) => {
    void ttsPlayer.play(key, text, opts);
  }, []);

  const toggle = useCallback((key: string, text: string, opts?: SpeakOptions) => {
    ttsPlayer.toggle(key, text, opts);
  }, []);

  const stop = useCallback(() => ttsPlayer.stop(), []);
  const stopLocal = useCallback(() => ttsPlayer.stopLocal(), []);
  const stopIfKeyPrefix = useCallback((prefix: string) => ttsPlayer.stopIfKeyPrefix(prefix), []);
  const phaseOf = useCallback(
    (key: string): TtsPhase => (snap.activeKey === key ? snap.phase : "idle"),
    [snap.activeKey, snap.phase],
  );

  return { ...snap, play, toggle, stop, stopLocal, stopIfKeyPrefix, phaseOf };
}
