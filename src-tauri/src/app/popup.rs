//! Selection result popup (DESIGN 5.5): placed next to the selection, grows
//! away from it as the translation streams in, hides on focus loss unless
//! pinned. A new capture while pinned reuses the popup where it is.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

use crate::selection::{Capture, ScreenRect};
use crate::state::AppState;

pub const LABEL: &str = "popup";
/// Logical width; the page reports its height.
const WIDTH: f64 = 440.0;
/// Room the popup may need below the selection before it opens above instead.
const MAX_HEIGHT: f64 = 420.0;
const INITIAL_HEIGHT: f64 = 120.0;

fn unsupported_hint(app: &AppHandle) -> String {
    match app.state::<AppState>().settings.get().hotkeys.shot {
        Some(keys) if !keys.is_empty() => format!("该应用不支持划词，可使用截图翻译（{keys}）"),
        _ => "该应用不支持划词，可使用截图翻译".to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionPayload {
    /// Increases with every capture.
    pub id: u64,
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    pinned: bool,
    /// Opened above the selection: height changes keep the bottom edge.
    above: bool,
    height: f64,
    last: Option<SelectionPayload>,
}

#[derive(Default)]
pub struct PopupState(Mutex<Inner>);

impl PopupState {
    pub fn pinned(&self) -> bool {
        self.0.lock().unwrap().pinned
    }
}

/// Position for a fresh popup below (or above) the selection.
fn place(win: &WebviewWindow, anchor: ScreenRect, height: f64) -> bool {
    let cx = (anchor.left + anchor.right) as f64 / 2.0;
    let cy = (anchor.top + anchor.bottom) as f64 / 2.0;
    let Some(m) = win.monitor_from_point(cx, cy).ok().flatten().or_else(|| win.primary_monitor().ok().flatten())
    else {
        return false;
    };
    let sf = m.scale_factor();
    let area = m.work_area();
    let (al, at) = (area.position.x as f64, area.position.y as f64);
    let (ar, ab) = (al + area.size.width as f64, at + area.size.height as f64);
    // The gesture points sit inside a text line; clear about one line.
    let line = 14.0 * sf;
    let gap = 6.0 * sf;
    let sel_bottom = anchor.bottom as f64 + line;
    let sel_top = anchor.top as f64 - line;
    let (w, h) = (WIDTH * sf, height * sf);
    let below_space = ab - sel_bottom - gap;
    let above = below_space < MAX_HEIGHT * sf && sel_top - gap - at > below_space;
    let y = if above { sel_top - gap - h } else { sel_bottom + gap };
    let y = y.clamp(at, (ab - h).max(at));
    let x = (anchor.left as f64 - 12.0 * sf).clamp(al, (ar - w).max(al));
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = win.set_size(PhysicalSize::new(w.round() as u32, h.round() as u32));
    above
}

fn focus(win: &WebviewWindow) {
    let _ = win.set_focus();
    if let Ok(h) = win.hwnd() {
        let hwnd = windows::Win32::Foundation::HWND(h.0 as *mut _);
        if !crate::selection::input::force_foreground(hwnd) {
            tracing::warn!("could not bring the selection popup to the foreground");
        }
    }
}

/// Show a capture result (called on the capture thread).
pub fn show_capture(app: &AppHandle, cap: Capture) {
    let Ok(win) = crate::app::windows::ensure_window(app, LABEL) else { return };
    let state = app.state::<PopupState>();
    let visible = win.is_visible().unwrap_or(false);
    let payload = {
        let mut inner = state.0.lock().unwrap();
        inner.seq += 1;
        let payload = SelectionPayload {
            id: inner.seq,
            error: cap.text.is_none().then(|| unsupported_hint(app)),
            text: cap.text,
        };
        inner.last = Some(payload.clone());
        let keep_place = inner.pinned && visible;
        if !keep_place {
            inner.pinned = false;
            if inner.height <= 0.0 {
                inner.height = INITIAL_HEIGHT;
            }
            inner.above = place(&win, cap.anchor, inner.height);
        }
        payload
    };
    let _ = app.emit_to(LABEL, "selection-captured", payload);
    if !visible {
        let _ = win.show();
        focus(&win);
    } else if !state.pinned() {
        focus(&win);
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        app.state::<PopupState>().0.lock().unwrap().pinned = false;
        let _ = win.hide();
        let _ = app.emit_to(LABEL, "popup-hidden", ());
    }
}

/// The page's content height changed (logical px).
#[tauri::command]
pub fn popup_set_height(app: AppHandle, state: State<'_, PopupState>, height: f64) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    let height = height.clamp(40.0, 800.0);
    let above = {
        let mut inner = state.0.lock().unwrap();
        if (inner.height - height).abs() < 0.5 {
            return;
        }
        inner.height = height;
        inner.above
    };
    let (Ok(pos), Ok(size), Ok(sf)) = (win.outer_position(), win.outer_size(), win.scale_factor()) else { return };
    let h = (height * sf).round() as i32;
    let mut y = if above { pos.y + size.height as i32 - h } else { pos.y };
    if let Ok(Some(m)) = win.current_monitor() {
        let a = m.work_area();
        let (top, bottom) = (a.position.y, a.position.y + a.size.height as i32);
        y = y.clamp(top, (bottom - h).max(top));
    }
    let _ = win.set_size(PhysicalSize::new(size.width, h as u32));
    if y != pos.y {
        let _ = win.set_position(PhysicalPosition::new(pos.x, y));
    }
}

#[tauri::command]
pub fn popup_set_pinned(state: State<'_, PopupState>, pinned: bool) {
    state.0.lock().unwrap().pinned = pinned;
}

#[tauri::command]
pub fn popup_close(app: AppHandle) {
    hide(&app);
}

/// Latest capture, for a page that loaded after it was emitted.
#[tauri::command]
pub fn popup_current(state: State<'_, PopupState>) -> Option<SelectionPayload> {
    state.0.lock().unwrap().last.clone()
}
