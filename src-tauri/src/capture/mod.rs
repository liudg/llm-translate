//! Screenshot translation (DESIGN 5.6): freeze the monitors, let the user
//! select a region on a native overlay, then show the region in the `shot`
//! result window at exactly the same physical position. OCR starts as soon as
//! the region is known; the page asks for the paragraphs and translates them
//! itself through `translate_batch`.

pub mod layout;
pub mod ocr;
pub mod overlay;
pub mod screen;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use tokio::sync::watch;
use windows::Win32::Foundation::{HWND, RECT};

use layout::Paragraph;
use ocr::Ocr;
use overlay::Overlay;

pub const LABEL: &str = "shot";
/// Toolbar geometry in CSS pixels (must match Shot.tsx).
const BAR_GAP: f64 = 4.0;
const BAR_HEIGHT: f64 = 40.0;
const BAR_MIN_WIDTH: f64 = 400.0;
const BAR_MAX_WIDTH: f64 = 552.0;
/// Transparent margin around the floating toolbar so its drop shadow and border
/// are never clipped by the frameless window bounds.
const SHADOW_PAD_X: f64 = 16.0;
const SHADOW_PAD_Y: f64 = 18.0;
/// Closes the overlay if the page never reports that it has drawn.
const OVERLAY_FALLBACK: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolbarPlace {
    Below,
    Above,
    /// No room outside the image: drawn over its bottom edge.
    Inside,
}

/// Sent with `shot-new`. Sizes and offsets are physical pixels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotPayload {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// Image position inside the window (the toolbar or its shadow pad may extend beyond it).
    pub image_x: u32,
    pub image_y: u32,
    /// Toolbar track inside the window (physical pixels).
    pub bar_x: u32,
    pub bar_y: u32,
    pub bar_width: u32,
    /// Align the toolbar to the right edge of its track (wide image or near monitor right edge).
    pub bar_align_right: bool,
    pub toolbar: ToolbarPlace,
    /// DPI scale of the capture monitor.
    pub scale: f64,
    /// Physical pixels per page CSS pixel: `scale` times the Windows text
    /// size, which WebView2 applies to the page.
    pub css_scale: f64,
}

type OcrResult = Option<Result<Vec<Paragraph>, String>>;

struct Shot {
    payload: ShotPayload,
    /// Window rectangle in physical screen pixels.
    window: RECT,
    bgra: Vec<u8>,
    ocr: watch::Receiver<OcrResult>,
}

pub struct CaptureState {
    ocr: Arc<Ocr>,
    /// A session (capture + overlay) is running.
    busy: AtomicBool,
    seq: AtomicU64,
    current: Mutex<Option<Arc<Shot>>>,
    overlay: Mutex<Option<(u64, Overlay)>>,
}

impl CaptureState {
    pub fn new(ocr_dir: PathBuf) -> Self {
        Self {
            ocr: Ocr::new(ocr_dir),
            busy: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            current: Mutex::new(None),
            overlay: Mutex::new(None),
        }
    }

    fn shot(&self, id: u64) -> Result<Arc<Shot>, String> {
        self.current.lock().unwrap().clone().filter(|s| s.payload.id == id).ok_or_else(|| "截图已关闭".to_string())
    }

    fn close_overlay(&self, id: u64) {
        let mut guard = self.overlay.lock().unwrap();
        if guard.as_ref().is_some_and(|(i, _)| *i == id) {
            guard.take(); // dropping closes it
        }
    }
}

