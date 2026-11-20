use serde::Serialize;
use tauri::window::{Effect, EffectsBuilder};
use tauri::Manager;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EffectInfo {
    os_build: u32,
    effect: &'static str,
}

/// Win11 (build >= 22000) -> Mica, Win10 -> Acrylic.
fn pick_effect(build: u32) -> (Effect, &'static str) {
    if build >= 22000 {
        (Effect::Mica, "Mica")
    } else {
        (Effect::Acrylic, "Acrylic")
    }
}

#[tauri::command]
fn window_effect_info(state: tauri::State<EffectInfo>) -> EffectInfo {
    state.inner().clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let build = windows_version::OsVersion::current().build;
            let (effect, name) = pick_effect(build);
            let win = app.get_webview_window("main").expect("main window");
            let applied = win
                .set_effects(EffectsBuilder::new().effect(effect).build())
                .map(|_| name)
                .unwrap_or("none");
            app.manage(EffectInfo { os_build: build, effect: applied });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![window_effect_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
