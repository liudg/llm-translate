import type React from "react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Loader2 } from "lucide-react";

type Variant = "primary" | "secondary" | "subtle";

const variants: Record<Variant, string> = {
  primary:
    "bg-[var(--accent)] text-white hover:bg-[var(--accent-hover)] disabled:bg-neutral-400/60 dark:text-black dark:disabled:bg-neutral-600",
  secondary:
    "border border-black/10 bg-white/70 hover:bg-white dark:border-white/10 dark:bg-white/[0.06] dark:hover:bg-white/10",
  subtle: "hover:bg-black/5 dark:hover:bg-white/10",
};

export function Button({
  variant = "secondary",
  className = "",
  icon,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; icon?: ReactNode }) {
  return (
    <button
      className={`inline-flex h-8 items-center justify-center gap-1.5 rounded-md px-3 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${variants[variant]} ${className}`}
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
      className={`rounded-lg border border-black/[0.06] bg-white/60 shadow-sm dark:border-white/[0.06] dark:bg-white/[0.04] ${className}`}
    >
      {children}
    </div>
  );
}

export function Progress({ value, indeterminate = false }: { value: number; indeterminate?: boolean }) {
  return (
    <div className="h-1 w-full overflow-hidden rounded-full bg-black/10 dark:bg-white/10">
      {indeterminate ? (
        <div className="progress-indeterminate h-full w-1/3 rounded-full bg-[var(--accent)]" />
      ) : (
        <div
          className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-200"
          style={{ width: `${Math.min(100, Math.max(0, value * 100))}%` }}
        />
      )}
    </div>
  );
}

export function Spinner({ className = "" }: { className?: string }) {
  return <Loader2 className={`size-4 animate-spin ${className}`} />;
}

export function Badge({ children, tone = "neutral" }: { children: ReactNode; tone?: "neutral" | "accent" | "good" | "warn" }) {
  const tones = {
    neutral: "bg-black/5 text-neutral-600 dark:bg-white/10 dark:text-neutral-300",
    accent: "bg-[var(--accent)]/15 text-[var(--accent-text)]",
    good: "bg-green-600/15 text-green-700 dark:text-green-400",
    warn: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  };
  return <span className={`rounded px-1.5 py-0.5 text-xs ${tones[tone]}`}>{children}</span>;
}

export function IconButton({
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { children: ReactNode }) {
  return (
    <button
      className={`inline-flex size-8 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-black/5 disabled:pointer-events-none disabled:opacity-35 dark:hover:bg-white/10 ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

/** Fluent-style segmented control. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  size = "md",
}: {
  value: T;
  options: { value: T; label: ReactNode; title?: string }[];
  onChange: (v: T) => void;
  size?: "sm" | "md";
}) {
  const h = size === "sm" ? "h-7 text-xs" : "h-8 text-sm";
  return (
    <div className="inline-flex rounded-md border border-[var(--stroke)] bg-[var(--surface)] p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          title={o.title}
          onClick={() => onChange(o.value)}
          className={`${h} rounded-[5px] px-3 whitespace-nowrap transition-colors ${
            o.value === value
              ? "bg-[var(--accent)] text-white shadow-sm dark:text-black"
              : "hover:bg-black/5 dark:hover:bg-white/10"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Select({
  className = "",
  children,
  ...rest
}: React.SelectHTMLAttributes<HTMLSelectElement> & { children: ReactNode }) {
  return (
    <select
      className={`h-8 rounded-md border border-[var(--stroke)] bg-[var(--surface-strong)] px-2 text-sm outline-none focus:border-[var(--accent)] disabled:opacity-50 ${className}`}
      {...rest}
    >
      {children}
    </select>
  );
}

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
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange?.(!checked)}
      className={`relative h-5 w-10 shrink-0 rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
        checked
          ? "border-transparent bg-[var(--accent)]"
          : "border-black/40 bg-transparent dark:border-white/50"
      }`}
    >
      <span
        className={`absolute top-1/2 size-3 -translate-y-1/2 rounded-full transition-all ${
          checked ? "left-[22px] bg-white dark:bg-black" : "left-[3px] bg-black/60 dark:bg-white/70"
        }`}
      />
    </button>
  );
}