/// Start a screenshot session (Alt+W). Ignored while one is running.
/// `minimize_main`: started from the main window's button, so get it out of
/// the way first.
pub fn start(app: &AppHandle, minimize_main: bool) {
    let state = app.state::<CaptureState>();
    if state.busy.swap(true, SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("shot-session".into()).spawn(move || {
        let state = app.state::<CaptureState>();
        session(&app, &state, minimize_main);
        state.busy.store(false, SeqCst);
    });
    if let Err(e) = spawned {
        tracing::error!("starting the screenshot session failed: {e}");
        state.busy.store(false, SeqCst);
    }
}

fn session(app: &AppHandle, state: &CaptureState, minimize_main: bool) {
    state.ocr.preload();
    screen::per_monitor_aware();
    // Our own windows must not end up in the frozen image.
    let mut wait = hide(app);
    if minimize_main
        && let Some(main) = app.get_webview_window("main")
        && main.is_visible().unwrap_or(false)
        && !main.is_minimized().unwrap_or(false)
    {
        let _ = main.minimize();
        wait = true;
    }
    if wait {
        std::thread::sleep(Duration::from_millis(250)); // let the hide / minimize animation finish
    }
    let frames = Arc::new(screen::capture_all());
    if frames.is_empty() {
        tracing::error!("screenshot: nothing captured");
        return;
    }
    let overlay = match overlay::open(frames.clone()) {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("screenshot overlay failed: {e}");
            return;
        }
    };
    // Warm up the result WebView while the user drags their selection.
    if app.get_webview_window(LABEL).is_none() {
        let warm_app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::app::windows::ensure_window(&warm_app, LABEL) {
                tracing::error!("creating screenshot result window failed: {e}");
            }
        });
    }
    let Some(sel) = overlay.wait() else {
        tracing::info!("screenshot cancelled");
        return;
    };
    let t0 = Instant::now();
    let frame = &frames[sel.monitor];
    let bgra = screen::crop(frame, sel.rect);
    let (w, h) = ((sel.rect.right - sel.rect.left) as u32, (sel.rect.bottom - sel.rect.top) as u32);
    let id = state.seq.fetch_add(1, SeqCst) + 1;
    let image = RECT {
        left: frame.rect.left + sel.rect.left,
        top: frame.rect.top + sel.rect.top,
        right: frame.rect.left + sel.rect.right,
        bottom: frame.rect.top + sel.rect.bottom,
    };
    let (window, payload) = geometry(id, image, frame);
    tracing::info!("screenshot {id}: {w}x{h} at ({}, {}) on monitor {}", image.left, image.top, sel.monitor);

    let (tx, rx) = watch::channel(None);
    let shot = Arc::new(Shot { payload: payload.clone(), window, bgra, ocr: rx });
    *state.current.lock().unwrap() = Some(shot.clone());
    let ocr = state.ocr.clone();
    std::thread::spawn(move || {
        let result = ocr.recognize(&shot.bgra, w, h).map(|lines| layout::paragraphs(lines, &shot.bgra, w, h));
        let _ = tx.send(Some(result));
    });

    let Ok(win) = crate::app::windows::ensure_window(app, LABEL) else { return };
    place(&win, window);
    *state.overlay.lock().unwrap() = Some((id, overlay));
    let _ = app.emit_to(LABEL, "shot-new", payload);
    tracing::debug!("screenshot {id} handed to the page in {} ms", t0.elapsed().as_millis());

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(OVERLAY_FALLBACK);
        app.state::<CaptureState>().close_overlay(id);
    });
}

/// Result window rectangle and the image/toolbar tracks inside it: the image
/// keeps its exact screen position while the transparent window extends around
/// the floating toolbar so its border and drop shadow are never clipped.
fn geometry(id: u64, image: RECT, frame: &screen::Frame) -> (RECT, ShotPayload) {
    geometry_for(id, image, frame.rect, frame.work, frame.scale as f64, crate::app::windows::text_scale())
}

