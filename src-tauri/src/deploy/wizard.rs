//! Deployment wizard commands (DESIGN 5.1). The step state machine lives in the
//! wizard window; these commands do the work of each step.
//!
//! Everything that changes which model runs (wizard launch, model switch, new
//! inference settings, bringing the saved model back) holds
//! `AppState::deploy`, so those never interleave.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use super::downloader::{self, Control, DownloadError, Progress, StopReason};
use super::hardware::{self, HardwareInfo, Recommendation};
use super::manifest::ModelEntry;
use crate::app::settings::Backend;
use crate::state::{ActiveDownload, AppState};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectResult {
    hardware: HardwareInfo,
    recommendation: Recommendation,
    models_dir: PathBuf,
}

#[tauri::command]
pub async fn deploy_detect(state: State<'_, AppState>) -> Result<DetectResult, String> {
    // Our own model would hide its VRAM from the free-memory reading. It is
    // started again by the launch step or the next translation.
    let _deploy = state.deploy.lock().await;
    state.sup.stop().await;
    let hardware = hardware::detect(&state.llama_exe).await;
    let recommendation = hardware::recommend(&hardware);
    tracing::info!("hardware: {hardware:?}");
    tracing::info!("recommendation: {recommendation:?}");
    Ok(DetectResult { hardware, recommendation, models_dir: state.settings.get().models_dir })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    #[serde(flatten)]
    entry: ModelEntry,
    installed: bool,
    downloaded_bytes: u64,
}

#[tauri::command]
pub fn deploy_models(state: State<'_, AppState>, dir: PathBuf) -> Vec<ModelView> {
    state
        .manifest
        .models
        .iter()
        .map(|m| ModelView {
            entry: m.clone(),
            installed: m.is_installed(&dir),
            downloaded_bytes: downloader::downloaded_bytes(m, &dir),
        })
        .collect()
}

