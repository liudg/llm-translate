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
/// Toolbar strip height and minimum width in DIPs (must match Shot.tsx).
const TOOLBAR_HEIGHT: f64 = 44.0;
const TOOLBAR_MIN_WIDTH: f64 = 392.0;
/// Closes the overlay if the page never reports that it has drawn.
const OVERLAY_FALLBACK: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, Serialize)]
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
    /// Image position inside the window (the toolbar may be wider or above).
    pub image_x: u32,
    pub image_y: u32,
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

/// Result window rectangle and the image's place in it: the image keeps its
/// screen position; the toolbar goes below it, else above, else over it.
fn geometry(id: u64, image: RECT, frame: &screen::Frame) -> (RECT, ShotPayload) {
    let s = frame.scale as f64;
    let css = s * crate::app::windows::text_scale();
    let (w, h) = (image.right - image.left, image.bottom - image.top);
    let bar = (TOOLBAR_HEIGHT * css).round() as i32;
    let win_w = w.max((TOOLBAR_MIN_WIDTH * css).round() as i32);
    let (mut left, mut image_x) = (image.left, 0);
    if left + win_w > frame.rect.right {
        left = image.right - win_w;
        image_x = win_w - w;
    }
    let work = frame.work;
    let (top, image_y, win_h, toolbar) = if image.bottom + bar <= work.bottom {
        (image.top, 0, h + bar, ToolbarPlace::Below)
    } else if image.top - bar >= work.top {
        (image.top - bar, bar, h + bar, ToolbarPlace::Above)
    } else {
        (image.top, 0, h.max(bar), ToolbarPlace::Inside)
    };
    let window = RECT { left, top, right: left + win_w, bottom: top + win_h };
    let payload = ShotPayload {
        id,
        width: w as u32,
        height: h as u32,
        image_x: image_x as u32,
        image_y: image_y as u32,
        toolbar,
        scale: s,
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
