//! Global shortcuts (DESIGN 5.7): input translation (mini window), screenshot
//! translation and the selection-translation switch. Bindings are stored in
//! settings as display strings (`Ctrl+Alt+Q`) and recorded on the settings page.
//!
//! The plugin's register / unregister run on the main thread and block until
//! done, so no lock of ours is held across those calls (the shortcut handler
//! also runs on the main thread and takes the lock).

use std::sync::Mutex;

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::settings::HotkeySettings;
use super::windows;
use crate::state::AppState;

/// Stable ids, also the field names in [`HotkeySettings`].
pub const IDS: [&str; 3] = ["mini", "shot", "selection"];

fn title(id: &str) -> &'static str {
    match id {
        "mini" => "输入翻译",
        "shot" => "截图翻译",
        _ => "划词翻译开关",
    }
}

const BUSY: &str = "已被其他程序占用";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInfo {
    pub id: &'static str,
    /// Display text such as `Alt+Q`; `None` when unbound.
    pub keys: Option<String>,
    pub registered: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    info: HotkeyInfo,
    shortcut: Option<Shortcut>,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    /// Recording on the settings page: everything is unregistered so pressing
    /// a bound combination records it instead of firing it.
    paused: bool,
}

#[derive(Default)]
pub struct Hotkeys(Mutex<Inner>);

impl Hotkeys {
    pub fn status(&self) -> Vec<HotkeyInfo> {
        self.0.lock().unwrap().entries.iter().map(|e| e.info.clone()).collect()
    }

    fn action_for(&self, shortcut: &Shortcut) -> Option<&'static str> {
        let inner = self.0.lock().unwrap();
        if inner.paused {
            return None;
        }
        inner.entries.iter().find(|e| e.shortcut.as_ref() == Some(shortcut)).map(|e| e.info.id)
    }
}

fn get(h: &HotkeySettings, id: &str) -> Option<String> {
    match id {
        "mini" => h.mini.clone(),
        "shot" => h.shot.clone(),
        _ => h.selection.clone(),
    }
}

fn set_field(h: &mut HotkeySettings, id: &str, keys: Option<String>) {
    match id {
        "mini" => h.mini = keys,
        "shot" => h.shot = keys,
        _ => h.selection = keys,
    }
}

/// Parse a display string. The global-hotkey parser calls the Windows key
/// `Super` and only knows some punctuation by name.
pub fn parse(keys: &str) -> Result<Shortcut, String> {
    let tokens: Vec<String> = keys
        .split('+')
        .map(|t| {
            match t.trim() {
                "Win" => "Super",
                "-" => "Minus",
                "." => "Period",
                "'" => "Quote",
                ";" => "Semicolon",
                "/" => "Slash",
                other => other,
            }
            .to_string()
        })
        .collect();
    // A lone "+" key would split into empty tokens; not supported.
    let shortcut: Shortcut = tokens.join("+").parse().map_err(|_| format!("无法识别的快捷键 {keys}"))?;
    if !shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) {
        return Err("快捷键需要包含 Ctrl、Alt 或 Win".into());
    }
    Ok(shortcut)
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            match app.state::<Hotkeys>().action_for(shortcut) {
                Some("mini") => windows::toggle_mini(app),
                Some("shot") => crate::capture::start(app, false),
                Some("selection") => toggle_selection(app),
                _ => {}
            }
        })
        .build()
}

/// Flip the selection-translation master switch; the tray icon follows the
/// settings change.
pub fn toggle_selection(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state.settings.update(|s| s.selection.enabled = !s.selection.enabled) {
        Ok(s) => tracing::info!("selection translation {}", if s.selection.enabled { "on" } else { "off" }),
        Err(e) => tracing::error!("saving selection switch failed: {e}"),
    }
}

fn try_register(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    app.global_shortcut().register(shortcut).map_err(|e| e.to_string())
}

fn publish(app: &AppHandle) {
    let status = app.state::<Hotkeys>().status();
    super::tray::update_hotkeys(app, &status);
    let _ = app.emit("hotkeys-changed", status);
}

