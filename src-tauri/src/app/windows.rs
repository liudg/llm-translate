//! Window helpers: theme, backdrop effects, showing the main and mini windows.

use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WebviewWindowBuilder};

use super::settings::Theme;

/// Every window label in tauri.conf.json.
const LABELS: [&str; 5] = ["main", "wizard", "mini", "popup", "shot"];

/// Win11 (build >= 22000) -> Mica, Win10 -> Acrylic.
pub fn pick_effect(build: u32) -> (Effect, &'static str) {
    if build >= 22000 { (Effect::Mica, "Mica") } else { (Effect::Acrylic, "Acrylic") }
}

/// Create the windows from tauri.conf.json (all `"create": false`) with the
/// backdrop for this OS build; Called at the end of
/// setup so no page can invoke a command before its state is managed.
pub fn create_all(app: &AppHandle, build: u32) -> tauri::Result<()> {
    let (effect, _) = pick_effect(build);
    for cfg in &app.config().app.windows {
        let builder = WebviewWindowBuilder::from_config(app, cfg)?;
        // The screenshot result window must show the image exactly, with
        // see-through margins: no backdrop.
        let builder = if cfg.label == crate::capture::LABEL {
            builder
        } else {
            builder.effects(EffectsBuilder::new().effect(effect).build())
        };
        builder.build()?;
    }
    Ok(())
}

/// Native window theme (title bar, Mica tint). The page itself follows the
/// `settings-changed` event, see `src/lib/theme.ts`.
pub fn apply_theme(app: &AppHandle, theme: Theme) {
    let t = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };
    for label in LABELS {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.set_theme(t);
        }
    }
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

/// Put the mini window on the monitor under the cursor: horizontally centred,
/// its top edge at a quarter of the work area so the result grows downward
/// without moving the input box.
fn place_mini(win: &WebviewWindow) {
    let Ok(cursor) = win.cursor_position() else { return };
    let monitor = win
        .monitor_from_point(cursor.x, cursor.y)
        .ok()
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let (Some(m), Ok(size)) = (monitor, win.outer_size()) else { return };
    let area = m.work_area();
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 / 4;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

pub fn show_mini(app: &AppHandle) {
    let Some(win) = app.get_webview_window("mini") else { return };
    place_mini(&win);
    let _ = win.show();
    let _ = win.set_focus();
    // The page focuses and selects its input.
    let _ = win.emit("mini-shown", ());
}

pub fn toggle_mini(app: &AppHandle) {
    let Some(win) = app.get_webview_window("mini") else { return };
    if win.is_visible().unwrap_or(false) && win.is_focused().unwrap_or(false) {
        let _ = win.hide();
    } else {
        show_mini(app);
    }
}
