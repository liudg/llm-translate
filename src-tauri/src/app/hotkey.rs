//! Global shortcuts (DESIGN 5.7). Fixed defaults for now: Alt+Q opens the mini
//! window (M2), Alt+W starts a screenshot translation (M4). Recording custom
//! shortcuts comes with M5.

use std::sync::Mutex;

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::windows;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInfo {
    /// Stable id: `mini` or `shot`.
    pub id: &'static str,
    /// Display text such as `Alt+Q`.
    pub keys: String,
    pub registered: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Hotkeys(pub Mutex<Vec<HotkeyInfo>>);

fn mini_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT), Code::KeyQ)
}

fn shot_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT), Code::KeyW)
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if *shortcut == mini_shortcut() {
                windows::toggle_mini(app);
            } else if *shortcut == shot_shortcut() {
                crate::capture::start(app, false);
            }
        })
        .build()
}

/// Register the default shortcuts and remember which failed.
pub fn register_defaults(app: &AppHandle) {
    let defaults = [("mini", "Alt+Q", mini_shortcut()), ("shot", "Alt+W", shot_shortcut())];
    let infos = defaults
        .into_iter()
        .map(|(id, keys, shortcut)| {
            let result = app.global_shortcut().register(shortcut);
            match &result {
                Ok(()) => tracing::info!("registered {keys} ({id})"),
                Err(e) => tracing::warn!("registering {keys} ({id}) failed: {e}"),
            }
            HotkeyInfo {
                id,
                keys: keys.into(),
                registered: result.is_ok(),
                error: result.err().map(|_| "已被其他程序占用".to_string()),
            }
        })
        .collect();
    *app.state::<Hotkeys>().0.lock().unwrap() = infos;
}

#[tauri::command]
pub fn hotkey_status(state: tauri::State<'_, Hotkeys>) -> Vec<HotkeyInfo> {
    state.0.lock().unwrap().clone()
}
