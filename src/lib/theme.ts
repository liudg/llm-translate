// Page theme: `data-theme` on <html> drives the CSS variables and Tailwind's
// `dark:` variant (see index.css). The native window theme (title bar, Mica
// tint) is set by the backend from the same setting and pre-injected on first paint.
import { useSyncExternalStore } from "react";
import { api, type Theme } from "./api";

export type ResolvedTheme = "light" | "dark";

const media = window.matchMedia("(prefers-color-scheme: dark)");
let current: Theme | null = null;

function initialResolved(): ResolvedTheme {
  const preset = document.documentElement.dataset.theme;
  if (preset === "dark" || preset === "light") return preset;
  return media.matches ? "dark" : "light";
}

let resolved: ResolvedTheme = initialResolved();
document.documentElement.dataset.theme = resolved;

const listeners = new Set<() => void>();

function compute(): ResolvedTheme {
  if (current === "light" || current === "dark") return current;
  return media.matches ? "dark" : "light";
}

function render() {
  const next = compute();
  if (next === resolved && document.documentElement.dataset.theme === next) return;
  resolved = next;
  document.documentElement.dataset.theme = next;
  listeners.forEach((fn) => fn());
}

media.addEventListener("change", () => {
  if (current === "system") {
    render();
    api.syncSystemTheme().catch(() => {});
  } else if (current === null) {
    render();
  }
});

export function applyTheme(theme: Theme) {
  current = theme;
  render();
}

/** Reactive resolved theme (`"light" | "dark"`), updates on both settings & OS changes. */
export function useResolvedTheme(): ResolvedTheme {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    () => resolved,
  );
}
