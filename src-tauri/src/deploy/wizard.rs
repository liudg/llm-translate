//! Deployment wizard commands (DESIGN 5.1). The step state machine lives in the
//! wizard window; these commands do the work of each step.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::downloader::{self, Control, DownloadError, Progress, StopReason};
use super::hardware::{self, HardwareInfo, Recommendation};
use super::manifest::ModelEntry;
use crate::app::settings::Backend;
use crate::state::AppState;

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
    {
        let mut map = state.downloads.lock().unwrap();
        if map.contains_key(&model_id) {
            return Err(DownloadError::Io("该模型正在下载".into()));
        }
        map.insert(model_id.clone(), ctl.clone());
    }
    let emit_id = model_id.clone();
    let progress: downloader::ProgressFn = Arc::new(move |p| {
        let _ = app.emit("download-progress", DownloadEvent { model_id: emit_id.clone(), progress: p });
    });
    tracing::info!("download {} -> {}", model.id, dir.display());
    let result = downloader::download(&state.http, &model, &dir, &ctl, progress).await;
    state.downloads.lock().unwrap().remove(&model_id);
    match &result {
        Ok(p) => tracing::info!("download ok: {}", p.display()),
        Err(DownloadError::Cancelled) => {
            let _ = std::fs::remove_file(downloader::part_path(&model.path_in(&dir)));
            tracing::info!("download cancelled: {}", model.id);
        }
        Err(e) => tracing::warn!("download {} stopped: {e:?}", model.id),
    }
    result
}

#[tauri::command]
pub fn download_control(state: State<'_, AppState>, model_id: String, action: String) {
    let reason = if action == "cancel" { StopReason::Cancel } else { StopReason::Pause };
    if let Some(ctl) = state.downloads.lock().unwrap().get(&model_id) {
        ctl.stop(reason);
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
    fell_back: bool,
    fallback_reason: Option<String>,
    sample_source: String,
    sample: String,
    load_ms: u64,
    translate_ms: u64,
}

const SELF_TEST: &str = "你好，世界";

/// Launch + SelfTest: start llama-server and translate a sample. A GPU failure
/// retries on CPU and that choice is saved.
#[tauri::command]
pub async fn deploy_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
    dir: PathBuf,
    backend: Backend,
    device: Option<String>,
    threads: u32,
) -> Result<LaunchOutcome, String> {
    let model = state.manifest.get(&model_id).cloned().ok_or(format!("未知模型 {model_id}"))?;
    let result = launch_and_test(&app, &state, &model, &dir, backend, device, threads).await;
    if result.is_err() {
        state.sup.stop().await;
    }
    result
}

/// Start `model` and run the self-test, falling back from GPU to CPU. Settings
/// are saved only on success.
async fn launch_and_test(
    app: &AppHandle,
    state: &AppState,
    model: &ModelEntry,
    dir: &Path,
    backend: Backend,
    device: Option<String>,
    threads: u32,
) -> Result<LaunchOutcome, String> {
    if !model.is_installed(dir) {
        return Err("模型文件不存在或不完整，请重新下载".into());
    }
    let emit = |phase: &'static str, backend: Backend, message: Option<String>| {
        let _ = app.emit("deploy-launch", LaunchEvent { phase, backend, message });
    };

    let mut fallback_reason = None;
    let mut attempt = (backend, device);
    loop {
        let (b, d) = attempt.clone();
        let spec = state.spec_for(model, dir, b, d.clone(), threads);
        state.sup.configure(spec).await;
        emit("starting", b, None);
        let t0 = Instant::now();
        let result = match state.sup.start().await {
            Ok(_) => {
                let load_ms = t0.elapsed().as_millis() as u64;
                emit("selftest", b, None);
                let t1 = Instant::now();
                match state.translator.translate_text(SELF_TEST, None).await {
                    Ok(s) if !s.trim().is_empty() => Ok((s, load_ms, t1.elapsed().as_millis() as u64)),
                    Ok(_) => Err("自检失败：翻译结果为空".to_string()),
                    Err(e) => Err(format!("自检失败：{e}")),
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok((sample, load_ms, translate_ms)) => {
                state
                    .settings
                    .update(|s| {
                        s.deployed = true;
                        s.model_id = Some(model.id.clone());
                        s.models_dir = dir.to_path_buf();
                        s.backend = b;
                        s.device = d.clone();
                        s.threads = threads.max(1);
                    })
                    .map_err(|e| format!("保存设置失败：{e}"))?;
                tracing::info!("deployed {} on {b:?}: {sample}", model.id);
                return Ok(LaunchOutcome {
                    backend: b,
                    fell_back: fallback_reason.is_some(),
                    fallback_reason,
                    sample_source: SELF_TEST.into(),
                    sample,
                    load_ms,
                    translate_ms,
                });
            }
            Err(e) if b == Backend::Vulkan => {
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
    let s = state.settings.get();
    let previous = state.spec_from_settings(&s);
    tracing::info!("switching model {:?} -> {}", s.model_id, model.id);
    let result = launch_and_test(&app, &state, &model, &s.models_dir, s.backend, s.device.clone(), s.threads).await;
    let Err(e) = result else { return result };
    tracing::warn!("switching to {} failed: {e}", model.id);
    let Some(prev) = previous else {
        state.sup.stop().await;
        return Err(e);
    };
    let sup = state.sup.clone();
    sup.configure(prev).await;
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
    if s.deployed && s.model_id.as_deref() == Some(model_id.as_str()) {
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
/// window.
#[tauri::command]
pub fn deploy_finish(app: AppHandle, autostart: Option<bool>) {
    if let Some(on) = autostart {
        let _ = crate::app::commands::apply_autostart(on);
    }
    crate::show_main(&app);
}

#[tauri::command]
pub fn open_wizard(app: AppHandle) {
    if let Some(w) = app.get_webview_window("wizard") {
        let _ = w.emit("wizard-reset", ());
        let _ = w.show();
        let _ = w.set_focus();
    }
}
