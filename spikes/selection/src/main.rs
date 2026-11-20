//! M0 spike: selection-text extraction without ever sending Ctrl+C.
//!
//! Detects drag-select / double-click via a low-level mouse hook, then tries:
//!   1. UI Automation TextPattern.GetSelection
//!   2. Ctrl+Insert with clipboard save/restore + sequence-number check
//! Run with `--both` to always try both methods (for the compatibility matrix).

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

#[derive(Debug, Clone, Copy)]
enum Gesture {
    Drag,
    DoubleClick,
}

static TX: OnceLock<Sender<Gesture>> = OnceLock::new();

struct HookState {
    down: POINT,
    down_t: u32,
    last_up: POINT,
    last_up_t: u32,
}
static mut HS: HookState = HookState {
    down: POINT { x: 0, y: 0 },
    down_t: 0,
    last_up: POINT { x: 0, y: 0 },
    last_up_t: 0,
};

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
            // ignore injected events (ours or other tools')
            if info.flags & LLMHF_INJECTED == 0 {
                let hs = &mut *std::ptr::addr_of_mut!(HS);
                match wp.0 as u32 {
                    WM_LBUTTONDOWN => {
                        hs.down = info.pt;
                        hs.down_t = info.time;
                    }
                    WM_LBUTTONUP => {
                        let dx = (info.pt.x - hs.down.x).abs();
                        let dy = (info.pt.y - hs.down.y).abs();
                        let g = if dx.max(dy) >= 8 {
                            Some(Gesture::Drag)
                        } else if info.time.wrapping_sub(hs.last_up_t) <= GetDoubleClickTime()
                            && (info.pt.x - hs.last_up.x).abs() <= 4
                            && (info.pt.y - hs.last_up.y).abs() <= 4
                        {
                            Some(Gesture::DoubleClick)
                        } else {
                            None
                        };
                        hs.last_up = info.pt;
                        hs.last_up_t = info.time;
                        if let (Some(g), Some(tx)) = (g, TX.get()) {
                            let _ = tx.send(g);
                        }
                    }
                    _ => {}
                }
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

fn foreground_info() -> (String, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut cls = [0u16; 256];
        let n = GetClassNameW(hwnd, &mut cls);
        let class = String::from_utf16_lossy(&cls[..n as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut name = String::from("?");
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok()
            {
                let full = String::from_utf16_lossy(&buf[..len as usize]);
                name = full.rsplit('\\').next().unwrap_or(&full).to_string();
            }
            let _ = CloseHandle(h);
        }
        (name, class)
    }
}

// ---------------- UIA ----------------

fn uia_selection(uia: &IUIAutomation) -> Result<String> {
    unsafe {
        let mut el = uia.GetFocusedElement()?;
        let walker = uia.ControlViewWalker()?;
        for _ in 0..6 {
            if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                let arr = tp.GetSelection()?;
                let mut out = String::new();
                for i in 0..arr.Length()? {
                    let r = arr.GetElement(i)?;
                    out.push_str(&r.GetText(-1)?.to_string());
                }
                return Ok(out);
            }
            match walker.GetParentElement(&el) {
                Ok(p) => el = p,
                Err(_) => break,
            }
        }
        Err(Error::new(E_NOINTERFACE, "no TextPattern in focus chain"))
    }
}

// ---------------- clipboard ----------------

struct SavedClipboard(Vec<(u32, Vec<u8>)>);

fn is_hglobal_format(f: u32) -> bool {
    // GDI-handle formats cannot be copied as raw memory; CF_DIB covers bitmaps.
    !matches!(f, 2 | 3 | 9 | 14 | 0x0080 | 0x0082 | 0x0083 | 0x008E)
}

fn open_clipboard_retry() -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn save_clipboard() -> SavedClipboard {
    let mut v = Vec::new();
    unsafe {
        if !open_clipboard_retry() {
            return SavedClipboard(v);
        }
        let mut f = EnumClipboardFormats(0);
        while f != 0 {
            if is_hglobal_format(f) {
                if let Ok(h) = GetClipboardData(f) {
                    let hg = HGLOBAL(h.0);
                    let size = GlobalSize(hg);
                    if size > 0 && size < 10 * 1024 * 1024 {
                        let p = GlobalLock(hg) as *const u8;
                        if !p.is_null() {
                            v.push((f, std::slice::from_raw_parts(p, size).to_vec()));
                            let _ = GlobalUnlock(hg);
                        }
                    }
                }
            }
            f = EnumClipboardFormats(f);
        }
        let _ = CloseClipboard();
    }
    SavedClipboard(v)
}

