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

/// Mandatory integrity level of a process (0x2000 medium, 0x3000 high).
/// `Err(true)`: the process exists but its token is closed to us, which for a
/// same-user process means it runs elevated. `Err(false)`: unknown.
fn integrity_level(pid: u32) -> Result<u32, bool> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        TokenIntegrityLevel,
    };
    use windows::Win32::System::Threading::OpenProcessToken;
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|_| false)?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        opened.map_err(|_| true)?;
        // TOKEN_MANDATORY_LABEL plus the SID it points to; 64 bytes is plenty.
        let mut buf = [0u64; 8];
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr().cast()),
            std::mem::size_of_val(&buf) as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.map_err(|_| false)?;
        let sid = (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(false);
        }
        Ok(*GetSidSubAuthority(sid, count as u32 - 1))
    }
}

/// The process runs at a higher integrity level than this one (typically "Run
/// as administrator"). UIPI then hides its mouse input from our low-level hook
/// and blocks the simulated copy.
pub fn runs_elevated_above_us(pid: u32) -> bool {
    use std::sync::OnceLock;
    static OWN: OnceLock<Option<u32>> = OnceLock::new();
    let Some(own) = *OWN.get_or_init(|| integrity_level(std::process::id()).ok()) else {
        return false;
    };
    match integrity_level(pid) {
        Ok(level) => level > own,
        Err(closed) => closed && own < 0x3000,
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
