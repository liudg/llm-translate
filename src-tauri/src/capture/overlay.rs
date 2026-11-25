//! Region selection (DESIGN 5.6 step 2): one native full-screen window per
//! monitor shows the frozen frame dimmed; dragging reveals the selection at
//! full brightness with an outline and a size label. Native GDI instead of a
//! WebView so it appears instantly and works in physical pixels on every
//! monitor regardless of scaling.
//!
//! Esc, or a right click with no drag in progress, cancels; a right click
//! during a drag drops that drag. A selection stays on screen after the mouse
//! is released until [`Overlay::close`], so the result window can appear on
//! top of it without the live desktop flashing in between.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};

use windows::Win32::Foundation::{CloseHandle, COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, GetProcessIdOfThread, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::screen::{Frame, per_monitor_aware};

const CLASS: windows::core::PCWSTR = w!("LLMTranslateShotOverlay");
/// Brightness of the area outside the selection.
const DIM: u32 = 140; // of 256
/// Win11 dark-theme accent, readable on the dimmed frame. COLORREF is 0x00BBGGRR.
const OUTLINE: COLORREF = COLORREF(0x00FF_C24C);
const LABEL_BG: COLORREF = COLORREF(0x0020_2020);
const HINT: &str = "拖动鼠标框选要翻译的区域　Esc 取消";
/// Posted to take the foreground back from another window of this app.
const WM_RECLAIM: u32 = WM_APP + 1;

/// A finished selection: frame index and rectangle relative to that frame.
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub monitor: usize,
    pub rect: RECT,
}

pub struct Overlay {
    thread_id: u32,
    result: Receiver<Option<Selection>>,
}

impl Overlay {
    /// Block until the user selects (`Some`) or cancels (`None`).
    pub fn wait(&self) -> Option<Selection> {
        self.result.recv().ok().flatten()
    }

    /// Remove the overlay windows.
    pub fn close(&self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.close();
    }
}

/// A 32-bit top-down DIB selected into its own memory DC.
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bmp) => {
                    let old = SelectObject(dc, bmp.into());
                    Some(Self { dc, bmp, old, bits: bits as *mut u8 })
                }
                Err(_) => {
                    let _ = DeleteDC(dc);
                    None
                }
            }
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

struct Win {
    hwnd: HWND,
    w: i32,
    h: i32,
    scale: f32,
    bright: Dib,
    dim: Dib,
    back: Dib,
    font: HFONT,
}

struct Drag {
    monitor: usize,
    start: POINT,
    cur: POINT,
}

impl Drag {
    fn rect(&self) -> RECT {
        RECT {
            left: self.start.x.min(self.cur.x),
            top: self.start.y.min(self.cur.y),
            right: self.start.x.max(self.cur.x),
            bottom: self.start.y.max(self.cur.y),
        }
    }
}

struct State {
    wins: Vec<Win>,
    drag: Option<Drag>,
    done: bool,
    tx: Sender<Option<Selection>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

impl State {
    fn index(&self, hwnd: HWND) -> Option<usize> {
        self.wins.iter().position(|w| w.hwnd == hwnd)
    }

    fn finish(&mut self, sel: Option<Selection>) {
        if self.done {
            return;
        }
        self.done = true;
        unsafe {
            let _ = ReleaseCapture();
        }
        let cancelled = sel.is_none();
        let _ = self.tx.send(sel);
        if cancelled {
            unsafe { PostQuitMessage(0) };
        }
    }

    /// Size label of a selection: above it, or inside when there is no room.
    /// `back`'s font must be selected.
    fn label_rect(win: &Win, r: RECT, back: HDC) -> (String, RECT) {
        let s = win.scale;
        let label = format!("{} × {}", r.right - r.left, r.bottom - r.top);
        let pad = (6.0 * s) as i32;
        let size = text_size(back, &label);
        let (lw, lh) = (size.cx + 2 * pad, size.cy + pad);
        let gap = (4.0 * s) as i32;
        let ly = if r.top - lh - gap >= 0 { r.top - lh - gap } else { r.top + gap };
        let lx = r.left.clamp(0, (win.w - lw).max(0));
        (label, RECT { left: lx, top: ly, right: lx + lw, bottom: ly + lh })
    }

