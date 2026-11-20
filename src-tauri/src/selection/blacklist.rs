//! Which foreground windows get the selection icon: not our own windows, not
//! blacklisted processes, not the desktop/taskbar, not full-screen apps.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, TRUE};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::{QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PWSTR};

/// Shell windows where selecting text is not a thing (desktop icons, taskbar).
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Executable file name (`notepad.exe`) of a process.
pub fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(full.rsplit('\\').next().unwrap_or(&full).to_string())
    }
}

/// A D3D exclusive full-screen app or presentation mode is running.
pub fn fullscreen_exclusive() -> bool {
    matches!(unsafe { SHQueryUserNotificationState() }, Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE)
}

/// Why the icon is not shown for a foreground window.
#[derive(Debug, PartialEq, Eq)]
pub enum Skip {
    OwnApp,
    Shell,
    Blacklisted(String),
    FullScreen,
}

/// Lower-cased process names.
pub type Blacklist = HashSet<String>;

pub fn check(fg: HWND, blacklist: &Blacklist) -> Result<(), Skip> {
    let pid = window_pid(fg);
    if pid == std::process::id() {
        return Err(Skip::OwnApp);
    }
    if SHELL_CLASSES.contains(&class_name(fg).as_str()) {
        return Err(Skip::Shell);
    }
    if !blacklist.is_empty() {
        if let Some(name) = process_name(pid) {
            if blacklist.contains(&name.to_lowercase()) {
                return Err(Skip::Blacklisted(name));
            }
        }
    }
    if fullscreen_exclusive() {
        return Err(Skip::FullScreen);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowProcess {
    /// Executable name, e.g. `WINWORD.EXE`.
    pub exe: String,
    /// Title of one of its visible windows.
    pub title: String,
}

/// Processes that currently own a visible, titled top-level window (for the
/// blacklist picker), one entry per executable, sorted by name.
pub fn window_processes() -> Vec<WindowProcess> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(lp.0 as *mut Vec<(u32, String)>);
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() && ex & WS_EX_TOOLWINDOW.0 == 0 {
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(h, &mut buf);
                if n > 0 {
                    out.push((window_pid(h), String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
            TRUE
        }
    }
    let mut wins: Vec<(u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut wins as *mut _ as isize));
    }
    let me = std::process::id();
    let mut by_exe: BTreeMap<String, WindowProcess> = BTreeMap::new();
    for (pid, title) in wins {
        if pid == me {
            continue;
        }
        let Some(exe) = process_name(pid) else { continue };
        by_exe.entry(exe.to_lowercase()).or_insert(WindowProcess { exe, title });
    }
    by_exe.into_values().collect()
}
