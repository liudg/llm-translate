// Page theme: `data-theme` on <html> drives the CSS variables and Tailwind's
// `dark:` variant (see index.css). The native window theme (title bar, Mica
// tint) is set by the backend from the same setting.
import type { Theme } from "./api";

const media = window.matchMedia("(prefers-color-scheme: dark)");
let current: Theme = "system";

function resolve(): "light" | "dark" {
  if (current === "system") return media.matches ? "dark" : "light";
  return current;
}

function render() {
  document.documentElement.dataset.theme = resolve();
}

media.addEventListener("change", render);
render();

export function applyTheme(theme: Theme) {
  current = theme;
  render();
}