fn set_raw(f: u32, data: &[u8]) {
    unsafe {
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) {
            let p = GlobalLock(hg) as *mut u8;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
                let _ = GlobalUnlock(hg);
                let _ = SetClipboardData(f, Some(HANDLE(hg.0)));
            }
        }
    }
}

fn restore_clipboard(saved: &SavedClipboard) {
    unsafe {
        if !open_clipboard_retry() {
            return;
        }
        let _ = EmptyClipboard();
        for (f, data) in &saved.0 {
            set_raw(*f, data);
        }
        // keep the restore out of Win+V clipboard history
        let excl = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        set_raw(excl, &[0]);
        let _ = CloseClipboard();
    }
}

fn read_clipboard_text() -> Option<String> {
    unsafe {
        if !open_clipboard_retry() {
            return None;
        }
        let mut out = None;
        if let Ok(h) = GetClipboardData(13 /* CF_UNICODETEXT */) {
            let hg = HGLOBAL(h.0);
            let p = GlobalLock(hg) as *const u16;
            if !p.is_null() {
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

fn key(vk: VIRTUAL_KEY, up: bool, ext: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if ext {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn modifiers_down() -> bool {
    unsafe {
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|k| (GetAsyncKeyState(k.0 as i32) as u16 & 0x8000) != 0)
    }
}

fn ctrl_insert_copy() -> std::result::Result<String, String> {
    // wait for the user to release modifiers (Shift+click etc.); never inject Alt-up
    let t0 = Instant::now();
    while modifiers_down() {
        if t0.elapsed() > Duration::from_millis(1000) {
            return Err("modifier keys still held".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let saved = save_clipboard();
    let seq0 = unsafe { GetClipboardSequenceNumber() };
    let inputs = [
        key(VK_CONTROL, false, false),
        key(VK_INSERT, false, true),
        key(VK_INSERT, true, true),
        key(VK_CONTROL, true, false),
    ];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    let t1 = Instant::now();
    let mut changed = false;
    while t1.elapsed() < Duration::from_millis(300) {
        if unsafe { GetClipboardSequenceNumber() } != seq0 {
            changed = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !changed {
        return Err("clipboard unchanged (app ignored Ctrl+Insert or nothing selected)".into());
    }
    thread::sleep(Duration::from_millis(20)); // let the owner finish writing
    let text = read_clipboard_text();
    restore_clipboard(&saved);
    text.ok_or_else(|| "clipboard has no text".into())
}

fn preview(s: &str) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let mut p: String = one.chars().take(60).collect();
    if one.chars().count() > 60 {
        p.push('…');
    }
    format!("{:?} ({} chars)", p, s.chars().count())
}

fn worker(rx: std::sync::mpsc::Receiver<Gesture>, both: bool) {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().expect("CoInitializeEx") };
    let uia: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.expect("UIA");
    for g in rx {
        thread::sleep(Duration::from_millis(120)); // let double-click selection settle
        let (proc_name, class) = foreground_info();
        println!("\n[{proc_name} | {class}] {g:?}");
        let t = Instant::now();
        let u = uia_selection(&uia);
        let uia_ms = t.elapsed().as_millis();
        let uia_ok = matches!(&u, Ok(s) if !s.trim().is_empty());
        match &u {
            Ok(s) => println!("  UIA        {uia_ms:4}ms  {}", preview(s)),
            Err(e) => println!("  UIA        {uia_ms:4}ms  ERR {}", e.message()),
        }
        if both || !uia_ok {
            let t = Instant::now();
            let c = ctrl_insert_copy();
            let ms = t.elapsed().as_millis();
            match c {
                Ok(s) => println!("  Ctrl+Ins   {ms:4}ms  {}", preview(&s)),
                Err(e) => println!("  Ctrl+Ins   {ms:4}ms  FAIL {e}"),
            }
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--term-test") {
        termtest::run();
        return;
    }
    let both = std::env::args().any(|a| a == "--both");
    println!("selection spike running (both={both}). Select text anywhere; Ctrl+Break to quit.");
    let (tx, rx) = channel();
    TX.set(tx).unwrap();
    thread::spawn(move || worker(rx, both));
    unsafe {
        let _hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).expect("hook");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---------------- automated terminal safety test ----------------
mod termtest {
    use super::*;
    use std::collections::HashSet;
    use std::process::Command;
    use windows::Win32::UI::HiDpi::*;

    fn ping_pids() -> HashSet<u32> {
        let out = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq PING.EXE"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split(',').nth(1))
            .filter_map(|p| p.trim_matches('"').parse().ok())
            .collect()
    }

    fn alive(pid: u32) -> bool {
        ping_pids().contains(&pid)
    }

    fn windows_of_class(class: &str) -> HashSet<isize> {
        struct Ctx<'a> {
            class: &'a str,
            out: HashSet<isize>,
        }
        unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
            unsafe {
                let ctx = &mut *(lp.0 as *mut Ctx);
                if IsWindowVisible(h).as_bool() {
                    let mut buf = [0u16; 256];
                    let n = GetClassNameW(h, &mut buf);
                    if String::from_utf16_lossy(&buf[..n as usize]) == ctx.class {
                        ctx.out.insert(h.0 as isize);
                    }
                }
                TRUE
            }
        }
        let mut ctx = Ctx { class, out: HashSet::new() };
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.out
    }

    fn mouse(flags: MOUSE_EVENT_FLAGS) {
        let i = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        unsafe { SendInput(&[i], std::mem::size_of::<INPUT>() as i32) };
    }

    fn move_abs(x: i32, y: i32) {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let i = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: (x - vx) * 65535 / (vw - 1),
                        dy: (y - vy) * 65535 / (vh - 1),
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[i], std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn drag(x1: i32, y1: i32, x2: i32, y2: i32) {
        move_abs(x1, y1);
        thread::sleep(Duration::from_millis(80));
        mouse(MOUSEEVENTF_LEFTDOWN);
        thread::sleep(Duration::from_millis(50));
        for s in 1..=15 {
            move_abs(x1 + (x2 - x1) * s / 15, y1 + (y2 - y1) * s / 15);
            thread::sleep(Duration::from_millis(25));
        }
        thread::sleep(Duration::from_millis(50));
        mouse(MOUSEEVENTF_LEFTUP);
    }

    fn send_keys(keys: &[INPUT]) {
        unsafe { SendInput(keys, std::mem::size_of::<INPUT>() as i32) };
    }

    struct Case {
        name: &'static str,
        exe: &'static str,
        args: Vec<&'static str>,
        class: &'static str,
        control_ctrl_c: bool,
    }

    pub fn run() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            // restore normal Ctrl+C processing; children inherit this attribute
            let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(None, false);
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        }
        let uia: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.unwrap();
        let cases = vec![
            Case { name: "conhost (cmd)", exe: "powershell.exe", args: vec!["-NoProfile", "-Command", "Start-Process conhost.exe -ArgumentList 'cmd /k ping -t 127.0.0.1'"], class: "ConsoleWindowClass", control_ctrl_c: false },
            Case { name: "Windows Terminal (cmd)", exe: "wt.exe", args: vec!["-w", "new", "cmd", "/k", "ping -t 127.0.0.1"], class: "CASCADIA_HOSTING_WINDOW_CLASS", control_ctrl_c: false },
            Case { name: "Windows Terminal (pwsh)", exe: "wt.exe", args: vec!["-w", "new", "powershell", "-NoLogo", "-NoExit", "-Command", "ping -t 127.0.0.1"], class: "CASCADIA_HOSTING_WINDOW_CLASS", control_ctrl_c: false },
            Case { name: "Git Bash (mintty)", exe: r"D:\LenovoSoftstore\Install\Git\usr\bin\mintty.exe", args: vec!["-h", "always", "-e", "/usr/bin/bash", "-lc", "ping -t 127.0.0.1"], class: "mintty", control_ctrl_c: false },
        ];
        let filter = std::env::args().nth(2).unwrap_or_default().to_lowercase();
        for c in cases {
            if !c.name.to_lowercase().contains(&filter) {
                continue;
            }
            println!("\n=== {} ===", c.name);
            let pings0 = ping_pids();
            let wins0 = windows_of_class(c.class);
            use std::os::windows::process::CommandExt;
            // CREATE_NEW_CONSOLE so conhost gets its own window instead of inheriting ours
            if let Err(e) = Command::new(c.exe).args(&c.args).creation_flags(0x0000_0010).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
                println!("  launch failed: {e}");
                continue;
            }
            thread::sleep(Duration::from_secs(7));
            let Some(pid) = ping_pids().difference(&pings0).copied().next() else {
                println!("  no new ping.exe found");
                continue;
            };
            let Some(hwnd) = windows_of_class(c.class).difference(&wins0).copied().next() else {
                println!("  no new window found");
                continue;
            };
            let hwnd = HWND(hwnd as *mut _);
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            thread::sleep(Duration::from_millis(500));
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut r);
            }
            let (w, h) = (r.right - r.left, r.bottom - r.top);

            // focus-click on an empty area so keystrokes really reach the terminal
            move_abs(r.left + w / 2, r.top + h * 85 / 100);
            thread::sleep(Duration::from_millis(50));
            mouse(MOUSEEVENTF_LEFTDOWN);
            mouse(MOUSEEVENTF_LEFTUP);
            thread::sleep(Duration::from_millis(500));
            let fg_ok = unsafe { GetForegroundWindow() } == hwnd;
            println!("  terminal is foreground: {fg_ok}");

            if c.control_ctrl_c {
                send_keys(&[key(VK_CONTROL, false, false), key(VirtualKey(0x43), false, false), key(VirtualKey(0x43), true, false), key(VK_CONTROL, true, false)]);
                thread::sleep(Duration::from_millis(1500));
                println!("  Ctrl+C (no selection)          -> ping alive: {}", alive(pid));
            } else {
                // 1) Ctrl+Insert with NO selection
                let r1 = ctrl_insert_copy();
                thread::sleep(Duration::from_millis(1500));
                println!("  Ctrl+Ins, no selection: {:<40} -> ping alive: {}", format!("{:?}", r1.as_ref().map(|s| preview(s))).chars().take(40).collect::<String>(), alive(pid));
                // 2) focus-click on an empty area, then drag-select, then UIA + Ctrl+Insert
                drag(r.left + w / 30, r.top + h * 16 / 100, r.left + w * 6 / 10, r.top + h * 24 / 100);
                thread::sleep(Duration::from_millis(300));
                let u = uia_selection(&uia);
                println!("  UIA after drag:     {}", match &u { Ok(s) => preview(s), Err(e) => format!("ERR {}", e.message()) });
                let r2 = ctrl_insert_copy();
                println!("  Ctrl+Ins after drag: {}", match &r2 { Ok(s) => preview(s), Err(e) => format!("FAIL {e}") });
                thread::sleep(Duration::from_millis(1500));
                println!("  -> ping alive after selection tests: {}", alive(pid));
                // CONTROL: clear selection, send real Ctrl+C -> ping must die (proves the test is sensitive)
                move_abs(r.left + w / 2, r.top + h * 85 / 100);
                thread::sleep(Duration::from_millis(50));
                mouse(MOUSEEVENTF_LEFTDOWN);
                mouse(MOUSEEVENTF_LEFTUP);
                thread::sleep(Duration::from_millis(400));
                // Esc clears conhost QuickEdit's zero-width selection left by the click
                send_keys(&[key(VK_ESCAPE, false, false), key(VK_ESCAPE, true, false)]);
                thread::sleep(Duration::from_millis(300));
                send_keys(&[key(VK_CONTROL, false, false), key(VirtualKey(0x43), false, false), key(VirtualKey(0x43), true, false), key(VK_CONTROL, true, false)]);
                thread::sleep(Duration::from_millis(1500));
                println!("  CONTROL Ctrl+C -> ping alive: {} (expected false)", alive(pid));
            }
            // cleanup
            let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
            thread::sleep(Duration::from_millis(500));
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            thread::sleep(Duration::from_millis(800));
        }
    }

    #[allow(non_snake_case)]
    fn VirtualKey(v: u16) -> VIRTUAL_KEY {
        VIRTUAL_KEY(v)
    }
}
