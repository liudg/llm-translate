//! Settings-page commands (DESIGN 5.8). Every change goes through
//! `SettingsStore::update`, which publishes `settings-changed` to all windows.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::paths;
use super::settings::{Backend, SelectionSettings, Theme};
use crate::deploy::hardware::{self, HardwareInfo};
use crate::selection::blacklist::{self, WindowProcess};
use crate::state::AppState;
use crate::translate::detect::Direction;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub os_build: u32,
    /// Backdrop requested for this OS build: `Mica` (Win11) or `纯色` (Win10).
    pub effect: &'static str,
    pub config_dir: PathBuf,
    pub logs_dir: PathBuf,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppInfo>) -> AppInfo {
    state.inner().clone()
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<(), String> {
    state.settings.save(|s| s.theme = theme)
}

/// Re-apply native window theme and Win10 solid backing colour when the OS
/// toggles light/dark mode while `Theme::System` is active.
#[tauri::command]
pub fn sync_system_theme(app: AppHandle, state: State<'_, AppState>) {
    if state.settings.get().theme == Theme::System {
        super::windows::apply_theme(&app, Theme::System);
    }
}

#[tauri::command]
pub fn set_idle_release(state: State<'_, AppState>, minutes: u32) -> Result<(), String> {
    state.settings.save(|s| s.idle_release_minutes = minutes)
}

/// Change backend/device/threads and restart the model with them. The restart
/// runs in the background; progress shows up as `inference-status`.
#[tauri::command]
pub async fn set_inference(
    state: State<'_, AppState>,
    backend: Backend,
    device: Option<String>,
    threads: u32,
) -> Result<(), String> {
    let device = if backend == Backend::Cpu { None } else { device };
    let deploy = state.deploy.lock().await;
    state.settings.save(|s| {
        s.backend = backend;
        s.device = device.clone();
        s.threads = threads.max(1);
    })?;
    let Some(spec) = state.spec_from_settings(&state.settings.get()) else {
        return Ok(()); // not deployed; used on the next deployment
    };
    tracing::info!("inference settings changed: {backend:?} {device:?} {threads} threads");
    let sup = state.sup.clone();
    sup.configure(spec).await;
    drop(deploy);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sup.ensure_ready().await {
            tracing::error!("restart with new settings failed: {e}");
        }
    });
    Ok(())
}

/// CPU and GPU list for the backend picker. Unlike the wizard's detection this
/// does not stop the running model, so free VRAM excludes it.
#[tauri::command]
pub async fn hardware_info(state: State<'_, AppState>) -> Result<HardwareInfo, String> {
    Ok(hardware::detect(&state.llama_exe).await)
}

fn open_in_explorer(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

#[tauri::command]
pub fn open_logs_dir() {
    open_in_explorer(paths::logs_dir());
}

#[tauri::command]
pub fn open_models_dir(state: State<'_, AppState>) {
    open_in_explorer(state.settings.get().models_dir);
}

/// Async: it may create the mini window (see `windows::off_main_thread`).
#[tauri::command]
pub async fn open_mini(app: AppHandle) {
    super::windows::show_mini(&app);
}

/// Structured payload for Mini / Popup -> Main "open in main window" handoff,
/// preserving direction lock and any already-completed translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTranslatePayload {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>, // "auto" | "zh2En" | "en2Zh"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

/// Show the main window on the Translate page and hand off `payload` to it
/// (used by Mini / Popup "open in main window" handoff).
#[tauri::command]
pub async fn open_main_with_text(app: AppHandle, payload: OpenTranslatePayload) {
    use tauri::Emitter;
    super::windows::show_main(&app);
    let _ = app.emit_to("main", "navigate", "translate");
    let _ = app.emit_to("main", "open-translate", payload);
}

/// Replace the selection-translation settings; the hook picks them up through
/// the settings forwarder.
#[tauri::command]
pub fn set_selection(state: State<'_, AppState>, selection: SelectionSettings) -> Result<(), String> {
    state.settings.save(|s| s.selection = selection)
}

/// Start with Windows (HKCU Run, `--minimized`). Read from the registry each
/// time, so turning it off in Task Manager shows up here too.
#[tauri::command]
pub fn autostart_enabled() -> bool {
    super::autostart::is_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let r = super::autostart::set(enabled);
    tracing::info!("autostart {}: {r:?}", if enabled { "on" } else { "off" });
    r.map_err(|e| format!("设置开机自启失败：{e}"))
}

/// Processes with a visible window, for the blacklist picker.
#[tauri::command]
pub async fn window_processes() -> Vec<WindowProcess> {
    tauri::async_runtime::spawn_blocking(blacklist::window_processes).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_translate_payload_serde_camel_case() {
        let minimal: OpenTranslatePayload = serde_json::from_str(r#"{"text":"hello"}"#).unwrap();
        assert_eq!(minimal.text, "hello");
        assert!(minimal.choice.is_none());
        assert!(minimal.result.is_none());
        assert!(minimal.direction.is_none());
        assert!(minimal.mixed.is_none());
        assert!(minimal.elapsed_ms.is_none());
        assert_eq!(serde_json::to_string(&minimal).unwrap(), r#"{"text":"hello"}"#);

        let full_json = r#"{"text":"你好","choice":"zh2En","result":"Hello","direction":"zh2En","mixed":false,"elapsedMs":142}"#;
        let full: OpenTranslatePayload = serde_json::from_str(full_json).unwrap();
        assert_eq!(full.choice.as_deref(), Some("zh2En"));
        assert_eq!(full.result.as_deref(), Some("Hello"));
        assert_eq!(full.direction, Some(Direction::Zh2En));
        assert_eq!(full.mixed, Some(false));
        assert_eq!(full.elapsed_ms, Some(142));
        assert_eq!(serde_json::to_string(&full).unwrap(), full_json);
    }
}
