// Precision Editorial UI primitives shared across all five windows.
import { useCallback, useEffect, useId, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Check, ChevronDown, Loader2 } from "lucide-react";

type Variant = "primary" | "secondary" | "subtle" | "danger";
type Size = "sm" | "md";

const variants: Record<Variant, string> = {
  primary:
    "bg-[var(--accent)] text-[var(--ink-inverse)] border border-transparent shadow-[var(--shadow-xs)] hover:bg-[var(--accent-hover)] active:scale-[0.985] disabled:opacity-45",
  secondary:
    "border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--ink)] shadow-[var(--shadow-xs)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)] active:scale-[0.985] disabled:opacity-45",
  subtle:
    "border border-transparent text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)] active:bg-[var(--bg-active)] disabled:opacity-40",
  danger:
    "bg-[var(--danger)] text-white border border-transparent shadow-[var(--shadow-xs)] hover:brightness-95 active:scale-[0.985] disabled:opacity-45",
};

const sizes: Record<Size, string> = {
  sm: "h-7 rounded-[7px] px-2.5 text-xs gap-1.5",
  md: "h-8 rounded-lg px-3 text-[13px] gap-1.5",
};

/** Signature Vermilion Seal Emblem ("译") — custom engraved geometric SVG seal. */
export function BrandSeal({
  size = "md",
  className = "",
}: {
  size?: "xs" | "sm" | "md" | "lg";
  className?: string;
}) {
  const box =
    size === "xs"
      ? "size-5 rounded-[5px]"
      : size === "sm"
        ? "size-6 rounded-[6px]"
        : size === "lg"
          ? "size-10 rounded-[10px]"
          : "size-7 rounded-[7px]";
  return (
    <span
      aria-hidden="true"
      className={`inline-flex shrink-0 items-center justify-center bg-[var(--accent)] text-[var(--ink-inverse)] shadow-[inset_0_1px_0_rgb(255_255_255/0.22),0_1px_2px_rgb(184_67_31/0.28)] select-none ${box} ${className}`}
    >
      <svg viewBox="0 0 32 32" fill="none" className="size-full">
        <rect
          x="2.5"
          y="2.5"
          width="27"
          height="27"
          rx="3.5"
          stroke="currentColor"
          strokeOpacity="0.28"
          strokeWidth="1.1"
        />
        {/* 讠 (speech radical) */}
        <path
          d="M9.4 8.0L11.4 10.4"
          stroke="currentColor"
          strokeWidth="2.1"
          strokeLinecap="round"
        />
        <path
          d="M7.2 13.8H10.8V23.2L13.4 20.8"
          stroke="currentColor"
          strokeWidth="2.05"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {/* 右部：又 + 二 + 丨 */}
        <path
          d="M15.8 8.6H22.8C21.2 11.8 18.4 14.2 14.8 15.8"
          stroke="currentColor"
          strokeWidth="2.05"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        <path
          d="M16.2 10.5C18.6 12.8 21.4 14.6 24.8 15.8"
          stroke="currentColor"
          strokeWidth="2.05"
          strokeLinecap="round"
        />
        <path
          d="M16.2 18.4H23.2M14.8 21.8H24.6M19.7 15.6V24.8"
          stroke="currentColor"
          strokeWidth="2.0"
          strokeLinecap="round"
        />
      </svg>
    </span>
  );
}

export function Button({
  variant = "secondary",
  size = "md",
  className = "",
  icon,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
}) {
  return (
    <button
      className={`inline-flex shrink-0 cursor-pointer items-center justify-center font-medium whitespace-nowrap transition-[background-color,border-color,color,transform,opacity] duration-150 disabled:cursor-not-allowed ${sizes[size]} ${variants[variant]} ${className}`}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
}

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-[var(--shadow-card)] ${className}`}
    >
      {children}
    </div>
  );
}

export function Progress({
  value,
  indeterminate = false,
  className = "",
}: {
  value: number;
  indeterminate?: boolean;
  className?: string;
}) {
  return (
    <div className={`h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-recessed)] ${className}`}>
      {indeterminate ? (
        <div className="progress-indeterminate h-full w-1/3 rounded-full bg-[var(--accent)]" />
      ) : (
        <div
          className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-200 ease-out"
          style={{ width: `${Math.min(100, Math.max(0, value * 100))}%` }}
        />
      )}
    </div>
  );
}

export function Spinner({ className = "" }: { className?: string }) {
  return <Loader2 className={`size-4 animate-spin ${className}`} />;
}

export function Badge({
  children,
  tone = "neutral",
  className = "",
}: {
  children: ReactNode;
  tone?: "neutral" | "accent" | "good" | "warn" | "danger";
  className?: string;
}) {
  const tones = {
    neutral: "bg-[var(--bg-recessed)] text-[var(--ink-secondary)] border-[var(--border-subtle)]",
    accent: "bg-[var(--accent-soft)] text-[var(--accent-text)] border-[var(--accent)]/20",
    good: "bg-[var(--good-soft)] text-[var(--good)] border-[var(--good)]/20",
    warn: "bg-[var(--warn-soft)] text-[var(--warn)] border-[var(--warn)]/20",
    danger: "bg-[var(--danger-soft)] text-[var(--danger)] border-[var(--danger)]/20",
  };
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] leading-tight font-medium whitespace-nowrap ${tones[tone]} ${className}`}
    >
      {children}
    </span>
  );
}