/// Register the saved shortcuts at startup and remember which failed.
pub fn register_saved(app: &AppHandle) {
    let saved = app.state::<AppState>().settings.get().hotkeys;
    let mut entries = Vec::new();
    for id in IDS {
        let keys = get(&saved, id);
        let mut entry = Entry {
            info: HotkeyInfo { id, keys: keys.clone(), registered: false, error: None },
            shortcut: None,
        };
        if let Some(keys) = &keys {
            match parse(keys) {
                Ok(sc) => {
                    entry.shortcut = Some(sc);
                    match try_register(app, sc) {
                        Ok(()) => {
                            tracing::info!("registered {keys} ({id})");
                            entry.info.registered = true;
                        }
                        Err(e) => {
                            tracing::warn!("registering {keys} ({id}) failed: {e}");
                            entry.info.error = Some(BUSY.into());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("saved shortcut {keys} ({id}) is invalid: {e}");
                    entry.info.error = Some(e);
                }
            }
        }
        entries.push(entry);
    }
    app.state::<Hotkeys>().0.lock().unwrap().entries = entries;
    publish(app);
}

/// Pause (unregister all) while the settings page records a shortcut, and
/// register them again afterwards.
pub fn set_paused(app: &AppHandle, paused: bool) {
    let hk = app.state::<Hotkeys>();
    let entries = {
        let mut inner = hk.0.lock().unwrap();
        if inner.paused == paused {
            return;
        }
        inner.paused = paused;
        inner.entries.clone()
    };
    let mut results = Vec::new();
    for e in &entries {
        let Some(sc) = e.shortcut else { continue };
        if paused {
            if e.info.registered {
                let _ = app.global_shortcut().unregister(sc);
            }
        } else {
            // Something may have grabbed it in the meantime.
            let r = try_register(app, sc);
            if let Err(err) = &r {
                tracing::warn!("re-registering {:?} ({}) failed: {err}", e.info.keys, e.info.id);
            }
            results.push((e.info.id, r.is_ok()));
        }
    }
    if !paused {
        let mut inner = hk.0.lock().unwrap();
        for (id, ok) in results {
            if let Some(e) = inner.entries.iter_mut().find(|e| e.info.id == id) {
                e.info.registered = ok;
                e.info.error = (!ok).then(|| BUSY.to_string());
            }
        }
        drop(inner);
        publish(app);
    }
}

/// Bind `id` to `keys` (`None` unbinds). The new combination is registered
/// before anything is saved; when that fails the old binding stays.
pub fn set(app: &AppHandle, id: &str, keys: Option<String>) -> Result<(), String> {
    let id = IDS.into_iter().find(|i| *i == id).ok_or_else(|| format!("未知快捷键 {id}"))?;
    let new_sc = keys.as_deref().map(parse).transpose()?;
    let hk = app.state::<Hotkeys>();
    let (old, paused) = {
        let inner = hk.0.lock().unwrap();
        if let Some(sc) = new_sc
            && let Some(other) = inner.entries.iter().find(|e| e.info.id != id && e.shortcut == Some(sc))
        {
            return Err(format!("与“{}”的快捷键相同", title(other.info.id)));
        }
        let old = inner.entries.iter().find(|e| e.info.id == id).cloned().ok_or("快捷键状态未初始化")?;
        (old, inner.paused)
    };

    let unchanged = new_sc == old.shortcut && (old.info.registered || new_sc.is_none());
    let mut registered = false;
    if !unchanged {
        // Setting the old combination again after it failed lands here too
        // and acts as a retry.
        if let Some(sc) = new_sc {
            try_register(app, sc).map_err(|e| {
                tracing::info!("shortcut {:?} for {id} unavailable: {e}", keys);
                format!("{} {BUSY}", keys.as_deref().unwrap_or_default())
            })?;
            registered = true;
            if paused {
                // Registered again when recording ends.
                let _ = app.global_shortcut().unregister(sc);
            }
        }
        if let Some(old_sc) = old.shortcut
            && old.info.registered
            && Some(old_sc) != new_sc
            && !paused
        {
            let _ = app.global_shortcut().unregister(old_sc);
        }
    } else {
        registered = old.info.registered;
    }

    {
        let mut inner = hk.0.lock().unwrap();
        if let Some(e) = inner.entries.iter_mut().find(|e| e.info.id == id) {
            e.shortcut = new_sc;
            e.info.keys = keys.clone();
            e.info.registered = registered;
            e.info.error = None;
        }
    }
    tracing::info!("shortcut {id} = {keys:?}");
    let state = app.state::<AppState>();
    let saved = state.settings.update(|s| set_field(&mut s.hotkeys, id, keys.clone()));
    publish(app);
    saved.map(|_| ()).map_err(|e| format!("保存设置失败：{e}"))
}

#[tauri::command]
pub fn hotkey_status(state: tauri::State<'_, Hotkeys>) -> Vec<HotkeyInfo> {
    state.status()
}

#[tauri::command]
pub async fn hotkey_pause(app: AppHandle, paused: bool) {
    set_paused(&app, paused);
}

#[tauri::command]
pub async fn hotkey_set(app: AppHandle, id: String, keys: Option<String>) -> Result<(), String> {
    set(&app, &id, keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Code;

    #[test]
    fn parses_display_strings() {
        assert_eq!(parse("Alt+Q").unwrap(), Shortcut::new(Some(Modifiers::ALT), Code::KeyQ));
        assert_eq!(
            parse("Ctrl+Shift+Win+/").unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::SUPER), Code::Slash)
        );
        assert_eq!(parse("Ctrl+F1").unwrap().key, Code::F1);
        assert_eq!(parse("Alt+Space").unwrap().key, Code::Space);
        assert!(parse("Shift+A").is_err());
        assert!(parse("Q").is_err());
        assert!(parse("Ctrl+Nope").is_err());
    }
}
