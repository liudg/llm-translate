//! Window helpers: theme, backdrop effects, showing the main and mini windows,
//! remembering the main window's placement.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, SW_HIDE, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
};

use super::paths;
use super::settings::Theme;

/// Every window label in tauri.conf.json.
const LABELS: [&str; 5] = ["main", "wizard", "mini", "popup", "shot"];
/// Windows that use the OS backdrop (Mica on Win11) or Fluent solid colour (Win10).
const BACKDROP_LABELS: [&str; 4] = ["main", "wizard", "mini", "popup"];

/// OS build cached at startup so lazy-created windows pick the same backdrop.
static OS_BUILD: AtomicU32 = AtomicU32::new(0);

/// Fluent solid backing colours for Win10 (must match `src/index.css`).
const SOLID_LIGHT: Color = Color(0xf3, 0xf3, 0xf3, 0xff);
const SOLID_DARK: Color = Color(0x20, 0x20, 0x20, 0xff);

/// Win11 (build >= 22000) -> Mica; Win10 has no Mica and its Acrylic lags
/// badly on drag/resize, so it gets a solid Fluent fill (DESIGN 2 / 8).
pub fn uses_mica(build: u32) -> bool {
    build >= 22000
}

/// Whether Windows is currently set to dark app mode (Win10 1607+ / Win11).
fn system_uses_dark_mode() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_u32("AppsUseLightTheme"))
        .is_ok_and(|v| v == 0)
}

fn solid_color(theme: Theme) -> Color {
    let dark = match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => system_uses_dark_mode(),
    };
    if dark { SOLID_DARK } else { SOLID_LIGHT }
}

fn tauri_theme(theme: Theme) -> Option<tauri::Theme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    }
}

/// Return an existing window or create it from `tauri.conf.json` with the
/// backdrop and theme for this OS build.
pub fn ensure_window(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    if let Some(win) = app.get_webview_window(label) {
        return Ok(win);
    }
    let cfg = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .ok_or_else(|| tauri::Error::WebviewNotFound)?;
    let theme = app.state::<crate::state::AppState>().settings.get().theme;
    let builder = WebviewWindowBuilder::from_config(app, cfg)?;
    // The screenshot result window must show the image exactly, with
    // see-through margins: no backdrop.
    let builder = if cfg.label == crate::capture::LABEL {
        builder
    } else if uses_mica(OS_BUILD.load(Ordering::Relaxed)) {
        builder.effects(EffectsBuilder::new().effect(Effect::Mica).build())
    } else {
        // Win10 solid fallback: disable transparency for crisp shadows and
        // fast composition, set the WebView2 backing colour, and flag the
        // DOM before first paint so index.css paints the matching solid fill.
        builder
            .transparent(false)
            .background_color(solid_color(theme))
            .initialization_script("document.documentElement.dataset.effect='solid';")
    };
    let win = match builder.build() {
        Ok(w) => w,
        // Another thread created it meanwhile (the screenshot session warms
        // `shot` up in the background while also needing it): use that one.
        Err(e) => return app.get_webview_window(label).ok_or(e),
    };
    let _ = win.set_theme(tauri_theme(theme));
    if label == "main" {
        restore_main_placement(app);
    }
    Ok(win)
}

/// Create only the windows needed at startup (all `"create": false` in
/// tauri.conf.json). Deployed runs pre-create `main`, `mini`, `popup` so input
/// and selection stay instant while skipping the unused `wizard` and `shot`
/// WebViews (saving ~130 MB WebView2 RAM and ~200 ms startup). First-run
/// startup only creates `wizard`.
pub fn create_startup(app: &AppHandle, build: u32, deployed: bool) -> tauri::Result<()> {
    OS_BUILD.store(build, Ordering::Relaxed);
    let initial: &[&str] = if deployed { &["main", "mini", "popup"] } else { &["wizard"] };
    for label in initial {
        ensure_window(app, label)?;
    }
    Ok(())
}

/// Native window theme (title bar, Mica tint, and Win10 solid backing colour).
/// The page itself follows the `settings-changed` event, see `src/lib/theme.ts`.
pub fn apply_theme(app: &AppHandle, theme: Theme) {
    let t = tauri_theme(theme);
    let solid = (!uses_mica(OS_BUILD.load(Ordering::Relaxed))).then(|| solid_color(theme));
    for label in LABELS {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.set_theme(t);
            if let Some(bg) = solid
                && BACKDROP_LABELS.contains(&label)
            {
                let _ = win.set_background_color(Some(bg));
            }
        }
    }
}