export function IconButton({
  active = false,
  size = "md",
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  size?: "sm" | "md";
  children: ReactNode;
}) {
  const dim = size === "sm" ? "size-7 rounded-[7px]" : "size-8 rounded-lg";
  return (
    <button
      className={`inline-flex shrink-0 cursor-pointer items-center justify-center transition-colors duration-150 disabled:pointer-events-none disabled:opacity-35 ${dim} ${
        active
          ? "bg-[var(--accent-soft)] text-[var(--accent-text)]"
          : "text-[var(--ink-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--ink)] active:bg-[var(--bg-active)]"
      } ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

/** Tactile recessed segmented control with crisp elevated active thumb. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  size = "md",
  className = "",
}: {
  value: T;
  options: { value: T; label: ReactNode; title?: string; dot?: boolean }[];
  onChange: (v: T) => void;
  size?: "sm" | "md";
  className?: string;
}) {
  const h = size === "sm" ? "h-6 text-xs px-2.5" : "h-7 text-[13px] px-3";
  return (
    <div
      role="tablist"
      className={`inline-flex items-center gap-0.5 rounded-[9px] border border-[var(--border-subtle)] bg-[var(--bg-recessed)] p-[3px] ${className}`}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            role="tab"
            aria-selected={active}
            title={o.title}
            onClick={() => onChange(o.value)}
            className={`relative inline-flex cursor-pointer items-center justify-center gap-1.5 rounded-[6px] whitespace-nowrap transition-[background-color,color,box-shadow] duration-150 ${h} ${
              active
                ? "bg-[var(--bg-elevated)] font-medium text-[var(--ink)] shadow-[var(--shadow-xs)]"
                : "text-[var(--ink-secondary)] hover:text-[var(--ink)]"
            }`}
          >
            {o.label}
            {o.dot && <span className="size-1.5 rounded-full bg-[var(--accent)]" />}
          </button>
        );
      })}
    </div>
  );
}

export interface SelectOption<T extends string | number = string> {
  value: T;
  label: string;
  desc?: string;
  badge?: string;
}

/** Bespoke accessible editorial dropdown replacing native WebView2 <select>. */
export function CustomSelect<T extends string | number = string>({
  value,
  options,
  onChange,
  disabled = false,
  placeholder = "请选择",
  className = "",
  menuClassName = "",
}: {
  value: T;
  options: SelectOption<T>[];
  onChange: (v: T) => void;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  menuClassName?: string;
}) {
  const [open, setOpen] = useState(false);
  const [dropUp, setDropUp] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const listboxId = useId();

  const selectedIndex = options.findIndex((o) => o.value === value);
  const current = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const closeMenu = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) triggerRef.current?.focus();
  }, []);

  const openMenu = useCallback(
    (initialIdx?: number) => {
      if (disabled || options.length === 0) return;
      if (rootRef.current) {
        const rect = rootRef.current.getBoundingClientRect();
        setDropUp(window.innerHeight - rect.bottom < 220 && rect.top > 220);
      }
      const fallbackIdx = selectedIndex >= 0 ? selectedIndex : 0;
      const target = initialIdx !== undefined ? Math.min(Math.max(0, initialIdx), options.length - 1) : fallbackIdx;
      setActiveIndex(target);
      setOpen(true);
    },
    [disabled, options.length, selectedIndex],
  );

  useEffect(() => {
    if (!open) return;
    optionRefs.current[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [open, activeIndex]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) closeMenu(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeMenu(true);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setActiveIndex((i) => (options.length ? (i + 1) % options.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setActiveIndex((i) => (options.length ? (i - 1 + options.length) % options.length : 0));
      } else if (e.key === "Home") {
        e.preventDefault();
        e.stopPropagation();
        setActiveIndex(0);
      } else if (e.key === "End") {
        e.preventDefault();
        e.stopPropagation();
        setActiveIndex(Math.max(0, options.length - 1));
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        e.stopPropagation();
        const picked = options[activeIndex];
        if (picked) onChange(picked.value);
        closeMenu(true);
      } else if (e.key === "Tab") {
        closeMenu(false);
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, options, activeIndex, onChange, closeMenu]);

  const toggle = () => {
    if (disabled) return;
    if (open) {
      closeMenu(true);
    } else {
      openMenu();
    }
  };

  return (
    <div ref={rootRef} className={`relative ${className}`}>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={open && options[activeIndex] ? `${listboxId}-opt-${activeIndex}` : undefined}
        onClick={toggle}
        onKeyDown={(e) => {
          if (open || disabled) return;
          if (e.key === "ArrowDown") {
            e.preventDefault();
            openMenu();
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            openMenu(options.length - 1);
          } else if (e.key === "Home") {
            e.preventDefault();
            openMenu(0);
          } else if (e.key === "End") {
            e.preventDefault();
            openMenu(options.length - 1);
          } else if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            openMenu();
          }
        }}
        className={`flex h-8 w-full cursor-pointer items-center justify-between gap-2 rounded-lg border px-2.5 text-left text-[13px] transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
          open
            ? "border-[var(--accent)] bg-[var(--bg-elevated)] ring-2 ring-[var(--accent-ring)]"
            : "border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)]"
        }`}
      >
        <span className="min-w-0 flex-1 truncate text-[var(--ink)]">{current?.label ?? placeholder}</span>
        <ChevronDown
          className={`size-3.5 shrink-0 text-[var(--ink-muted)] transition-transform duration-150 ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div
          id={listboxId}
          role="listbox"
          className={`animate-panel-in absolute right-0 z-40 max-h-64 min-w-full overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--bg-elevated)] p-1 shadow-[var(--shadow-popover)] ${
            dropUp ? "bottom-full mb-1.5" : "top-full mt-1.5"
          } ${menuClassName}`}
        >
          {options.map((o, idx) => {
            const selected = o.value === value;
            const active = idx === activeIndex;
            return (
              <button
                key={String(o.value)}
                ref={(el) => {
                  optionRefs.current[idx] = el;
                }}
                id={`${listboxId}-opt-${idx}`}
                type="button"
                role="option"
                tabIndex={-1}
                aria-selected={selected}
                onMouseEnter={() => setActiveIndex(idx)}
                onClick={() => {
                  onChange(o.value);
                  closeMenu(true);
                }}
                className={`flex w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors ${
                  selected
                    ? active
                      ? "bg-[var(--accent-soft)] text-[var(--accent-text)] ring-1 ring-inset ring-[var(--accent)]/30"
                      : "bg-[var(--accent-soft)] text-[var(--accent-text)]"
                    : active
                      ? "bg-[var(--bg-hover)] text-[var(--ink)]"
                      : "text-[var(--ink)] hover:bg-[var(--bg-hover)]"
                }`}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-[13px] leading-snug">
                    <span className={`truncate ${selected ? "font-medium" : ""}`}>{o.label}</span>
                    {o.badge && (
                      <span className="shrink-0 rounded bg-[var(--bg-recessed)] px-1.5 py-px text-[10px] font-medium text-[var(--ink-secondary)]">
                        {o.badge}
                      </span>
                    )}
                  </div>
                  {o.desc && <div className="mt-0.5 truncate text-[11px] text-[var(--ink-muted)]">{o.desc}</div>}
                </div>
                {selected && <Check className="size-3.5 shrink-0 text-[var(--accent)]" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Tactile pill switch. */
export function Toggle({
  checked,
  disabled,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange?: (v: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange?.(!checked)}
      className={`relative inline-flex h-[22px] w-10 shrink-0 cursor-pointer items-center rounded-full border p-[2px] transition-colors duration-200 disabled:cursor-not-allowed disabled:opacity-40 ${
        checked
          ? "border-transparent bg-[var(--accent)]"
          : "border-[var(--border-strong)] bg-[var(--bg-recessed)]"
      }`}
    >
      <span
        className={`block size-4 rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.22)] transition-transform duration-200 ease-out ${
          checked ? "translate-x-[18px]" : "translate-x-0"
        }`}
      />
    </button>
  );
}

/** Tactile mechanical keycap chip (accepts single key or `Ctrl+Alt+Q` combo). */
export function Kbd({
  keys,
  dim = false,
  compact = false,
}: {
  keys: string;
  dim?: boolean;
  compact?: boolean;
}) {
  const parts = keys.split("+").filter(Boolean);
  return (
    <span className={`inline-flex items-center gap-0.5 ${dim ? "opacity-60" : ""}`}>
      {parts.map((k, i) => (
        <kbd
          key={`${k}-${i}`}
          className={`inline-flex items-center justify-center rounded-[5px] border border-[var(--border-strong)]/80 border-b-2 bg-[var(--bg-surface)] font-medium text-[var(--ink-secondary)] shadow-[inset_0_1px_0_rgb(255_255_255/0.4)] dark:shadow-none ${
            compact ? "h-[18px] min-w-[18px] px-1 text-[10px]" : "h-5 min-w-5 px-1.5 text-[11px]"
          }`}
        >
          {k}
        </kbd>
      ))}
    </span>
  );
}
