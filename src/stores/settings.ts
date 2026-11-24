import { create } from "zustand";
import { api, events, type Settings } from "../lib/api";
import { applyTheme } from "../lib/theme";
import { follow, once } from "../lib/sync";

interface SettingsStore {
  settings: Settings | null;
}

export const useSettings = create<SettingsStore>(() => ({ settings: null }));

/** Load settings once per window and follow `settings-changed`; also drives the theme. */
export const startSettingsSync = once(() =>
  follow(events.settings, api.getSettings, (settings: Settings) => {
    useSettings.setState({ settings });
    applyTheme(settings.theme);
  }),
);