fn geometry_for(
    id: u64,
    image: RECT,
    rect: RECT,
    work: RECT,
    scale: f64,
    text_scale: f64,
) -> (RECT, ShotPayload) {
    let css = scale * text_scale;
    let (w, h) = (image.right - image.left, image.bottom - image.top);
    let gap = (BAR_GAP * css).ceil() as i32;
    let bar_h = (BAR_HEIGHT * css).ceil() as i32;
    let bar_min_w = (BAR_MIN_WIDTH * css).ceil() as i32;
    let bar_max_w = (BAR_MAX_WIDTH * css).ceil() as i32;
    let pad_x = (SHADOW_PAD_X * css).ceil() as i32;
    let pad_y = (SHADOW_PAD_Y * css).ceil() as i32;
    let frame_w = (rect.right - rect.left).max(1);
    let effective_min_w = bar_min_w.min(frame_w);

    // 1. Vertical placement: prefer Below/Above with full shadow room, then tight Below/Above, else Inside.
    let (toolbar, bar_top, win_top, win_bottom) =
        if image.bottom + gap + bar_h <= work.bottom && image.bottom + gap + bar_h + pad_y <= rect.bottom {
            let bt = image.bottom + gap;
            (ToolbarPlace::Below, bt, image.top, bt + bar_h + pad_y)
        } else if image.top - gap - bar_h >= work.top && image.top - gap - bar_h - pad_y >= rect.top {
            let bt = image.top - gap - bar_h;
            (ToolbarPlace::Above, bt, bt - pad_y, image.bottom)
        } else if image.bottom + gap + bar_h <= work.bottom {
            let bt = image.bottom + gap;
            (ToolbarPlace::Below, bt, image.top, (bt + bar_h + pad_y).min(rect.bottom))
        } else if image.top - gap - bar_h >= work.top {
            let bt = image.top - gap - bar_h;
            (ToolbarPlace::Above, bt, (bt - pad_y).max(rect.top), image.bottom)
        } else {
            let bb = (image.bottom - gap).max(image.top + bar_h).min(rect.bottom);
            let bt = (bb - bar_h).max(rect.top);
            let wt = image.top.min(bt - pad_y).max(rect.top);
            let wb = image.bottom.max(bb + pad_y).min(rect.bottom);
            (ToolbarPlace::Inside, bt, wt, wb)
        };

    // 2. Horizontal toolbar track [track_left, track_right] in screen pixels.
    let inside_inset = if matches!(toolbar, ToolbarPlace::Inside) { gap } else { 0 };
    let (track_left, track_right, bar_align_right) = if w >= effective_min_w + inside_inset * 2 {
        let track_w = (w - inside_inset * 2).min(bar_max_w);
        let right = image.right - inside_inset;
        (right - track_w, right, true)
    } else if image.left + effective_min_w + pad_x <= rect.right {
        (image.left, image.left + effective_min_w, false)
    } else if image.right - effective_min_w - pad_x >= rect.left {
        (image.right - effective_min_w, image.right, true)
    } else if image.left + effective_min_w <= rect.right {
        (image.left, image.left + effective_min_w, false)
    } else {
        let right = image.right.min(rect.right);
        let left = (right - effective_min_w).max(rect.left);
        let right = (left + effective_min_w).min(rect.right);
        (left, right, true)
    };

    // 3. Window horizontal bounds: union of image and padded toolbar track, clamped to monitor.
    let win_left = image.left.min(track_left - pad_x).max(rect.left);
    let win_right = image.right.max(track_right + pad_x).min(rect.right);
    let safe_left = track_left.clamp(win_left, win_right);
    let safe_right = track_right.clamp(safe_left, win_right);

    let window = RECT {
        left: win_left,
        top: win_top,
        right: win_right.max(win_left + 1),
        bottom: win_bottom.max(win_top + 1),
    };
    let payload = ShotPayload {
        id,
        width: w as u32,
        height: h as u32,
        image_x: (image.left - win_left) as u32,
        image_y: (image.top - win_top) as u32,
        bar_x: (safe_left - win_left) as u32,
        bar_y: (bar_top - win_top).max(0) as u32,
        bar_width: (safe_right - safe_left) as u32,
        bar_align_right,
        toolbar,
        scale,
        css_scale: css,
    };
    (window, payload)
}

fn place(win: &WebviewWindow, r: RECT) {
    let _ = win.set_position(PhysicalPosition::new(r.left, r.top));
    let _ = win.set_size(PhysicalSize::new((r.right - r.left) as u32, (r.bottom - r.top) as u32));
}

/// Hide the result window. Returns whether it was visible.
pub fn hide(app: &AppHandle) -> bool {
    let Some(win) = app.get_webview_window(LABEL) else { return false };
    let visible = win.is_visible().unwrap_or(false);
    if visible {
        let _ = win.hide();
    }
    app.state::<crate::state::AppState>().tts.stop_for(app, LABEL);
    *app.state::<CaptureState>().current.lock().unwrap() = None;
    let _ = app.emit_to(LABEL, "shot-hidden", ());
    visible
}

