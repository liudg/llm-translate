//! Settings-page commands (DESIGN 5.8). Every change goes through
//! `SettingsStore::update`, which publishes `settings-changed` to all windows.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};

use super::paths;
use super::settings::{Backend, SelectionSettings, Theme};
use crate::deploy::hardware::{self, HardwareInfo};
use crate::selection::blacklist::{self, WindowProcess};
use crate::state::AppState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub os_build: u32,
    /// Backdrop requested for this OS build: `Mica` or `Acrylic`.
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
    state.settings.update(|s| s.theme = theme).map(|_| ()).map_err(|e| format!("保存设置失败：{e}"))
}

#[tauri::command]
pub fn set_idle_release(state: State<'_, AppState>, minutes: u32) -> Result<(), String> {
    state
        .settings
        .update(|s| s.idle_release_minutes = minutes)
        .map(|_| ())
        .map_err(|e| format!("保存设置失败：{e}"))
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
    let settings = state
        .settings
        .update(|s| {
            s.backend = backend;
            s.device = device.clone();
            s.threads = threads.max(1);
        })
        .map_err(|e| format!("保存设置失败：{e}"))?;
    let Some(spec) = state.spec_from_settings(&settings) else {
        return Ok(()); // not deployed; used on the next deployment
    };
    tracing::info!("inference settings changed: {backend:?} {device:?} {threads} threads");
    let sup = state.sup.clone();
    sup.configure(spec).await;
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

#[tauri::command]
pub fn open_mini(app: AppHandle) {
    super::windows::show_mini(&app);
}

/// Replace the selection-translation settings; the hook picks them up through
/// the settings forwarder.
#[tauri::command]
pub fn set_selection(state: State<'_, AppState>, selection: SelectionSettings) -> Result<(), String> {
    state
        .settings
        .update(|s| s.selection = selection)
        .map(|_| ())
        .map_err(|e| format!("保存设置失败：{e}"))
}

/// Start with Windows (HKCU Run, `--minimized`). Read from the registry each
/// time, so turning it off in Task Manager shows up here too.
#[tauri::command]
pub fn autostart_enabled() -> bool {
    super::autostart::is_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    apply_autostart(enabled)
}

pub fn apply_autostart(enabled: bool) -> Result<(), String> {
    let r = super::autostart::set(enabled);
    tracing::info!("autostart {}: {r:?}", if enabled { "on" } else { "off" });
    r.map_err(|e| format!("设置开机自启失败：{e}"))
}

/// Processes with a visible window, for the blacklist picker.
#[tauri::command]
pub async fn window_processes() -> Vec<WindowProcess> {
    tauri::async_runtime::spawn_blocking(blacklist::window_processes).await.unwrap_or_default()
}
