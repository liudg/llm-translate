pub mod app;
pub mod deploy;
pub mod inference;
pub mod state;
pub mod translate;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

use app::settings::{Backend, Settings, SettingsStore};
use deploy::manifest::Manifest;
use inference::supervisor::{Notice, Status, Supervisor};
use state::AppState;
use translate::scheduler::{TranslateEvent, TranslateRequest, Translator};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EffectInfo {
    os_build: u32,
    effect: &'static str,
}

/// Win11 (build >= 22000) -> Mica, Win10 -> Acrylic.
fn pick_effect(build: u32) -> (Effect, &'static str) {
    if build >= 22000 { (Effect::Mica, "Mica") } else { (Effect::Acrylic, "Acrylic") }
}

#[tauri::command]
fn window_effect_info(state: State<EffectInfo>) -> EffectInfo {
    state.inner().clone()
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
fn inference_status(state: State<'_, AppState>) -> Status {
    state.sup.status()
}

#[tauri::command]
async fn translate(state: State<'_, AppState>, req: TranslateRequest, on_event: Channel<TranslateEvent>) -> Result<(), ()> {
    let translator = state.translator.clone();
    translator
        .translate(req, move |ev| {
            let _ = on_event.send(ev);
        })
        .await;
    Ok(())
}

#[tauri::command]
fn translate_cancel(state: State<'_, AppState>) {
    state.translator.cancel();
}

#[tauri::command]
fn open_logs_dir() {
    let _ = std::process::Command::new("explorer").arg(app::paths::logs_dir()).spawn();
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("wizard") {
        let _ = w.hide();
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_state(app: &AppHandle) -> anyhow::Result<AppState> {
    let res = app.path().resource_dir()?;
    let manifest = Arc::new(Manifest::load(&res.join("models.json"))?);
    let llama_exe = res.join("llama").join("llama-server.exe");
    let settings = SettingsStore::new(Settings::load());

    let notice_app = app.clone();
    let notice_settings = settings.clone();
    let sup = Supervisor::new(Arc::new(move |n: Notice| {
        match &n {
            Notice::FellBackToCpu { .. } => {
                if let Err(e) = notice_settings.update(|s| {
                    s.backend = Backend::Cpu;
                    s.device = None;
                }) {
                    tracing::error!("saving CPU fallback failed: {e}");
                }
            }
        }
        let _ = notice_app.emit("inference-notice", n);
    }));
    sup.set_idle_minutes(settings.get().idle_release_minutes);
    let translator = Arc::new(Translator::new(sup.clone(), manifest.clone()));
    Ok(AppState {
        settings,
        manifest,
        llama_exe,
        sup,
        translator,
        downloads: Mutex::new(HashMap::new()),
        http: deploy::downloader::client(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = app::logging::init();
    tracing::info!("LLM Translate {} starting", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let build = windows_version::OsVersion::current().build;
            let (effect, name) = pick_effect(build);
            let mut applied = "none";
            for label in ["main", "wizard"] {
                if let Some(win) = app.get_webview_window(label) {
                    if win.set_effects(EffectsBuilder::new().effect(effect).build()).is_ok() {
                        applied = name;
                    }
                }
            }
            app.manage(EffectInfo { os_build: build, effect: applied });

            let state = build_state(app.handle())?;
            let settings = state.settings.get();
            let spec = state.spec_from_settings(&settings);
            let sup = state.sup.clone();
            app.manage(state);

            // Forward supervisor status changes to every window.
            let handle = app.handle().clone();
            let mut rx = sup.subscribe();
            tauri::async_runtime::spawn(async move {
                while rx.changed().await.is_ok() {
                    let s = rx.borrow_and_update().clone();
                    let _ = handle.emit("inference-status", s);
                }
            });

            match spec {
                Some(spec) => {
                    tracing::info!("deployed model {}; preloading", spec.model_id);
                    show_main(app.handle());
                    tauri::async_runtime::spawn(async move {
                        sup.configure(spec).await;
                        if let Err(e) = sup.ensure_ready().await {
                            tracing::error!("preload failed: {e}");
                        }
                    });
                }
                None => {
                    tracing::info!("not deployed; opening wizard");
                    if let Some(w) = app.get_webview_window("wizard") {
                        w.show()?;
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                match window.label() {
                    // Re-deploy opened from the main window: just go back to it.
                    "wizard" if app.state::<AppState>().settings.get().deployed => {
                        api.prevent_close();
                        show_main(app);
                    }
                    // No tray yet (M5): closing a top-level window quits.
                    _ => app.exit(0),
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            window_effect_info,
            get_settings,
            inference_status,
            translate,
            translate_cancel,
            open_logs_dir,
            deploy::wizard::deploy_detect,
            deploy::wizard::deploy_models,
            deploy::wizard::disk_free_bytes,
            deploy::wizard::download_model,
            deploy::wizard::download_control,
            deploy::wizard::deploy_launch,
            deploy::wizard::deploy_finish,
            deploy::wizard::open_wizard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