#[tauri::command]
pub fn shot_start(app: AppHandle) {
    start(&app, true);
}

/// Latest screenshot, for a page that loaded after `shot-new`.
#[tauri::command]
pub fn shot_current(state: State<'_, CaptureState>) -> Option<ShotPayload> {
    state.current.lock().unwrap().as_ref().map(|s| s.payload.clone())
}

/// The screenshot as raw RGBA. Async (like `shot_copy_image`): sync commands
/// run on the main thread, and converting / copying a 4K image there stalls
/// every window.
#[tauri::command]
pub async fn shot_image(state: State<'_, CaptureState>, id: u64) -> Result<Response, String> {
    let shot = state.shot(id)?;
    let mut rgba = shot.bgra.clone();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Ok(Response::new(rgba))
}

/// Paragraphs found by OCR (waits for it).
#[tauri::command]
pub async fn shot_ocr(state: State<'_, CaptureState>, id: u64) -> Result<Vec<Paragraph>, String> {
    let mut rx = state.shot(id)?.ocr.clone();
    let result = rx.wait_for(|r| r.is_some()).await.map_err(|_| "文字识别已中止".to_string())?;
    result.clone().unwrap()
}

/// The page has drawn the screenshot: show the window over the overlay, then
/// remove the overlay.
#[tauri::command]
pub fn shot_shown(app: AppHandle, state: State<'_, CaptureState>, id: u64) {
    let Ok(shot) = state.shot(id) else { return };
    let Some(win) = app.get_webview_window(LABEL) else { return };
    let _ = win.show();
    // Showing may move the window onto a monitor with another DPI; re-apply.
    place(&win, shot.window);
    let _ = win.set_focus();
    if let Ok(h) = win.hwnd() {
        crate::selection::input::force_foreground(HWND(h.0 as *mut _));
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        app.state::<CaptureState>().close_overlay(id);
    });
}

#[tauri::command]
pub fn shot_close(app: AppHandle) {
    hide(&app);
}

