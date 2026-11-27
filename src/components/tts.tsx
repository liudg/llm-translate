// Shared editorial read-aloud button with loading spinner & active soundwave/stop state.
import type { MouseEvent } from "react";
import { Square, Volume2 } from "lucide-react";
import type { TtsPhase } from "../lib/tts";
import { Spinner } from "./ui";

function SoundWave({ className = "size-3.5" }: { className?: string }) {
  return (
    <span aria-hidden="true" className={`inline-flex items-end justify-center gap-[2px] ${className}`}>
      <span className="tts-bar tts-bar-1 h-full w-[2px] rounded-full bg-current" />
      <span className="tts-bar tts-bar-2 h-full w-[2px] rounded-full bg-current" />
      <span className="tts-bar tts-bar-3 h-full w-[2px] rounded-full bg-current" />
    </span>
  );
}

export function SpeakButton({
  phase,
  disabled = false,
  title = "朗读",
  stopTitle = "停止朗读",
  label,
  activeLabel = "停止",
  size = "sm",
  onClick,
  className = "",
}: {
  phase: TtsPhase;
  disabled?: boolean;
  title?: string;
  stopTitle?: string;
  label?: string;
  activeLabel?: string;
  size?: "xs" | "sm" | "md";
  onClick: (e: MouseEvent<HTMLButtonElement>) => void;
  className?: string;
}) {
  const isLoading = phase === "loading";
  const isPlaying = phase === "playing";
  const active = isLoading || isPlaying;

  const iconSize = size === "xs" ? "size-3" : "size-3.5";
  const dim = label
    ? size === "xs"
      ? "h-6 rounded-md px-2 text-caption gap-1"
      : size === "md"
        ? "h-8 rounded-lg px-3 text-body gap-1.5"
        : "h-7 rounded-md px-2.5 text-caption gap-1.5"
    : size === "xs"
      ? "size-6 rounded-md"
      : size === "md"
        ? "size-8 rounded-lg"
        : "size-7 rounded-md";

  return (
    <button
      type="button"
      disabled={disabled && !active}
      onClick={onClick}
      title={active ? stopTitle : title}
      aria-label={active ? stopTitle : title}
      aria-pressed={active}
      className={`group inline-flex shrink-0 cursor-pointer items-center justify-center font-medium whitespace-nowrap transition-colors duration-150 disabled:pointer-events-none disabled:opacity-35 ${dim} ${
        active
          ? "bg-[var(--accent-soft)] text-[var(--accent-text)] hover:bg-[var(--accent)]/18"
          : "text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)] active:bg-[var(--bg-active)]"
      } ${className}`}
    >
      {isLoading ? (
        <Spinner className={iconSize} />
      ) : isPlaying ? (
        <>
          <SoundWave className={`${iconSize} group-hover:hidden`} />
          <Square className={`hidden ${size === "xs" ? "size-2.5" : "size-3"} fill-current group-hover:block`} />
        </>
      ) : (
        <Volume2 className={iconSize} />
      )}
      {label && <span>{isLoading ? "合成中…" : isPlaying ? activeLabel : label}</span>}
    </button>
  );
}