#[tauri::command]
pub fn disk_free_bytes(dir: PathBuf) -> Option<u64> {
    hardware::disk_free(&dir)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadEvent {
    model_id: String,
    #[serde(flatten)]
    progress: Progress,
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    model_id: String,
    dir: PathBuf,
) -> Result<PathBuf, DownloadError> {
    let model = state
        .manifest
        .get(&model_id)
        .cloned()
        .ok_or_else(|| DownloadError::Io(format!("未知模型 {model_id}")))?;
    let ctl = Control::default();
    let from_wizard = window.label() == "wizard";
    {
        let mut map = state.downloads.lock().unwrap();
        if map.contains_key(&model_id) {
            return Err(DownloadError::Io("该模型正在下载".into()));
        }
        map.insert(model_id.clone(), ActiveDownload { ctl: ctl.clone(), from_wizard });
    }
    // Registered first, then checked: closing the wizard hides it, then
    // pauses what is registered. So a download either gets paused by the
    // close or sees the wizard gone here, never runs on unseen.
    if from_wizard && !window.is_visible().unwrap_or(false) {
        state.downloads.lock().unwrap().remove(&model_id);
        return Err(DownloadError::Io("部署向导已关闭".into()));
    }
    let emit_id = model_id.clone();
    let emitter = app.clone();
    let progress: downloader::ProgressFn = Arc::new(move |p| {
        let _ = emitter.emit("download-progress", DownloadEvent { model_id: emit_id.clone(), progress: p });
    });
    tracing::info!("download {} -> {}", model.id, dir.display());
    let result = downloader::download(&state.http, &model, &dir, &ctl, progress).await;
    match &result {
        Ok(p) => tracing::info!("download ok: {}", p.display()),
        Err(DownloadError::Cancelled) => {
            let _ = std::fs::remove_file(downloader::part_path(&model.path_in(&dir)));
            tracing::info!("download cancelled: {}", model.id);
        }
        Err(e) => tracing::warn!("download {} stopped: {e:?}", model.id),
    }
    // Released only once the files are final (a cancelled `.part` is gone).
    state.downloads.lock().unwrap().remove(&model_id);
    if from_wizard {
        // The settings page refreshes now (e.g. after the wizard closed and
        // paused this download), not when the pause was requested.
        let _ = app.emit_to("main", "models-changed", ());
    }
    result
}

#[tauri::command]
pub fn download_control(state: State<'_, AppState>, model_id: String, action: String) {
    let reason = if action == "cancel" { StopReason::Cancel } else { StopReason::Pause };
    if let Some(d) = state.downloads.lock().unwrap().get(&model_id) {
        d.ctl.stop(reason);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchEvent {
    phase: &'static str,
    backend: Backend,
    message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOutcome {
    backend: Backend,
    /// Set when the GPU failed and the model runs on CPU instead.
    fallback_reason: Option<String>,
    sample: String,
    load_ms: u64,
    translate_ms: u64,
}

/// Translated by the self-test (the pages show the same text).
const SELF_TEST: &str = "你好，世界";

/// Launch + SelfTest: start llama-server and translate a sample. A GPU failure
/// retries on CPU and that choice is saved.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // command arguments come from the page
pub async fn deploy_launch(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    model_id: String,
    dir: PathBuf,
    backend: Backend,
    device: Option<String>,
    threads: u32,
) -> Result<LaunchOutcome, String> {
    let model = state.manifest.get(&model_id).cloned().ok_or(format!("未知模型 {model_id}"))?;
    check_installed(&model, &dir)?;
    let _deploy = state.deploy.lock().await;
    // Closing the wizard queues a reload of the saved model behind this lock,
    // so a launch from a closed wizard is not wanted any more.
    if !window.is_visible().unwrap_or(false) {
        return Err("部署向导已关闭".into());
    }
    let result = launch_and_test(&state, &app, &model, &dir, backend, device, threads).await;
    if result.is_err() {
        // Back to the saved model; it starts on first use.
        restore_saved(&state).await;
    }
    result
}

/// Checked before anything is stopped, so a missing file never costs the
/// running model.
fn check_installed(model: &ModelEntry, dir: &Path) -> Result<(), String> {
    if model.is_installed(dir) { Ok(()) } else { Err("模型文件不存在或不完整，请重新下载".into()) }
}

/// Back to the model saved in settings after a failed launch. The candidate
/// is stopped first, so even one with the saved spec starts afresh next time.
/// Returns whether a saved model exists.
async fn restore_saved(state: &AppState) -> bool {
    state.sup.stop().await;
    state.sup.configure_with(|| state.spec_from_settings(&state.settings.get())).await
}

/// Configure and start the model saved in settings (if any) in the
/// background, once any launch in progress is over.
fn reload_saved(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let configured = {
            let _deploy = state.deploy.lock().await;
            state.sup.configure_with(|| state.spec_from_settings(&state.settings.get())).await
        };
        if configured && let Err(e) = state.sup.ensure_ready().await {
            tracing::error!("reloading the saved model after the wizard closed failed: {e}");
        }
    });
}

/// The wizard is downloading or a launch is running: closing it or clicking
/// the tray icon must not throw that away silently.
pub fn wizard_busy(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    state.deploy.try_lock().is_err() || state.downloads.lock().unwrap().values().any(|d| d.from_wizard)
}

/// Re-deploy wizard closed without finishing: pause its downloads (the
/// `.part` stays; 设置 → 模型 offers 继续下载) and bring the saved model back,
/// which the hardware detection step stopped.
pub fn on_wizard_closed(app: &AppHandle) {
    let state = app.state::<AppState>();
    for (id, d) in state.downloads.lock().unwrap().iter() {
        if d.from_wizard {
            tracing::info!("wizard closed; pausing download of {id}");
            d.ctl.stop(StopReason::Pause);
        }
    }
    // The settings page refreshes when each paused download has wound down
    // (`download_model` emits `models-changed`).
    reload_saved(app);
}

/// Start `model` and run the self-test, falling back from GPU to CPU. Settings
/// are saved only on success. The caller holds `AppState::deploy` and has
/// checked the model file (`check_installed`).
async fn launch_and_test(
    state: &AppState,
    app: &AppHandle,
    model: &ModelEntry,
    dir: &Path,
    backend: Backend,
    device: Option<String>,
    threads: u32,
) -> Result<LaunchOutcome, String> {
    let emit = |phase: &'static str, backend: Backend, message: Option<String>| {
        let _ = app.emit("deploy-launch", LaunchEvent { phase, backend, message });
    };

    let mut fallback_reason = None;
    let mut attempt = (backend, device);
    loop {
        let (b, d) = attempt.clone();
        state.sup.configure(state.spec_for(model, dir, b, d, threads)).await;
        emit("starting", b, None);
        let t0 = Instant::now();
        // `tried`: the backend that actually ran, if this attempt fails (a
        // request's CPU fallback may already have moved it to CPU).
        let (result, tried) = match state.sup.start().await {
            Err(e) => (Err(e), b),
            Ok((port, running)) => {
                let load_ms = t0.elapsed().as_millis() as u64;
                let tried = running.backend;
                emit("selftest", tried, None);
                let t1 = Instant::now();
                let r = match state.translator.self_test(SELF_TEST, model, port).await {
                    Ok(s) if !s.trim().is_empty() => Ok((s, load_ms, t1.elapsed().as_millis() as u64, running)),
                    Ok(_) => Err("自检失败：翻译结果为空".to_string()),
                    Err(e) => Err(format!("自检失败：{e}")),
                };
                (r, tried)
            }
        };
        match result {
            Ok((sample, load_ms, translate_ms, running)) => {
                // Save and report what actually passed the test.
                if running.backend != attempt.0 && fallback_reason.is_none() {
                    fallback_reason = Some("GPU 推理程序运行失败，已自动改用 CPU".to_string());
                }
                state.settings.save(|s| {
                    s.deployed = true;
                    s.model_id = Some(model.id.clone());
                    s.models_dir = dir.to_path_buf();
                    s.backend = running.backend;
                    s.device = running.device.clone();
                    s.threads = threads.max(1);
                })?;
                tracing::info!("deployed {} on {:?}: {sample}", model.id, running.backend);
                return Ok(LaunchOutcome { backend: running.backend, fallback_reason, sample, load_ms, translate_ms });
            }
            Err(e) if tried == Backend::Vulkan => {
                tracing::warn!("GPU launch failed, retrying on CPU: {e}");
                emit("fallback", Backend::Cpu, Some(e.clone()));
                fallback_reason = Some(e);
                attempt = (Backend::Cpu, None);
            }
            Err(e) => {
                return Err(match fallback_reason {
                    Some(g) => format!("GPU 启动失败（{g}），CPU 也失败：{e}"),
                    None => e,
                });
            }
        }
    }
}

/// Model management (DESIGN 5.8): switch to another downloaded model with the
/// saved backend. Runs the same launch + self-test as the wizard; if even the
/// CPU attempt fails, the previous model is loaded again.
#[tauri::command]
pub async fn model_switch(app: AppHandle, state: State<'_, AppState>, model_id: String) -> Result<LaunchOutcome, String> {
    let model = state.manifest.get(&model_id).cloned().ok_or(format!("未知模型 {model_id}"))?;
    check_installed(&model, &state.settings.get().models_dir)?;
    let _deploy = state.deploy.lock().await;
    let s = state.settings.get();
    tracing::info!("switching model {:?} -> {}", s.model_id, model.id);
    let result = launch_and_test(&state, &app, &model, &s.models_dir, s.backend, s.device.clone(), s.threads).await;
    let Err(e) = result else { return result };
    tracing::warn!("switching to {} failed: {e}", model.id);
    if !restore_saved(&state).await {
        return Err(e);
    }
    let sup = state.sup.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sup.ensure_ready().await {
            tracing::error!("restoring the previous model failed: {e}");
        }
    });
    Err(format!("{e}\n已恢复为原来的模型。"))
}