/// Put an RGBA image (raw body; `x-width` / `x-height` headers) on the
/// clipboard as a 24-bit DIB.
#[tauri::command]
pub async fn shot_copy_image(app: AppHandle, request: Request<'_>) -> Result<(), String> {
    let InvokeBody::Raw(rgba) = request.body() else { return Err("缺少图像数据".into()) };
    let header = |k: &str| -> Option<usize> { request.headers().get(k)?.to_str().ok()?.parse().ok() };
    let (Some(w), Some(h)) = (header("x-width"), header("x-height")) else { return Err("缺少图像尺寸".into()) };
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err("图像尺寸不符".into());
    }
    let stride = (w * 3).div_ceil(4) * 4;
    let mut dib = Vec::with_capacity(40 + stride * h);
    let header_fields: [u32; 10] = [40, w as u32, h as u32, 1 | (24 << 16), 0, (stride * h) as u32, 0, 0, 0, 0];
    for v in header_fields {
        dib.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..h).rev() {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        for px in row.as_chunks::<4>().0 {
            dib.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        dib.resize(dib.len() + (stride - w * 3), 0);
    }
    let owner = app.get_webview_window(LABEL).and_then(|w| w.hwnd().ok()).map(|h| HWND(h.0 as *mut _));
    const CF_DIB: u32 = 8;
    if crate::selection::clipboard::set_contents(owner, &[(CF_DIB, dib)]) {
        Ok(())
    } else {
        Err("无法打开剪贴板".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn narrow_below_reserves_shadow_padding_and_keeps_image_fixed() {
        // Exact repro of 499x143 at (996, 434) on 3840x2160 @ 150% DPI, 125% text scale.
        let screen = rect(0, 0, 3840, 2160);
        let work = rect(0, 0, 3840, 2088);
        let img = rect(996, 434, 996 + 499, 434 + 143);
        let (win, p) = geometry_for(1, img, screen, work, 1.5, 1.25);

        let css = 1.875;
        let pad_x = (SHADOW_PAD_X * css).ceil() as i32;
        let pad_y = (SHADOW_PAD_Y * css).ceil() as i32;
        let bar_h = (BAR_HEIGHT * css).ceil() as i32;
        let bar_min_w = (BAR_MIN_WIDTH * css).ceil() as i32;

        assert_eq!(p.toolbar, ToolbarPlace::Below);
        assert!(!p.bar_align_right);
        // Image keeps exact screen coordinates.
        assert_eq!(win.left + p.image_x as i32, img.left);
        assert_eq!(win.top + p.image_y as i32, img.top);
        // Toolbar aligns with image left and has full shadow padding on left, right and bottom.
        assert_eq!(p.bar_x, p.image_x);
        assert_eq!(p.bar_width as i32, bar_min_w);
        assert!(p.bar_x as i32 >= pad_x);
        assert!((win.right - win.left) - (p.bar_x + p.bar_width) as i32 >= pad_x);
        assert!((win.bottom - win.top) - (p.bar_y as i32 + bar_h) >= pad_y);
    }

    #[test]
    fn narrow_near_right_edge_flips_and_stays_in_monitor() {
        let screen = rect(0, 0, 1920, 1080);
        let work = rect(0, 0, 1920, 1040);
        let img = rect(1700, 200, 1880, 320);
        let (win, p) = geometry_for(2, img, screen, work, 1.0, 1.0);

        assert_eq!(p.toolbar, ToolbarPlace::Below);
        assert!(p.bar_align_right);
        assert_eq!(win.left + p.image_x as i32, img.left);
        assert_eq!(win.left + (p.bar_x + p.bar_width) as i32, img.right);
        assert!(win.left >= screen.left && win.right <= screen.right);
        assert!(p.bar_x >= SHADOW_PAD_X as u32);
        assert!((win.right - win.left) - (p.bar_x + p.bar_width) as i32 >= SHADOW_PAD_X as i32);
    }

    #[test]
    fn wide_image_right_aligns_bar_without_extra_left_margin() {
        let screen = rect(0, 0, 2560, 1440);
        let work = rect(0, 0, 2560, 1392);
        let img = rect(300, 200, 1500, 500);
        let (win, p) = geometry_for(3, img, screen, work, 1.0, 1.0);

        assert_eq!(p.toolbar, ToolbarPlace::Below);
        assert!(p.bar_align_right);
        assert_eq!(win.left, img.left);
        assert_eq!(p.image_x, 0);
        assert_eq!(p.bar_width, BAR_MAX_WIDTH as u32);
        assert_eq!(win.left + (p.bar_x + p.bar_width) as i32, img.right);
        assert_eq!(win.right, img.right + SHADOW_PAD_X as i32);
    }

    #[test]
    fn near_bottom_places_toolbar_above_with_top_shadow_padding() {
        let screen = rect(0, 0, 1920, 1080);
        let work = rect(0, 0, 1920, 1040);
        let img = rect(400, 900, 700, 1030);
        let (win, p) = geometry_for(4, img, screen, work, 1.0, 1.0);

        assert_eq!(p.toolbar, ToolbarPlace::Above);
        assert_eq!(win.left + p.image_x as i32, img.left);
        assert_eq!(win.top + p.image_y as i32, img.top);
        assert_eq!(p.bar_y, SHADOW_PAD_Y as u32);
        assert!(p.bar_y + BAR_HEIGHT as u32 + BAR_GAP as u32 == p.image_y);
    }

    #[test]
    fn full_height_places_toolbar_inside_and_never_underflows() {
        let screen = rect(0, 0, 1920, 1080);
        let work = rect(0, 0, 1920, 1040);
        let img = rect(0, 0, 1920, 1040);
        let (win, p) = geometry_for(5, img, screen, work, 1.0, 1.0);

        assert_eq!(p.toolbar, ToolbarPlace::Inside);
        assert!(p.bar_align_right);
        assert!(win.left >= screen.left && win.right <= screen.right);
        assert!(win.top >= screen.top && win.bottom <= screen.bottom);
        assert_eq!(p.image_x, 0);
        assert_eq!(p.image_y, 0);
        assert!(p.bar_x + p.bar_width <= (win.right - win.left) as u32);
    }
}
