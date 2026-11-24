//! Deployment wizard commands (DESIGN 5.1). The step state machine lives in the
//! wizard window; these commands do the work of each step.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use super::downloader::{self, Control, DownloadError, Progress, StopReason};
use super::hardware::{self, HardwareInfo, Recommendation};
use super::manifest::ModelEntry;
use crate::app::settings::Backend;
use crate::state::{ActiveDownload, AppState};

/// Launches + self-tests running (wizard and / or model switch). A count,
/// so one ending does not clear it while the other still runs.
static LAUNCHING: AtomicUsize = AtomicUsize::new(0);

struct Launching;
impl Launching {
    fn begin() -> Self {
        LAUNCHING.fetch_add(1, Ordering::SeqCst);
        Launching
    }
}
impl Drop for Launching {
    fn drop(&mut self) {
        LAUNCHING.fetch_sub(1, Ordering::SeqCst);
    }
}

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
    // Who brings the saved model back after the wizard closes: closing hides
    // the wizard, then reloads unless LAUNCHING is set (`on_wizard_closed`).
    // A launch sets LAUNCHING, then checks visibility, and when it ends
    // without saving it clears LAUNCHING, then checks again. Each side writes
    // before it reads, so at least one of them sees the other and reloads
    // (both reloading is harmless: it is the same spec).
    // A reload never undoes another launch (see `reload_saved`).
    let launching = Launching::begin();
    if !window.is_visible().unwrap_or(false) {
        drop(launching);
        reload_saved(&app);
        return Err("部署向导已关闭".into());
    }
    let mut candidate = None;
    let result = launch_and_test(&app, &model, &dir, backend, device, threads, &mut candidate).await;
    if result.is_err() {
        // Back to the saved model, not left on the failed candidate (stopped
        // even if it has the saved spec, so a retry starts it afresh), but
        // only while we still own it.
        if let Some(ours) = candidate {
            match state.spec_from_settings(&state.settings.get()) {
                Some(saved) => {
                    state.sup.configure_if(ours, saved, true).await;
                }
                None => state.sup.stop_if(ours).await,
            }
        }
        // Only now: a close until here skips its reload (LAUNCHING), so it
        // cannot claim the spec first and keep the failed process running.
        drop(launching);
        // Wizard gone: start the saved model now, including when this launch
        // failed before configuring anything (the close skipped its reload
        // because we were running). Else it starts on first use.
        if !window.is_visible().unwrap_or(false) {
            reload_saved(&app);
        }
    }
    result
}

/// Configure and start the model saved in settings (if any), in the
/// background, unless another launch (wizard / model switch) owns the model.
///
/// The spec's owner is read first, then LAUNCHING; a launch raises
/// LAUNCHING before it configures anything. So a launch that already
/// configured its candidate is seen here and left alone, and one that starts
/// later takes ownership, which makes `configure_if` refuse.
fn reload_saved(app: &AppHandle) {
    let state = app.state::<AppState>();
    let owner = state.sup.owner();
    if LAUNCHING.load(Ordering::SeqCst) > 0 {
        return; // that launch saves its model or restores the previous one
    }
    let Some(spec) = state.spec_from_settings(&state.settings.get()) else { return };
    let sup = state.sup.clone();
    tauri::async_runtime::spawn(async move {
        if sup.configure_if(owner, spec, false).await.is_none() {
            return; // someone else configured a model meanwhile
        }
        if let Err(e) = sup.ensure_ready().await {
            tracing::error!("reloading the saved model after the wizard closed failed: {e}");
        }
    });
}

/// The wizard is downloading or launching: closing it or clicking the tray
/// icon must not throw that away silently.
pub fn wizard_busy(app: &AppHandle) -> bool {
    LAUNCHING.load(Ordering::SeqCst) > 0
        || app.state::<AppState>().downloads.lock().unwrap().values().any(|d| d.from_wizard)
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
    // A launch still running is left alone: a failed wizard launch reloads
    // the saved model itself (see `deploy_launch`), a model switch restores
    // its previous model.
    reload_saved(app);
}

/// Start `model` and run the self-test, falling back from GPU to CPU. Settings
/// are saved only on success.
async fn launch_and_test(
    app: &AppHandle,
    model: &ModelEntry,
    dir: &Path,
    backend: Backend,
    device: Option<String>,
    threads: u32,
    candidate: &mut Option<u64>,
) -> Result<LaunchOutcome, String> {
    if !model.is_installed(dir) {
        return Err("模型文件不存在或不完整，请重新下载".into());
    }
    let state = app.state::<AppState>();
    let emit = |phase: &'static str, backend: Backend, message: Option<String>| {
        let _ = app.emit("deploy-launch", LaunchEvent { phase, backend, message });
    };

    let mut fallback_reason = None;
    let mut attempt = (backend, device);
    loop {
        let (b, d) = attempt.clone();
        let spec = state.spec_for(model, dir, b, d.clone(), threads);
        // The CPU retry only replaces our own GPU candidate (the automatic
        // CPU fallback keeps us the owner): if another launch took over
        // meanwhile, this one is over.
        let owner = match *candidate {
            None => state.sup.configure(spec).await,
            Some(ours) => match state.sup.configure_if(ours, spec, false).await {
                Some(owner) => owner,
                None => return Err("模型已被其他操作切换，本次部署已中止".into()),
            },
        };
        *candidate = Some(owner);
        emit("starting", b, None);
        let t0 = Instant::now();
        // Started only while we own the spec: the process tested is ours.
        let result = match state.sup.start_owned(owner).await {
            Ok((port, running)) => {
                let load_ms = t0.elapsed().as_millis() as u64;
                emit("selftest", running.backend, None);
                let t1 = Instant::now();
                match state.translator.self_test(SELF_TEST, model.clone(), port).await {
                    Ok(s) if !s.trim().is_empty() => Ok((s, load_ms, t1.elapsed().as_millis() as u64, running)),
                    Ok(_) => Err("自检失败：翻译结果为空".to_string()),
                    Err(e) => Err(format!("自检失败：{e}")),
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok((sample, load_ms, translate_ms, running)) => {
                // A concurrent request may have fallen back to CPU before we
                // started: save and report what actually passed the test.
                let (b, d) = (running.backend, running.device);
                if b != attempt.0 && fallback_reason.is_none() {
                    fallback_reason = Some("GPU 推理程序运行失败，已自动改用 CPU".to_string());
                }
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
    let launching = Launching::begin();
    let mut candidate = None;
    let result =
        launch_and_test(&app, &model, &s.models_dir, s.backend, s.device.clone(), s.threads, &mut candidate)
            .await;
    let Err(e) = result else { return result };
    tracing::warn!("switching to {} failed: {e}", model.id);
    let Some(prev) = previous else {
        if let Some(ours) = candidate {
            state.sup.stop_if(ours).await;
        }
        return Err(e);
    };
    // Only over our own failed candidate: a wizard launch may have taken over.
    let sup = state.sup.clone();
    let restored = match candidate {
        Some(ours) => sup.configure_if(ours, prev, true).await.is_some(),
        None => false,
    };
    if !restored {
        // Nothing of ours to roll back (failed before configuring, or taken
        // over). A wizard closed meanwhile skipped its reload because we were
        // running, so do it now; it leaves any other running launch alone.
        drop(launching);
        reload_saved(&app);
        return Err(e);
    }
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
    let existed = app.get_webview_window("wizard").is_some();
    if let Ok(w) = crate::app::windows::ensure_window(&app, "wizard") {
        if existed {
            let _ = w.emit("wizard-reset", ());
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
