pub mod app;
pub mod capture;
pub mod deploy;
pub mod inference;
pub mod selection;
pub mod state;
pub mod translate;
pub mod tts;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent};

use app::commands::AppInfo;
use app::hotkey::Hotkeys;
use app::popup::{self, PopupState};
use app::settings::{Backend, Settings, SettingsStore};
use app::{paths, windows};
use capture::CaptureState;
use deploy::manifest::Manifest;
use inference::supervisor::{Notice, Status, Supervisor};
use selection::{Selection, SelectionEvent};
use state::AppState;
use translate::scheduler::{BatchEvent, BatchRequest, TranslateEvent, TranslateRequest, Translator};

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
async fn translate(
    window: WebviewWindow,
    state: State<'_, AppState>,
    req: TranslateRequest,
    on_event: Channel<TranslateEvent>,
) -> Result<(), ()> {
    let translator = state.translator.clone();
    translator
        .translate(window.label(), req, move |ev| {
            let _ = on_event.send(ev);
        })
        .await;
    Ok(())
}

#[tauri::command]
async fn translate_batch(
    window: WebviewWindow,
    state: State<'_, AppState>,
    req: BatchRequest,
    on_event: Channel<BatchEvent>,
) -> Result<(), ()> {
    let translator = state.translator.clone();
    translator
        .translate_batch(window.label(), req, move |ev| {
            let _ = on_event.send(ev);
        })
        .await;
    Ok(())
}