pub fn show_main(app: &AppHandle) {
    if let Ok(w) = ensure_window(app, "main") {
        let _ = w.show();
        let _ = w.unminimize();
        if RESTORE_MAXIMIZED.swap(false, Ordering::Relaxed) {
            let _ = w.maximize();
        }
        let _ = w.set_focus();
    }
    super::updater::on_main_shown(app);
    // Pre-warm mini & popup after the first deployment finishes.
    let _ = ensure_window(app, "mini");
    let _ = ensure_window(app, "popup");
    // Hide the wizard immediately and destroy it asynchronously (outside its
    // own IPC / CloseRequested handler) so its WebView2 renderer is freed.
    if let Some(w) = app.get_webview_window("wizard") {
        // Every way out of the wizard (close button, finish, tray menu) lands
        // here: pause its downloads and bring the saved model back.
        let was_visible = w.is_visible().unwrap_or(false);
        let _ = w.hide();
        if was_visible {
            crate::deploy::wizard::on_wizard_closed(app);
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(w) = app.get_webview_window("wizard")
                && !w.is_visible().unwrap_or(false)
            {
                let _ = w.destroy();
            }
        });
    }
}

/// Main window placement in `%APPDATA%\LLMTranslate\window.json`: the normal
/// (restored) rectangle in workspace coordinates, as `GetWindowPlacement`
/// reports it, and whether it was maximized. Kept out of settings.json so
/// moving the window does not broadcast `settings-changed`.
#[derive(Debug, Serialize, Deserialize)]
struct Placement {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    maximized: bool,
}

/// Maximize on the first `show_main` after restoring a maximized placement
/// (the window is still hidden when the placement is applied).
static RESTORE_MAXIMIZED: AtomicBool = AtomicBool::new(false);
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn placement_file() -> std::path::PathBuf {
    paths::config_dir().join("window.json")
}

fn main_hwnd(app: &AppHandle) -> Option<(WebviewWindow, HWND)> {
    let w = app.get_webview_window("main")?;
    let h = w.hwnd().ok()?;
    Some((w, HWND(h.0 as *mut _)))
}

/// Save the main window's placement if it is visible (a hidden window's
/// placement is whatever was saved when it was hidden).
pub fn save_main_placement(app: &AppHandle) {
    let Some((w, hwnd)) = main_hwnd(app) else { return };
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_err() {
        return;
    }
    let r = wp.rcNormalPosition;
    let maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32 || (wp.flags.0 & WPF_RESTORETOMAXIMIZED.0) != 0;
    let p = Placement { left: r.left, top: r.top, right: r.right, bottom: r.bottom, maximized };
    let path = placement_file();
    let _ = std::fs::create_dir_all(paths::config_dir());
    if let Err(e) = serde_json::to_vec(&p).map_err(anyhow::Error::from).and_then(|d| Ok(std::fs::write(&path, d)?)) {
        tracing::warn!("saving window placement failed: {e}");
    }
}

/// Save the placement shortly after the last move / resize.
pub fn schedule_save_main_placement(app: &AppHandle) {
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        if SAVE_GENERATION.load(Ordering::Relaxed) == generation {
            save_main_placement(&app);
        }
    });
}

/// Apply the saved placement to the (still hidden) main window. Skipped when
/// the rectangle is not on any current monitor; the window then stays centred.
pub fn restore_main_placement(app: &AppHandle) {
    let Ok(data) = std::fs::read(placement_file()) else { return };
    let Ok(p) = serde_json::from_slice::<Placement>(&data) else { return };
    let Some((_, hwnd)) = main_hwnd(app) else { return };
    let rect = RECT { left: p.left, top: p.top, right: p.right, bottom: p.bottom };
    if rect.right - rect.left < 200 || rect.bottom - rect.top < 150 {
        return;
    }
    if unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid() {
        tracing::info!("saved window placement is off-screen; centring");
        return;
    }
    let wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: SW_HIDE.0 as u32,
        rcNormalPosition: rect,
        ..Default::default()
    };
    if unsafe { SetWindowPlacement(hwnd, &wp) }.is_ok() {
        RESTORE_MAXIMIZED.store(p.maximized, Ordering::Relaxed);
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
    let Ok(win) = ensure_window(app, "mini") else { return };
    place_mini(&win);
    let _ = win.show();
    let _ = win.set_focus();
    // The page focuses and selects its input.
    let _ = win.emit("mini-shown", ());
}

pub fn toggle_mini(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("mini")
        && win.is_visible().unwrap_or(false)
        && win.is_focused().unwrap_or(false)
    {
        let _ = win.hide();
    } else {
        show_mini(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win10_uses_solid_and_win11_uses_mica() {
        assert!(!uses_mica(17763)); // Win10 1809
        assert!(!uses_mica(19045)); // Win10 22H2
        assert!(!uses_mica(21999));
        assert!(uses_mica(22000)); // Win11 21H2
        assert!(uses_mica(26200)); // Win11 24H2+
    }
}