    /// Everything a drag draws on window `i`: selection, outline and label.
    fn drag_area(&self, i: usize) -> Option<RECT> {
        let win = &self.wins[i];
        let d = self.drag.as_ref().filter(|d| d.monitor == i)?;
        let r = d.rect();
        let label = unsafe {
            let old = SelectObject(win.back.dc, win.font.into());
            let (_, l) = Self::label_rect(win, r, win.back.dc);
            SelectObject(win.back.dc, old);
            l
        };
        let t = (2.0 * win.scale).ceil() as i32 + 1;
        Some(RECT {
            left: r.left.min(label.left) - t,
            top: r.top.min(label.top) - t,
            right: r.right.max(label.right) + t,
            bottom: r.bottom.max(label.bottom) + t,
        })
    }

    /// Repaint `clip` (the window's update region bounds). Only that part
    /// is rebuilt and copied: a drag invalidates just what it changed, so
    /// moving the mouse no longer copies the whole screen twice.
    fn paint(&self, i: usize, hdc: HDC, clip: RECT) {
        let win = &self.wins[i];
        let (w, h, s) = (win.w, win.h, win.scale);
        let back = win.back.dc;
        let c = RECT { left: clip.left.max(0), top: clip.top.max(0), right: clip.right.min(w), bottom: clip.bottom.min(h) };
        let (cw, ch) = (c.right - c.left, c.bottom - c.top);
        if cw <= 0 || ch <= 0 {
            return;
        }
        unsafe {
            let _ = BitBlt(back, c.left, c.top, cw, ch, Some(win.dim.dc), c.left, c.top, SRCCOPY);
            let old_font = SelectObject(back, win.font.into());
            SetBkMode(back, TRANSPARENT);
            SetTextColor(back, COLORREF(0x00FF_FFFF));
            match self.drag.as_ref().filter(|d| d.monitor == i) {
                Some(d) => {
                    let r = d.rect();
                    let (rw, rh) = (r.right - r.left, r.bottom - r.top);
                    let _ = BitBlt(back, r.left, r.top, rw, rh, Some(win.bright.dc), r.left, r.top, SRCCOPY);
                    outline(back, r, (1.5 * s).round().max(1.0) as i32);
                    let (label, lr) = Self::label_rect(win, r, back);
                    pill(back, lr, (4.0 * s) as i32);
                    draw_text(back, &label, lr);
                }
                None if self.drag.is_none() => {
                    let pad = (12.0 * s) as i32;
                    let size = text_size(back, HINT);
                    let (pw, ph) = (size.cx + 2 * pad, size.cy + pad);
                    let px = (w - pw) / 2;
                    let py = (24.0 * s) as i32;
                    let r = RECT { left: px, top: py, right: px + pw, bottom: py + ph };
                    pill(back, r, ph);
                    draw_text(back, HINT, r);
                }
                None => {}
            }
            SelectObject(back, old_font);
            let _ = BitBlt(hdc, c.left, c.top, cw, ch, Some(back), c.left, c.top, SRCCOPY);
        }
    }

    fn invalidate(&self) {
        for win in &self.wins {
            unsafe {
                let _ = InvalidateRect(Some(win.hwnd), None, false);
            }
        }
    }
}

fn text_size(dc: HDC, text: &str) -> SIZE {
    let t: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(dc, &t, &mut size);
    }
    size
}

fn draw_text(dc: HDC, text: &str, mut r: RECT) {
    let mut t: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        DrawTextW(dc, &mut t, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }
}

fn pill(dc: HDC, r: RECT, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(LABEL_BG);
        let old_brush = SelectObject(dc, brush.into());
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, radius, radius);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(brush.into());
    }
}

