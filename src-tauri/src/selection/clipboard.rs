//! Copy the selection with Ctrl+Insert (DESIGN 5.5 step 2). Ctrl+Insert never
//! produces the 0x03 interrupt that Ctrl+C sends to console programs, so it is
//! safe in every terminal. The user's clipboard is saved and restored.

use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_INSERT};
use windows::core::w;

use super::input;

const CF_UNICODETEXT: u32 = 13;
const MAX_FORMAT_BYTES: usize = 10 * 1024 * 1024;
const POLL_TIMEOUT: Duration = Duration::from_millis(300);

/// Raw HGLOBAL contents of every copyable clipboard format.
struct Saved(Vec<(u32, Vec<u8>)>);

/// GDI-handle formats cannot be copied as raw memory (CF_DIB covers bitmaps);
/// private/GDI-object ranges are skipped too.
fn is_hglobal_format(f: u32) -> bool {
    !matches!(f, 2 | 3 | 9 | 14 | 0x0080 | 0x0082 | 0x0083 | 0x008E) && !(0x0300..=0x03FF).contains(&f)
}

/// Clipboard opened for the duration of the guard. `owner` must be a window of
/// a thread that pumps messages, or other apps' EmptyClipboard calls would
/// block on our WM_DESTROYCLIPBOARD.
struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> Option<Self> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Some(Open);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn save(owner: Option<HWND>) -> Saved {
    let mut out = Vec::new();
    let Some(_open) = Open::new(owner) else { return Saved(out) };
    unsafe {
        let mut f = EnumClipboardFormats(0);
        while f != 0 {
            if is_hglobal_format(f) {
                if let Ok(h) = GetClipboardData(f) {
                    let hg = HGLOBAL(h.0);
                    let size = GlobalSize(hg);
                    if size > 0 && size < MAX_FORMAT_BYTES {
                        let p = GlobalLock(hg) as *const u8;
                        if !p.is_null() {
                            out.push((f, std::slice::from_raw_parts(p, size).to_vec()));
                            let _ = GlobalUnlock(hg);
                        }
                    }
                }
            }
            f = EnumClipboardFormats(f);
        }
    }
    Saved(out)
}

fn set_raw(f: u32, data: &[u8]) {
    unsafe {
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) else { return };
        let p = GlobalLock(hg) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(hg));
            return;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        let _ = GlobalUnlock(hg);
        if SetClipboardData(f, Some(HANDLE(hg.0))).is_err() {
            let _ = GlobalFree(Some(hg));
        }
    }
}

/// Replace the clipboard with raw HGLOBAL formats (e.g. a screenshot as
/// CF_DIB). `owner` must be a window whose thread pumps messages.
pub fn set_contents(owner: Option<HWND>, items: &[(u32, Vec<u8>)]) -> bool {
    let Some(_open) = Open::new(owner) else { return false };
    unsafe {
        let _ = EmptyClipboard();
    }
    for (f, data) in items {
        set_raw(*f, data);
    }
    true
}

/// Put the saved contents back, marked so Win+V history does not record the
/// restore as a new entry.
fn restore(owner: Option<HWND>, saved: &Saved) {
    let Some(_open) = Open::new(owner) else {
        tracing::warn!("could not reopen the clipboard to restore it");
        return;
    };
    unsafe {
        let _ = EmptyClipboard();
        for (f, data) in &saved.0 {
            set_raw(*f, data);
        }
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        set_raw(exclude, &[0]);
    }
}

fn read_text(owner: Option<HWND>) -> Option<String> {
    let _open = Open::new(owner)?;
    unsafe {
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if p.is_null() {
            return None;
        }
        let max = GlobalSize(hg) / 2;
        let len = (0..max).take_while(|&i| *p.add(i) != 0).count();
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        let _ = GlobalUnlock(hg);
        Some(s)
    }
}

#[derive(Debug)]
pub enum CopyError {
    /// Alt still held; injecting would combine with it.
    ModifierHeld,
    /// The app ignored Ctrl+Insert or nothing was selected.
    Unchanged,
    /// Clipboard changed but held no text (e.g. conhost's empty selection).
    Empty,
}

/// Send Ctrl+Insert to the focused window and read what it copied.
pub fn copy_selection(owner: Option<HWND>) -> Result<String, CopyError> {
    if !input::release_modifiers(Duration::from_millis(300)) {
        return Err(CopyError::ModifierHeld);
    }
    let saved = save(owner);
    let seq0 = unsafe { GetClipboardSequenceNumber() };
    input::send(&[
        input::key(VK_CONTROL, false, false),
        input::key(VK_INSERT, false, true),
        input::key(VK_INSERT, true, true),
        input::key(VK_CONTROL, true, false),
    ]);
    let t0 = Instant::now();
    let mut changed = false;
    while t0.elapsed() < POLL_TIMEOUT {
        if unsafe { GetClipboardSequenceNumber() } != seq0 {
            changed = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !changed {
        return Err(CopyError::Unchanged);
    }
    // Let the owner finish writing all its formats.
    thread::sleep(Duration::from_millis(20));
    let text = read_text(owner);
    restore(owner, &saved);
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t),
        _ => Err(CopyError::Empty),
    }
}
