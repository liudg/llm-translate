//! Global shortcuts (DESIGN 5.7). M2 registers the fixed default Alt+Q for the
//! mini window; recording custom shortcuts comes with M5.

use std::sync::Mutex;

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::windows;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInfo {
    /// Stable id: `mini`.
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

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() == ShortcutState::Pressed && *shortcut == mini_shortcut() {
                windows::toggle_mini(app);
            }
        })
        .build()
}

/// Register the default shortcuts and remember which failed.
pub fn register_defaults(app: &AppHandle) {
    let shortcut = mini_shortcut();
    let result = app.global_shortcut().register(shortcut);
    let info = HotkeyInfo {
        id: "mini",
        keys: "Alt+Q".into(),
        registered: result.is_ok(),
        error: result.as_ref().err().map(|e| {
            tracing::warn!("registering Alt+Q failed: {e}");
            "已被其他程序占用".to_string()
        }),
    };
    if info.registered {
        tracing::info!("registered Alt+Q for the mini window");
    }
    *app.state::<Hotkeys>().0.lock().unwrap() = vec![info];
}

#[tauri::command]
pub fn hotkey_status(state: tauri::State<'_, Hotkeys>) -> Vec<HotkeyInfo> {
    state.0.lock().unwrap().clone()
}
