import { create } from "zustand";
import { api, events, type Settings } from "../lib/api";
import { applyTheme } from "../lib/theme";

interface SettingsStore {
  settings: Settings | null;
}

export const useSettings = create<SettingsStore>(() => ({ settings: null }));

let started = false;

/** Load settings once per window and follow `settings-changed`; also drives the theme. */
export function startSettingsSync() {
  if (started) return;
  started = true;
  const set = (settings: Settings) => {
    useSettings.setState({ settings });
    applyTheme(settings.theme);
  };
  events.settings(set);
  api.getSettings().then(set);
}
