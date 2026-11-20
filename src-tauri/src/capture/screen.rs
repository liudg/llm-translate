//! Freeze every monitor (DESIGN 5.6 step 1). One GDI BitBlt per monitor from
//! the screen DC: with DWM this returns the composed desktop including layered
//! windows (`CAPTUREBLT`), needs no D3D device, and works for rotated outputs
//! and outputs on another GPU, where DXGI Desktop Duplication fails.

use std::time::Instant;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::core::BOOL;

/// One monitor, frozen. Coordinates are physical virtual-screen pixels.
pub struct Frame {
    pub rect: RECT,
    pub work: RECT,
    /// DPI scale, 1.0 = 96 dpi.
    pub scale: f32,
    /// Top-down BGRA, `width * height * 4` bytes, alpha 255.
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
}

/// Make the calling thread per-monitor DPI aware (v2) so window and capture
/// coordinates are physical pixels.
pub fn per_monitor_aware() {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

struct MonitorInfo {
    rect: RECT,
    work: RECT,
    scale: f32,
}

fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn cb(mon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(lp.0 as *mut Vec<MonitorInfo>);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                let (mut x, mut y) = (96u32, 96u32);
                let scale =
                    if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() { x as f32 / 96.0 } else { 1.0 };
                out.push(MonitorInfo { rect: mi.rcMonitor, work: mi.rcWork, scale });
            }
            true.into()
        }
    }
    let mut out: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Copy a screen rectangle into top-down BGRA.
fn grab(r: RECT) -> Option<Vec<u8>> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
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
        let out = match CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(dib) => {
                let old = SelectObject(mem, dib.into());
                let ok = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
                let _ = GdiFlush();
                let px = ok.then(|| {
                    let mut v = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
                    for a in v.iter_mut().skip(3).step_by(4) {
                        *a = 255;
                    }
                    v
                });
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
                px
            }
            Err(_) => None,
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        out
    }
}

/// Freeze all monitors. Call on a per-monitor-aware thread.
pub fn capture_all() -> Vec<Frame> {
    let t0 = Instant::now();
    let frames: Vec<Frame> = monitors()
        .into_iter()
        .filter_map(|m| match grab(m.rect) {
            Some(bgra) => Some(Frame { rect: m.rect, work: m.work, scale: m.scale, bgra }),
            None => {
                tracing::warn!("capturing monitor {:?} failed", m.rect);
                None
            }
        })
        .collect();
    tracing::info!(
        "captured {} monitor(s) in {} ms: {}",
        frames.len(),
        t0.elapsed().as_millis(),
        frames
            .iter()
            .map(|f| format!("{}x{}@{}%", f.width(), f.height(), (f.scale * 100.0).round()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    frames
}

/// Crop `r` (frame-relative) out of a frame as top-down BGRA.
pub fn crop(frame: &Frame, r: RECT) -> Vec<u8> {
    let fw = frame.width() as usize;
    let (w, h) = ((r.right - r.left) as usize, (r.bottom - r.top) as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in r.top as usize..r.top as usize + h {
        let start = (y * fw + r.left as usize) * 4;
        out.extend_from_slice(&frame.bgra[start..start + w * 4]);
    }
    out
}
