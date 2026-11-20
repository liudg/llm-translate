//! The selection icon: a native layered window that never takes focus
//! (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`), so clicking it leaves the selection
//! and keyboard focus in the source app for UIA / Ctrl+Insert. Drawn in
//! software: an accent rounded square with a white ring, soft shadow and a
//! white "译" glyph.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Window edge in DIPs (the square is 24 with room for the ring and shadow).
const WINDOW_DIP: f32 = 32.0;
const SQUARE_DIP: f32 = 24.0;
const ACCENT: (f32, f32, f32) = (0x00 as f32, 0x5f as f32, 0xb8 as f32);
const ACCENT_HOVER: (f32, f32, f32) = (0x0a as f32, 0x6c as f32, 0xc4 as f32);

/// DPI scale (1.0 = 96 dpi) of the monitor containing `pt`.
pub fn scale_at(pt: POINT) -> f32 {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() { x as f32 / 96.0 } else { 1.0 }
    }
}

pub fn work_area_at(pt: POINT) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

pub struct Icon {
    pub hwnd: HWND,
    /// Screen rectangle while visible.
    pub rect: RECT,
    pub visible: bool,
    hover: bool,
    scale: f32,
}

impl Icon {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd, rect: RECT::default(), visible: false, hover: false, scale: 1.0 }
    }

    /// Show at the lower right of the cursor, kept inside the monitor's work
    /// area. Returns the screen rectangle.
    pub fn show_near(&mut self, cursor: POINT) -> RECT {
        let s = scale_at(cursor);
        let size = (WINDOW_DIP * s).round() as i32;
        let area = work_area_at(cursor);
        let mut x = cursor.x + (8.0 * s) as i32;
        let mut y = cursor.y + (14.0 * s) as i32;
        if x + size > area.right {
            x = cursor.x - size - (4.0 * s) as i32;
        }
        if y + size > area.bottom {
            y = cursor.y - size - (4.0 * s) as i32;
        }
        x = x.clamp(area.left, (area.right - size).max(area.left));
        y = y.clamp(area.top, (area.bottom - size).max(area.top));
        self.scale = s;
        self.hover = false;
        self.rect = RECT { left: x, top: y, right: x + size, bottom: y + size };
        self.paint();
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                size,
                size,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.visible = true;
        self.rect
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
            self.hover = false;
        }
    }

    pub fn set_hover(&mut self, hover: bool) {
        if self.visible && hover != self.hover {
            self.hover = hover;
            self.paint();
        }
    }

    fn paint(&self) {
        let size = (self.rect.right - self.rect.left).max(1);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            };
            let old_bmp = SelectObject(mem, dib.into());
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
            px.fill(0);
            self.draw_glyph(mem, size);
            let _ = GdiFlush();
            self.compose(px, size);

            let pos = POINT { x: self.rect.left, y: self.rect.top };
            let sz = SIZE { cx: size, cy: size };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&pos),
                Some(&sz),
                Some(mem),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(mem, old_bmp);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    /// Square geometry in pixels: (left, top, edge, radius).
    fn square(&self, size: i32) -> (f32, f32, f32, f32) {
        let s = self.scale;
        let edge = SQUARE_DIP * s;
        let left = (size as f32 - edge) / 2.0;
        let top = left - 1.0 * s; // leave more room below for the shadow
        (left, top, edge, 6.0 * s)
    }

    /// White grayscale-antialiased glyph; its red channel becomes coverage.
    unsafe fn draw_glyph(&self, dc: HDC, size: i32) {
        unsafe {
            let (left, top, edge, _) = self.square(size);
            let font = CreateFontW(
                -(15.0 * self.scale).round() as i32,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Microsoft YaHei UI"),
            );
            let old = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let mut r = RECT {
                left: left.round() as i32,
                top: top.round() as i32,
                right: (left + edge).round() as i32,
                bottom: (top + edge).round() as i32,
            };
            let mut text: Vec<u16> = "译".encode_utf16().collect();
            DrawTextW(dc, &mut text, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old);
            let _ = DeleteObject(font.into());
        }
    }

    /// Replace the buffer with premultiplied BGRA: shadow, white ring, accent
    /// square, glyph.
    fn compose(&self, px: &mut [u8], size: i32) {
        let s = self.scale;
        let (left, top, edge, radius) = self.square(size);
        let (cx, cy, half) = (left + edge / 2.0, top + edge / 2.0, edge / 2.0);
        let ring = 1.25 * s;
        let accent = if self.hover { ACCENT_HOVER } else { ACCENT };
        for y in 0..size {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                let glyph = px[i + 2] as f32 / 255.0;
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d_inner = rounded_box(fx - cx, fy - cy, half, radius);
                let d_outer = d_inner - ring;
                let d_shadow = rounded_box(fx - cx, fy - cy - 1.5 * s, half + ring, radius + ring);
                let shadow = {
                    let t = ((d_shadow + 0.5 * s) / (3.5 * s)).clamp(0.0, 1.0);
                    0.30 * (1.0 - t) * (1.0 - t)
                };
                let c_outer = (0.5 - d_outer).clamp(0.0, 1.0);
                let c_inner = (0.5 - d_inner).clamp(0.0, 1.0);
                // premultiplied (r, g, b, a) in 0..=255
                let mut c = (0.0f32, 0.0f32, 0.0f32, shadow * 255.0);
                c = over(c, (255.0, 255.0, 255.0), c_outer);
                c = over(c, accent, c_inner);
                c = over(c, (255.0, 255.0, 255.0), glyph * c_inner);
                px[i] = c.2.round() as u8;
                px[i + 1] = c.1.round() as u8;
                px[i + 2] = c.0.round() as u8;
                px[i + 3] = c.3.round() as u8;
            }
        }
    }
}

/// Signed distance from a point (relative to the centre) to a rounded square.
fn rounded_box(px: f32, py: f32, half: f32, r: f32) -> f32 {
    let qx = px.abs() - (half - r);
    let qy = py.abs() - (half - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Composite an opaque colour with coverage `a` over a premultiplied pixel.
fn over(dst: (f32, f32, f32, f32), src: (f32, f32, f32), a: f32) -> (f32, f32, f32, f32) {
    (
        src.0 * a + dst.0 * (1.0 - a),
        src.1 * a + dst.1 * (1.0 - a),
        src.2 * a + dst.2 * (1.0 - a),
        255.0 * a + dst.3 * (1.0 - a),
    )
}
