// Persistent translation history synchronized across windows.
import { create } from "zustand";
import type { Direction } from "../lib/api";

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
const MAX_ITEMS = 60;

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

interface HistoryStore {
  items: HistoryItem[];
  record: (entry: Omit<HistoryItem, "id" | "timestamp">) => void;
  remove: (id: string) => void;
  clear: () => void;
}

export const useHistory = create<HistoryStore>((set) => ({
  items: loadHistory(),
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
}));

// Keep all open windows (Main, Mini, Popup) in sync when another window writes history.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === HISTORY_KEY || e.key === null) {
      useHistory.setState({ items: loadHistory() });
    }
  });
}