/// Delete a downloaded (or partly downloaded) model file. The model in use and
/// a model being downloaded cannot be deleted.
#[tauri::command]
pub fn model_delete(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    let model = state.manifest.get(&model_id).ok_or(format!("未知模型 {model_id}"))?;
    let s = state.settings.get();
    let in_use = s.deployed && s.model_id.as_deref() == Some(model_id.as_str());
    if in_use || state.sup.spec().is_some_and(|spec| spec.model_id == model_id) {
        return Err("正在使用的模型不能删除，请先切换到其他模型".into());
    }
    if state.downloads.lock().unwrap().contains_key(&model_id) {
        return Err("该模型正在下载，请先取消下载".into());
    }
    let path = model.path_in(&s.models_dir);
    for p in [downloader::part_path(&path), path] {
        match std::fs::remove_file(&p) {
            Ok(()) => tracing::info!("deleted {}", p.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("删除 {} 失败：{e}", p.display())),
        }
    }
    Ok(())
}

/// Leave the wizard: apply the "start with Windows" choice and show the main
/// window. Async so it runs off the main thread: creating the main window from
/// a synchronous command deadlocks on Windows (see `off_main_thread`).
#[tauri::command]
pub async fn deploy_finish(app: AppHandle, autostart: Option<bool>) {
    if let Some(on) = autostart {
        let _ = crate::app::commands::set_autostart(on);
    }
    crate::show_main(&app);
}

/// Async for the same reason as `deploy_finish`: it may create the wizard.
#[tauri::command]
pub async fn open_wizard(app: AppHandle) {
    let existed = app.get_webview_window("wizard").is_some();
    if let Ok(w) = crate::app::windows::ensure_window(&app, "wizard") {
        if let Some(main) = app.get_webview_window("main")
            && main.is_visible().unwrap_or(false)
        {
            crate::app::windows::save_main_placement(&app);
            let _ = main.hide();
        }
        if existed {
            let _ = w.emit("wizard-reset", ());
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
