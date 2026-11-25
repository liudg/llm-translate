// Unified frameless custom title bar for Main and Wizard windows.
import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

function MinimizeGlyph() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
      <path d="M0.5 5.5H9.5" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}

function MaximizeGlyph() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
      <rect x="0.5" y="0.5" width="9" height="9" rx="0.75" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}

function RestoreGlyph() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
      <rect x="0.5" y="2.5" width="7" height="7" rx="0.5" stroke="currentColor" strokeWidth="1" />
      <path
        d="M2.5 2.5V1A0.5 0.5 0 0 1 3 0.5H9A0.5 0.5 0 0 1 9.5 1V7A0.5 0.5 0 0 1 9 7.5H7.5"
        stroke="currentColor"
        strokeWidth="1"
      />
    </svg>
  );
}

function CloseGlyph() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
      <path
        d="M0.75 0.75L9.25 9.25M9.25 0.75L0.75 9.25"
        stroke="currentColor"
        strokeWidth="1.05"
        strokeLinecap="square"
      />
    </svg>
  );
}

export function TitleBar({
  left,
  center,
  right,
  maximizable = true,
  className = "",
}: {
  left?: ReactNode;
  center?: ReactNode;
  right?: ReactNode;
  maximizable?: boolean;
  className?: string;
}) {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!maximizable) {
      document.documentElement.dataset.maximized = "false";
      return;
    }
    let active = true;
    const sync = () => {
      win
        .isMaximized()
        .then((m) => {
          if (!active) return;
          setMaximized(m);
          document.documentElement.dataset.maximized = String(m);
        })
        .catch(() => {});
    };
    sync();
    const un = win.onResized(sync);
    return () => {
      active = false;
      un.then((f) => f());
    };
  }, [maximizable, win]);

  const toggleMaximize = () => {
    if (!maximizable) return;
    win.toggleMaximize().catch(() => {});
  };

  return (
    <header
      data-tauri-drag-region
      className={`flex h-[46px] shrink-0 items-center border-b border-[var(--border-subtle)] pl-3.5 select-none ${className}`}
    >
      {/* Left slot (brand + navigation) */}
      <div data-tauri-drag-region className="flex h-full shrink-0 items-center gap-2.5">
        {left}
      </div>

      {/* Draggable center space */}
      <div
        data-tauri-drag-region
        className="flex h-full min-w-6 flex-1 items-center justify-center px-2"
      >
        {center}
      </div>

      {/* Right slot (status + actions) */}
      {right && (
        <div data-tauri-drag-region className="mr-1.5 flex h-full shrink-0 items-center gap-1.5">
          {right}
        </div>
      )}

      {/* Native-proportion Windows 11 caption controls (46px uniform width) */}
      <div className="flex h-full shrink-0 items-stretch">
        <button
          type="button"
          aria-label="最小化"
          title="最小化"
          onClick={() => win.minimize()}
          className="inline-flex w-[46px] cursor-pointer items-center justify-center text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
        >
          <MinimizeGlyph />
        </button>
        {maximizable && (
          <button
            type="button"
            aria-label={maximized ? "还原" : "最大化"}
            title={maximized ? "还原" : "最大化"}
            onClick={toggleMaximize}
            className="inline-flex w-[46px] cursor-pointer items-center justify-center text-[var(--ink-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--ink)]"
          >
            {maximized ? <RestoreGlyph /> : <MaximizeGlyph />}
          </button>
        )}
        <button
          type="button"
          aria-label="关闭"
          title="关闭"
          onClick={() => win.close()}
          className="inline-flex w-[46px] cursor-pointer items-center justify-center text-[var(--ink-secondary)] transition-colors hover:bg-[#c42b1c] hover:text-white"
        >
          <CloseGlyph />
        </button>
      </div>
    </header>
  );
}