/// Outline drawn just outside `r` so it never covers selected pixels.
fn outline(dc: HDC, r: RECT, t: i32) {
    unsafe {
        let brush = CreateSolidBrush(OUTLINE);
        let (l, tp, rt, b) = (r.left - t, r.top - t, r.right + t, r.bottom + t);
        for edge in [
            RECT { left: l, top: tp, right: rt, bottom: r.top },
            RECT { left: l, top: r.bottom, right: rt, bottom: b },
            RECT { left: l, top: r.top, right: r.left, bottom: r.bottom },
            RECT { left: r.right, top: r.top, right: rt, bottom: r.bottom },
        ] {
            FillRect(dc, &edge, brush);
        }
        let _ = DeleteObject(brush.into());
    }
}

/// Whether thread `tid` belongs to this process.
fn own_thread(tid: u32) -> bool {
    if tid == 0 {
        return false;
    }
    unsafe {
        let Ok(h) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid) else { return false };
        let pid = GetProcessIdOfThread(h);
        let _ = CloseHandle(h);
        pid != 0 && pid == GetCurrentProcessId()
    }
}

fn point(lp: LPARAM) -> POINT {
    POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            with_state(|s| {
                if let Some(i) = s.index(hwnd) {
                    s.paint(i, hdc, ps.rcPaint);
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            true
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_LBUTTONDOWN => with_state(|s| {
            if s.done {
                return;
            }
            if let Some(i) = s.index(hwnd) {
                let p = point(lp);
                s.drag = Some(Drag { monitor: i, start: p, cur: p });
                unsafe { SetCapture(hwnd) };
                s.invalidate();
            }
        })
        .is_some(),
        WM_MOUSEMOVE => with_state(|s| {
            let Some(i) = s.index(hwnd) else { return };
            let (w, h) = (s.wins[i].w, s.wins[i].h);
            let before = s.drag_area(i);
            if let Some(d) = s.drag.as_mut().filter(|d| d.monitor == i && !s.done) {
                let p = point(lp);
                d.cur = POINT { x: p.x.clamp(0, w), y: p.y.clamp(0, h) };
                // Only the old and new selection (with outline and label).
                let changed = match (before, s.drag_area(i)) {
                    (Some(a), Some(b)) => Some(RECT {
                        left: a.left.min(b.left),
                        top: a.top.min(b.top),
                        right: a.right.max(b.right),
                        bottom: a.bottom.max(b.bottom),
                    }),
                    _ => None,
                };
                let _ = unsafe { InvalidateRect(Some(hwnd), changed.as_ref().map(|r| r as *const RECT), false) };
            }
        })
        .is_some(),
        WM_LBUTTONUP => with_state(|s| {
            if s.done {
                return;
            }
            let Some(d) = s.drag.as_ref() else { return };
            let r = d.rect();
            let monitor = d.monitor;
            let min = (4.0 * s.wins[monitor].scale).round() as i32;
            if r.right - r.left >= min && r.bottom - r.top >= min {
                s.finish(Some(Selection { monitor, rect: r }));
            } else {
                s.drag = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                s.invalidate();
            }
        })
        .is_some(),
        WM_RBUTTONUP => with_state(|s| {
            if s.done {
                return;
            }
            if s.drag.take().is_some() {
                unsafe {
                    let _ = ReleaseCapture();
                }
                s.invalidate();
            } else {
                s.finish(None);
            }
        })
        .is_some(),
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => with_state(|s| s.finish(None)).is_some(),
        // Switched to another app (e.g. Alt+Tab): give up. One of our own
        // windows taking it (the result window's first WebView2 creation,
        // which runs while the overlay is up) is not the user leaving: take
        // the foreground back so Esc keeps working. lParam: the thread being
        // activated.
        WM_ACTIVATEAPP if wp.0 == 0 => {
            if own_thread(lp.0 as u32) {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_RECLAIM, WPARAM(0), LPARAM(0));
                }
                true
            } else {
                with_state(|s| s.finish(None)).is_some()
            }
        }
        // Not after a selection: the result window then rightly takes over.
        WM_RECLAIM => with_state(|s| {
            if !s.done && !crate::selection::input::force_foreground(hwnd) {
                tracing::warn!("screenshot overlay could not take the foreground back");
            }
        })
        .is_some(),
        _ => false,
    };
    if handled { LRESULT(0) } else { unsafe { DefWindowProcW(hwnd, msg, wp, lp) } }
}