#[tauri::command]
fn translate_cancel(window: WebviewWindow, state: State<'_, AppState>) {
    state.translator.cancel_for(window.label());
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
    let tts = Arc::new(tts::TtsService::new());
    Ok(AppState {
        settings,
        manifest,
        llama_exe,
        sup,
        translator,
        tts,
        downloads: Mutex::new(HashMap::new()),
        http: deploy::downloader::client(),
        deploy: tokio::sync::Mutex::new(()),
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
                if s.selection.enabled != sel.enabled {
                    app::tray::sync_selection(&handle, s.selection.enabled);
                }
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

/// The first time the main window is closed, tell the user the app keeps
/// running in the tray.
fn tray_hint_once(app: &AppHandle) {
    use tauri_plugin_notification::NotificationExt;
    let state = app.state::<AppState>();
    if state.settings.get().tray_hint_shown {
        return;
    }
    if let Err(e) = app
        .notification()
        .builder()
        .title("LLM Translate 仍在运行")
        .body("已最小化到系统托盘，快捷键和划词翻译照常可用。可在托盘图标的菜单中退出。")
        .show()
    {
        tracing::warn!("tray notification failed: {e}");
    }
    let _ = state.settings.update(|s| s.tray_hint_shown = true);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started = std::time::Instant::now();
    let _log_guard = app::logging::init();
    tracing::info!("LLM Translate {} starting", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        // Must come first: a second launch only activates this instance.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tracing::info!("second instance started with {args:?}; activating");
            app::tray::open_primary(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(app::hotkey::plugin())
        .manage(Hotkeys::default())
        .manage(PopupState::default())
        .manage(app::updater::UpdaterState::default())
        .setup(move |app| {
            let state = build_state(app.handle())?;
            let settings = state.settings.get();
            if settings.deployed || settings.models_dir.exists() {
                app::settings::sync_models_dir_registry(&settings.models_dir);
            }
            let spec = state.spec_from_settings(&settings);
            let handle = app.handle().clone();
            let selection = Selection::start(&settings.selection, move |ev| match ev {
                SelectionEvent::Captured(cap) => {
                    if app::tray::deployed(&handle) {
                        popup::show_capture(&handle, cap);
                    } else {
                        app::tray::open_primary(&handle);
                    }
                }
                SelectionEvent::ExcludeApp(exe) => {
                    tracing::info!("excluding {exe} from selection via icon menu");
                    if let Err(e) = handle.state::<AppState>().settings.update(|s| s.selection.exclude_app(&exe)) {
                        tracing::error!("saving selection exclusion failed: {e}");
                    }
                }
                SelectionEvent::DisableSelection => {
                    tracing::info!("disabling selection via icon menu");
                    if let Err(e) = handle.state::<AppState>().settings.update(|s| s.selection.enabled = false) {
                        tracing::error!("saving selection disable failed: {e}");
                    }
                }
            })
                .map(Arc::new)
                .map_err(|e| tracing::error!("selection translation unavailable: {e}"))
                .ok();
            spawn_forwarders(app.handle(), &state, selection);
            app.manage(CaptureState::new(app.path().resource_dir()?.join("ocr")));
            app.manage(state);
            app::tray::create(app.handle())?;
            app::hotkey::register_saved(app.handle());

            let build = windows_version::OsVersion::current().build;
            app.manage(AppInfo {
                version: env!("CARGO_PKG_VERSION"),
                os_build: build,
                effect: if windows::uses_mica(build) { "Mica" } else { "纯色" },
                config_dir: paths::config_dir(),
                logs_dir: paths::logs_dir(),
            });

            // Windows last: their pages call commands that need the state above.
            windows::create_startup(app.handle(), build, spec.is_some())?;

            // Started with Windows: stay in the tray.
            let minimized = std::env::args().any(|a| a == app::autostart::ARG);
            tracing::info!("startup ready in {} ms (minimized={minimized})", started.elapsed().as_millis());
            match spec {
                Some(spec) => {
                    tracing::info!("deployed model {}; preloading{}", spec.model_id, if minimized { " (tray only)" } else { "" });
                    if !minimized {
                        show_main(app.handle());
                    }
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = handle.state::<AppState>();
                        // Settings read under the lock: a switch that came first wins.
                        let configured = {
                            let _deploy = state.deploy.lock().await;
                            state.sup.configure_with(|| state.spec_from_settings(&state.settings.get())).await
                        };
                        if configured && let Err(e) = state.sup.ensure_ready().await {
                            tracing::error!("preload failed: {e}");
                        }
                    });
                }
                None => {
                    tracing::info!("not deployed; opening wizard");
                    if let Ok(w) = windows::ensure_window(app.handle(), "wizard") {
                        w.show()?;
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::Moved(_) | WindowEvent::Resized(_) if window.label() == "main" => {
                    windows::schedule_save_main_placement(app);
                }
                // The mini window behaves like a flyout: it hides on focus loss.
                WindowEvent::Focused(false) if window.label() == "mini" => {
                    let _ = window.hide();
                    let state = app.state::<AppState>();
                    state.translator.cancel_for("mini");
                    state.tts.stop_for(app, "mini");
                    let _ = window.emit("mini-hidden", ());
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
                        let state = app.state::<AppState>();
                        state.translator.cancel_for("mini");
                        state.tts.stop_for(app, "mini");
                        let _ = window.emit("mini-hidden", ());
                    }
                    popup::LABEL => {
                        api.prevent_close();
                        popup::hide(app);
                    }
                    capture::LABEL => {
                        api.prevent_close();
                        capture::hide(app);
                    }
                    // The main window hides to the tray; quit from the tray menu.
                    "main" => {
                        api.prevent_close();
                        app.state::<AppState>().tts.stop_for(app, "main");
                        windows::save_main_placement(app);
                        let _ = window.hide();
                        tray_hint_once(app);
                    }
                    // Re-deploy opened from the main window: close wizard to free its WebView and return to main.
                    "wizard" if app.state::<AppState>().settings.get().deployed => {
                        api.prevent_close();
                        windows::off_main_thread(app, show_main);
                    }
                    // Closing the wizard before the first deployment quits.
                    _ => app.exit(0),
                },
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            inference_status,
            translate,
            translate_batch,
            translate_cancel,
            app::commands::app_info,
            app::commands::set_theme,
            app::commands::sync_system_theme,
            app::commands::set_idle_release,
            app::commands::set_inference,
            app::commands::hardware_info,
            app::commands::open_logs_dir,
            app::commands::open_models_dir,
            app::commands::open_mini,
            app::commands::open_main_with_text,
            app::commands::set_selection,
            app::commands::set_tts,
            app::commands::window_processes,
            tts::tts_synthesize,
            tts::tts_stop,
            tts::tts_voices,
            app::popup::popup_set_height,
            app::popup::popup_set_pinned,
            app::popup::popup_close,
            app::popup::popup_block_app,
            app::popup::popup_current,
            app::hotkey::hotkey_status,
            app::hotkey::hotkey_pause,
            app::hotkey::hotkey_set,
            app::commands::autostart_enabled,
            app::commands::set_autostart,
            app::updater::update_status,
            app::updater::update_check,
            app::updater::update_install,
            app::updater::update_ignore,
            capture::shot_start,
            capture::shot_current,
            capture::shot_image,
            capture::shot_ocr,
            capture::shot_shown,
            capture::shot_close,
            capture::shot_copy_image,
            deploy::wizard::deploy_detect,
            deploy::wizard::deploy_models,
            deploy::wizard::disk_free_bytes,
            deploy::wizard::download_model,
            deploy::wizard::download_control,
            deploy::wizard::deploy_launch,
            deploy::wizard::deploy_finish,
            deploy::wizard::model_switch,
            deploy::wizard::model_delete,
            deploy::wizard::open_wizard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
