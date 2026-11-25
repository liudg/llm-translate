// Persistent translation history & reading preferences synchronized across windows.
import { create } from "zustand";
import type { Direction } from "../lib/api";

export type ReadingScale = "sm" | "md" | "lg";
export type ReadingFont = "sans" | "serif";
export type HistoryOrigin = "main" | "mini" | "popup";

export interface HistoryItem {
  id: string;
  source: string;
  result: string;
  direction: Direction;
  mixed: boolean;
  elapsedMs: number | null;
  timestamp: number;
  origin?: HistoryOrigin;
}

const HISTORY_KEY = "llm-translate.history.v1";
const PREFS_KEY = "llm-translate.prefs.v1";
const MAX_ITEMS = 60;

interface ReadingPrefs {
  fontScale: ReadingScale;
  fontFamily: ReadingFont;
}

function loadHistory(): HistoryItem[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.slice(0, MAX_ITEMS) : [];
  } catch {
    return [];
  }
}

function saveHistory(items: HistoryItem[]) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(items.slice(0, MAX_ITEMS)));
  } catch {
    // Ignore storage quota issues.
  }
}

function loadPrefs(): ReadingPrefs {
  let fontScale: ReadingScale = "md";
  let fontFamily: ReadingFont = "sans";
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<ReadingPrefs>;
      if (parsed.fontScale === "sm" || parsed.fontScale === "md" || parsed.fontScale === "lg") {
        fontScale = parsed.fontScale;
      }
      if (parsed.fontFamily === "sans" || parsed.fontFamily === "serif") {
        fontFamily = parsed.fontFamily;
      }
    }
  } catch {
    // Ignore parse errors.
  }
  return { fontScale, fontFamily };
}

function savePrefs(patch: Partial<ReadingPrefs>): ReadingPrefs {
  const next = { ...loadPrefs(), ...patch };
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(next));
  } catch {
    // Ignore storage errors.
  }
  return next;
}

interface HistoryStore {
  items: HistoryItem[];
  fontScale: ReadingScale;
  fontFamily: ReadingFont;
  record: (entry: Omit<HistoryItem, "id" | "timestamp">) => void;
  remove: (id: string) => void;
  clear: () => void;
  setFontScale: (scale: ReadingScale) => void;
  setFontFamily: (f: ReadingFont) => void;
}

const initialPrefs = loadPrefs();

export const useHistory = create<HistoryStore>((set) => ({
  items: loadHistory(),
  fontScale: initialPrefs.fontScale,
  fontFamily: initialPrefs.fontFamily,
  record: (entry) => {
    const source = entry.source.trim();
    const result = entry.result.trim();
    if (!source || !result) return;
    // Re-read latest items from localStorage so concurrent writes from other windows are never overwritten.
    const latest = loadHistory();
    const filtered = latest.filter(
      (it) => !(it.source === source && it.direction === entry.direction && it.mixed === entry.mixed),
    );
    const item: HistoryItem = {
      ...entry,
      source,
      result,
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      timestamp: Date.now(),
    };
    const items = [item, ...filtered].slice(0, MAX_ITEMS);
    saveHistory(items);
    set({ items });
  },
  remove: (id) => {
    const latest = loadHistory();
    const items = latest.filter((it) => it.id !== id);
    saveHistory(items);
    set({ items });
  },
  clear: () => {
    saveHistory([]);
    set({ items: [] });
  },
  setFontScale: (fontScale) => {
    const next = savePrefs({ fontScale });
    set({ fontScale: next.fontScale, fontFamily: next.fontFamily });
  },
  setFontFamily: (fontFamily) => {
    const next = savePrefs({ fontFamily });
    set({ fontScale: next.fontScale, fontFamily: next.fontFamily });
  },
}));

// Keep all open windows (Main, Mini, Popup) in sync when another window writes history or prefs.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === HISTORY_KEY || e.key === null) {
      useHistory.setState({ items: loadHistory() });
    }
    if (e.key === PREFS_KEY || e.key === null) {
      const prefs = loadPrefs();
      useHistory.setState({ fontScale: prefs.fontScale, fontFamily: prefs.fontFamily });
    }
  });
}
