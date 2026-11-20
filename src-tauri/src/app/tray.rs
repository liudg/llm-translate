//! Tray icon (DESIGN 5.7): left click opens the main window; the menu has
//! input translation / screenshot translation / selection switch / settings /
//! quit. The icon turns grey while selection translation is off.

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use super::hotkey::{self, HotkeyInfo};
use super::windows;
use crate::state::AppState;

const ID: &str = "main";
const NAME: &str = "LLM Translate";

struct TrayMenu {
    mini: MenuItem<Wry>,
    shot: MenuItem<Wry>,
    selection: CheckMenuItem<Wry>,
    icon: Image<'static>,
    grey: Image<'static>,
}

/// Desaturated, semi-transparent copy of the app icon.
fn grey_icon(icon: &Image<'_>) -> Image<'static> {
    let mut rgba = icon.rgba().to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        let l = (0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32) as u8;
        px[0] = l;
        px[1] = l;
        px[2] = l;
        px[3] = (px[3] as f32 * 0.6) as u8;
    }
    Image::new_owned(rgba, icon.width(), icon.height())
}

/// Menu label with the shortcut right-aligned (a tab in a Win32 menu label).
fn label(text: &str, keys: Option<&str>) -> String {
    match keys {
        Some(k) => format!("{text}\t{k}"),
        None => text.to_string(),
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let settings = app.state::<AppState>().settings.get();
    let icon = app.default_window_icon().cloned().map(|i| i.to_owned()).expect("bundle has an icon");
    let grey = grey_icon(&icon);
    let hk = settings.hotkeys;

    let mini = MenuItem::with_id(app, "mini", label("输入翻译", hk.mini.as_deref()), true, None::<&str>)?;
    let shot = MenuItem::with_id(app, "shot", label("截图翻译", hk.shot.as_deref()), true, None::<&str>)?;
    let selection =
        CheckMenuItem::with_id(app, "selection", "划词翻译", true, settings.selection.enabled, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &mini,
            &shot,
            &selection,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?,
        ],
    )?;

    let enabled = settings.selection.enabled;
    TrayIconBuilder::with_id(ID)
        .icon(if enabled { icon.clone() } else { grey.clone() })
        .tooltip(tooltip(enabled))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "mini" => {
                if deployed(app) {
                    windows::show_mini(app);
                } else {
                    open_primary(app);
                }
            }
            "shot" => crate::capture::start(app, false),
            "selection" => hotkey::toggle_selection(app),
            "settings" => {
                if deployed(app) {
                    windows::show_main(app);
                    let _ = app.emit_to("main", "navigate", "settings");
                } else {
                    open_primary(app);
                }
            }
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open_primary(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayMenu { mini, shot, selection, icon, grey });
    Ok(())
}

fn tooltip(selection_enabled: bool) -> String {
    if selection_enabled { NAME.to_string() } else { format!("{NAME}（划词翻译已关闭）") }
}

fn deployed(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.get().deployed
}

/// Main window when deployed, otherwise the deployment wizard.
pub fn open_primary(app: &AppHandle) {
    if deployed(app) {
        windows::show_main(app);
    } else if let Some(w) = app.get_webview_window("wizard") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn quit(app: &AppHandle) {
    windows::save_main_placement(app);
    app.exit(0);
}

/// Follow the selection switch: check mark, icon and tooltip.
pub fn sync_selection(app: &AppHandle, enabled: bool) {
    let (Some(menu), Some(tray)) = (app.try_state::<TrayMenu>(), app.tray_by_id(ID)) else { return };
    let _ = menu.selection.set_checked(enabled);
    let _ = tray.set_icon(Some(if enabled { menu.icon.clone() } else { menu.grey.clone() }));
    let _ = tray.set_tooltip(Some(tooltip(enabled)));
}

/// Show the current shortcuts next to the menu items.
pub fn update_hotkeys(app: &AppHandle, status: &[HotkeyInfo]) {
    let Some(menu) = app.try_state::<TrayMenu>() else { return };
    let keys = |id: &str| status.iter().find(|h| h.id == id && h.registered).and_then(|h| h.keys.clone());
    let _ = menu.mini.set_text(label("输入翻译", keys("mini").as_deref()));
    let _ = menu.shot.set_text(label("截图翻译", keys("shot").as_deref()));
}
