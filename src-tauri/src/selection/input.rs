//! Synthetic keyboard input and focus helpers.

use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

pub fn key(vk: VIRTUAL_KEY, up: bool, extended: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

pub fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// Make sure no modifier is held before injecting Ctrl+Insert, so the target
/// does not see e.g. Ctrl+Shift+Insert. Waits up to `wait` for the user to let
/// go, then injects key-ups for Shift / Ctrl / Win. Alt is never released by
/// injection (a lone Alt-up opens the menu bar in many apps); if Alt is still
/// held this returns false and the caller gives up.
pub fn release_modifiers(wait: Duration) -> bool {
    let t0 = Instant::now();
    let mods = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN];
    while mods.iter().any(|&k| is_down(k)) && t0.elapsed() < wait {
        thread::sleep(Duration::from_millis(10));
    }
    if is_down(VK_MENU) {
        return false;
    }
    let ups: Vec<INPUT> = [
        (VK_LSHIFT, false),
        (VK_RSHIFT, false),
        (VK_LCONTROL, false),
        (VK_RCONTROL, true),
        (VK_LWIN, true),
        (VK_RWIN, true),
    ]
    .into_iter()
    .filter(|&(k, _)| is_down(k))
    .map(|(k, ext)| key(k, true, ext))
    .collect();
    if !ups.is_empty() {
        send(&ups);
        thread::sleep(Duration::from_millis(15));
    }
    true
}

/// Bring `hwnd` to the foreground from a background process. Plain
/// SetForegroundWindow is refused unless we received the last input event, so
/// fall back to briefly attaching to the foreground thread's input queue.
pub fn force_foreground(hwnd: HWND) -> bool {
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            return true;
        }
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        GetForegroundWindow() == hwnd
    }
}
