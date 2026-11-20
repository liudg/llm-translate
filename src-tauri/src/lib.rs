pub mod app;
pub mod deploy;
pub mod inference;
pub mod selection;
pub mod state;
pub mod translate;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

use app::commands::AppInfo;
use app::hotkey::Hotkeys;
use app::popup::{self, PopupState};
use app::settings::{Backend, Settings, SettingsStore};
use app::{paths, windows};
use deploy::manifest::Manifest;
use inference::supervisor::{Notice, Status, Supervisor};
use selection::Selection;
use state::AppState;
use translate::scheduler::{TranslateEvent, TranslateRequest, Translator};

pub use app::windows::show_main;

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

/// Forward supervisor status and settings changes to every window, and apply
/// the side effects of a settings change (theme, idle release, selection).
fn spawn_forwarders(app: &AppHandle, state: &AppState, selection: Option<Arc<Selection>>) {
    let handle = app.clone();
    let mut rx = state.sup.subscribe();
    tauri::async_runtime::spawn(async move {
        while rx.changed().await.is_ok() {
            let s = rx.borrow_and_update().clone();
            let _ = handle.emit("inference-status", s);
        }
    });

    let handle = app.clone();
    let sup = state.sup.clone();
    let mut rx = state.settings.subscribe();
    let mut theme = state.settings.get().theme;
    let mut sel = state.settings.get().selection;
    tauri::async_runtime::spawn(async move {
        while rx.changed().await.is_ok() {
            let s = rx.borrow_and_update().clone();
            if s.theme != theme {
                theme = s.theme;
                windows::apply_theme(&handle, theme);
            }
            if s.selection != sel {
                sel = s.selection.clone();
                if let Some(selection) = &selection {
                    selection.set_config(&sel);
                }
            }
            sup.set_idle_minutes(s.idle_release_minutes);
            let _ = handle.emit("settings-changed", s);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = app::logging::init();
    tracing::info!("LLM Translate {} starting", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(app::hotkey::plugin())
        .manage(Hotkeys::default())
        .manage(PopupState::default())
        .setup(|app| {
            let state = build_state(app.handle())?;
            let settings = state.settings.get();
            let spec = state.spec_from_settings(&settings);
            let sup = state.sup.clone();
            let handle = app.handle().clone();
            let selection = Selection::start(&settings.selection, move |cap| popup::show_capture(&handle, cap))
                .map(Arc::new)
                .map_err(|e| tracing::error!("selection translation unavailable: {e}"))
                .ok();
            spawn_forwarders(app.handle(), &state, selection);
            app.manage(state);
            app::hotkey::register_defaults(app.handle());

            let build = windows_version::OsVersion::current().build;
            app.manage(AppInfo {
                version: env!("CARGO_PKG_VERSION"),
                os_build: build,
                effect: windows::pick_effect(build).1,
                config_dir: paths::config_dir(),
                logs_dir: paths::logs_dir(),
            });

            // Windows last: their pages call commands that need the state above.
            windows::create_all(app.handle(), build)?;
            windows::apply_theme(app.handle(), settings.theme);

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
            let app = window.app_handle();
            match event {
                // The mini window behaves like a flyout: it hides on focus loss.
                WindowEvent::Focused(false) if window.label() == "mini" => {
                    let _ = window.hide();
                }
                // The selection popup closes on focus loss unless pinned.
                WindowEvent::Focused(false)
                    if window.label() == popup::LABEL && !app.state::<PopupState>().pinned() =>
                {
                    popup::hide(app);
                }
                WindowEvent::CloseRequested { api, .. } => match window.label() {
                    "mini" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    popup::LABEL => {
                        api.prevent_close();
                        popup::hide(app);
                    }
                    // Re-deploy opened from the main window: just go back to it.
                    "wizard" if app.state::<AppState>().settings.get().deployed => {
                        api.prevent_close();
                        show_main(app);
                    }
                    // No tray yet (M5): closing a top-level window quits.
                    _ => app.exit(0),
                },
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            inference_status,
            translate,
            translate_cancel,
            app::commands::app_info,
            app::commands::set_theme,
            app::commands::set_idle_release,
            app::commands::set_inference,
            app::commands::hardware_info,
            app::commands::open_logs_dir,
            app::commands::open_models_dir,
            app::commands::open_mini,
            app::commands::set_selection,
            app::commands::window_processes,
            app::popup::popup_set_height,
            app::popup::popup_set_pinned,
            app::popup::popup_close,
            app::popup::popup_current,
            app::hotkey::hotkey_status,
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