fn create_win(frame: &Frame) -> Option<Win> {
    let (w, h) = (frame.width(), frame.height());
    let bright = Dib::new(w, h)?;
    let dim = Dib::new(w, h)?;
    let back = Dib::new(w, h)?;
    let n = (w * h * 4) as usize;
    unsafe {
        let b = std::slice::from_raw_parts_mut(bright.bits, n);
        b.copy_from_slice(&frame.bgra);
        let d = std::slice::from_raw_parts_mut(dim.bits, n);
        for (o, &v) in d.iter_mut().zip(frame.bgra.iter()) {
            *o = ((v as u32 * DIM) >> 8) as u8;
        }
        let font = CreateFontW(
            -(14.0 * frame.scale).round() as i32,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Microsoft YaHei UI"),
        );
        let instance = GetModuleHandleW(None).ok()?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("LLM Translate · 截图"),
            WS_POPUP,
            frame.rect.left,
            frame.rect.top,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        Some(Win { hwnd, w, h, scale: frame.scale, bright, dim, back, font })
    }
}

fn register_class() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // Fails harmlessly when already registered by an earlier session.
        RegisterClassExW(&class);
    }
}

/// Show the overlay over the frozen frames on a new thread.
pub fn open(frames: Arc<Vec<Frame>>) -> anyhow::Result<Overlay> {
    let (ready_tx, ready_rx) = sync_channel::<anyhow::Result<u32>>(1);
    let (tx, result) = channel();
    std::thread::Builder::new().name("shot-overlay".into()).spawn(move || {
        per_monitor_aware();
        register_class();
        let wins: Vec<Win> = frames.iter().filter_map(create_win).collect();
        if wins.is_empty() {
            let _ = ready_tx.send(Err(anyhow::anyhow!("creating the overlay windows failed")));
            return;
        }
        drop(frames);
        let hwnds: Vec<(HWND, RECT)> = wins
            .iter()
            .map(|w| {
                let mut r = RECT::default();
                unsafe {
                    let _ = GetWindowRect(w.hwnd, &mut r);
                }
                (w.hwnd, r)
            })
            .collect();
        STATE.with(|s| *s.borrow_mut() = Some(State { wins, drag: None, done: false, tx }));
        unsafe {
            for &(hwnd, r) in &hwnds {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_SHOWWINDOW,
                );
                let _ = UpdateWindow(hwnd);
            }
            // Keyboard focus (Esc) goes to the window under the cursor.
            let mut cur = POINT::default();
            let _ = GetCursorPos(&mut cur);
            let target = hwnds
                .iter()
                .find(|(_, r)| cur.x >= r.left && cur.x < r.right && cur.y >= r.top && cur.y < r.bottom)
                .map(|(h, _)| *h)
                .unwrap_or(hwnds[0].0);
            if !crate::selection::input::force_foreground(target) {
                tracing::warn!("screenshot overlay could not take the foreground");
            }
        }
        let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
            // A close before any selection counts as cancelled.
            let _ = state.tx.send(None);
            for win in state.wins {
                unsafe {
                    let _ = DestroyWindow(win.hwnd);
                    let _ = DeleteObject(win.font.into());
                }
            }
        }
    })?;
    let thread_id = ready_rx.recv()??;
    Ok(Overlay { thread_id, result })
}
