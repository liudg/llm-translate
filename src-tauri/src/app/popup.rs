//! Selection result popup (DESIGN 5.5): placed next to the selection, grows
//! away from it as the translation streams in, hides on focus loss unless
//! pinned. A new capture while pinned reuses the popup where it is.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

use crate::app::windows::text_scale;
use crate::selection::clipboard::CopyError;
use crate::selection::{Capture, ScreenRect};
use crate::state::AppState;

pub const LABEL: &str = "popup";
/// Logical width; the page reports its height.
const WIDTH: f64 = 440.0;
/// Room the popup may need below the selection before it opens above instead.
const MAX_HEIGHT: f64 = 420.0;
const INITIAL_HEIGHT: f64 = 120.0;

/// Message for a failed capture, and whether the app itself is to blame (so
/// the popup offers to stop showing the icon there).
fn failure_hint(app: &AppHandle, err: CopyError) -> (String, bool) {
    match err {
        CopyError::Busy => ("剪贴板正被其他程序占用，请稍后重试".to_string(), false),
        CopyError::ModifierHeld => ("请先松开 Alt 键再点击翻译图标".to_string(), false),
        CopyError::Unchanged | CopyError::Empty => {
            let hint = match app.state::<AppState>().settings.get().hotkeys.shot {
                Some(keys) if !keys.is_empty() => format!("没有读到选中的文字，这个应用可能不支持划词，可改用截图翻译（{keys}）"),
                _ => "没有读到选中的文字，这个应用可能不支持划词，可改用截图翻译".to_string(),
            };
            (hint, true)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionPayload {
    /// Increases with every capture.
    pub id: u64,
    pub text: Option<String>,
    pub error: Option<String>,
    /// Executable the capture failed in, when the popup may offer to add it
    /// to the blacklist.
    pub block_app: Option<String>,
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
    // Page CSS pixels -> physical pixels.
    let sf = m.scale_factor() * text_scale();
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
    let (text, error, block_app) = match cap.text {
        Ok(t) => (Some(t), None, None),
        Err(e) => {
            let (hint, blame_app) = failure_hint(app, e);
            (None, Some(hint), cap.app.filter(|_| blame_app))
        }
    };
    // Window calls below wait for the main thread, whose focus handler takes
    // this lock: never hold it across them.
    let (payload, place_height) = {
        let mut inner = state.0.lock().unwrap();
        inner.seq += 1;
        let payload = SelectionPayload { id: inner.seq, error, text, block_app };
        inner.last = Some(payload.clone());
        let keep_place = inner.pinned && visible;
        let place_height = (!keep_place).then(|| {
            inner.pinned = false;
            if inner.height <= 0.0 {
                inner.height = INITIAL_HEIGHT;
            }
            inner.height
        });
        (payload, place_height)
    };
    if let Some(height) = place_height {
        let above = place(&win, cap.anchor, height);
        state.0.lock().unwrap().above = above;
    }
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
        // The page clears itself on `popup-hidden` and reports its smaller
        // height, so the next capture does not open at the old result's size.
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
    let (Ok(pos), Ok(outer), Ok(inner), Ok(sf)) =
        (win.outer_position(), win.outer_size(), win.inner_size(), win.scale_factor())
    else {
        return;
    };
    let sf = sf * text_scale();
    // `set_size` takes the inner size; positions are of the outer frame,
    // which may be larger (invisible resize borders).
    let h = (height * sf).round() as i32;
    let frame = outer.height as i32 - inner.height as i32;
    let mut y = if above { pos.y + outer.height as i32 - (h + frame) } else { pos.y };
    if let Ok(Some(m)) = win.current_monitor() {
        let a = m.work_area();
        let (top, bottom) = (a.position.y, a.position.y + a.size.height as i32);
        y = y.clamp(top, (bottom - h - frame).max(top));
    }
    let _ = win.set_size(PhysicalSize::new(inner.width, h as u32));
    if y != pos.y {
        let _ = win.set_position(PhysicalPosition::new(pos.x, y));
    }
}

#[tauri::command]
pub fn popup_set_pinned(state: State<'_, PopupState>, pinned: bool) {
    state.0.lock().unwrap().pinned = pinned;
}

/// Add an executable to the selection blacklist and close the popup.
#[tauri::command]
pub fn popup_block_app(app: AppHandle, state: State<'_, AppState>, exe: String) -> Result<(), String> {
    state.settings.save(|s| {
        if !s.selection.blacklist.iter().any(|n| n.eq_ignore_ascii_case(&exe)) {
            s.selection.blacklist.push(exe);
        }
    })?;
    hide(&app);
    Ok(())
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
