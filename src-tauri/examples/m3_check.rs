//! M3 acceptance check (DESIGN 9): selecting text in a terminal that runs
//! `ping -t` must never terminate it.
//!
//! cargo run --example m3_check [-- capture|e2e [filter]]
//!
//! * `capture` (default): calls the app's capture code (UIA, then
//!   Ctrl+Insert) directly in conhost and Windows Terminal (cmd, PowerShell).
//! * `e2e`: drives the running app end to end. Start it with
//!   `LLMT_ACCEPT_INJECTED=1` (debug build) so the hook accepts the injected
//!   drag; the check then clicks the selection icon and expects the popup.
//!
//! Each case ends with a real Ctrl+C as a control: it must kill ping, which
//! proves the test would notice an interrupt.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use llm_translate_lib::selection::{self, input, uia::Uia};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, w};

fn ping_pids() -> HashSet<u32> {
    let out = Command::new("tasklist").args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq PING.EXE"]).output().unwrap();
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
            if IsWindowVisible(h).as_bool() && selection::blacklist::class_name(h) == ctx.class {
                ctx.out.insert(h.0 as isize);
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
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
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

fn click_at(x: i32, y: i32) {
    move_abs(x, y);
    thread::sleep(Duration::from_millis(60));
    mouse(MOUSEEVENTF_LEFTDOWN);
    thread::sleep(Duration::from_millis(30));
    mouse(MOUSEEVENTF_LEFTUP);
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

fn tap(vk: VIRTUAL_KEY) {
    input::send(&[input::key(vk, false, false), input::key(vk, true, false)]);
}

fn ctrl_c() {
    let c = VIRTUAL_KEY(0x43);
    input::send(&[
        input::key(VK_CONTROL, false, false),
        input::key(c, false, false),
        input::key(c, true, false),
        input::key(VK_CONTROL, true, false),
    ]);
}

fn preview(s: &str) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let p: String = one.chars().take(48).collect();
    format!("{p:?} ({} chars)", s.chars().count())
}

struct Case {
    name: &'static str,
    exe: &'static str,
    args: &'static [&'static str],
    class: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "conhost (cmd)",
        exe: "powershell.exe",
        args: &["-NoProfile", "-Command", "Start-Process conhost.exe -ArgumentList 'cmd /k ping -t 127.0.0.1'"],
        class: "ConsoleWindowClass",
    },
    Case {
        name: "Windows Terminal (cmd)",
        exe: "wt.exe",
        args: &["-w", "new", "cmd", "/k", "ping -t 127.0.0.1"],
        class: "CASCADIA_HOSTING_WINDOW_CLASS",
    },
    Case {
        name: "Windows Terminal (PowerShell)",
        exe: "wt.exe",
        args: &["-w", "new", "powershell", "-NoLogo", "-NoExit", "-Command", "ping -t 127.0.0.1"],
        class: "CASCADIA_HOSTING_WINDOW_CLASS",
    },
];

fn visible_window(class: &str, title: Option<&str>) -> Option<(HWND, RECT)> {
    let class_w: Vec<u16> = class.encode_utf16().chain([0]).collect();
    let title_w: Option<Vec<u16>> = title.map(|t| t.encode_utf16().chain([0]).collect());
    unsafe {
        let h = FindWindowW(
            windows::core::PCWSTR(class_w.as_ptr()),
            title_w.as_ref().map_or(windows::core::PCWSTR::null(), |t| windows::core::PCWSTR(t.as_ptr())),
        )
        .ok()?;
        if !IsWindowVisible(h).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        Some((h, r))
    }
}

/// Returns whether the case passed.
fn run_case(c: &Case, e2e: bool, uia: &Uia) -> bool {
    println!("\n=== {} ===", c.name);
    let pings0 = ping_pids();
    let wins0 = windows_of_class(c.class);
    // CREATE_NEW_CONSOLE so conhost gets its own window.
    if let Err(e) = Command::new(c.exe)
        .args(c.args)
        .creation_flags(0x0000_0010)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        println!("  launch failed: {e}");
        return false;
    }
    thread::sleep(Duration::from_secs(6));
    let Some(pid) = ping_pids().difference(&pings0).copied().next() else {
        println!("  no new ping.exe");
        return false;
    };
    let Some(hwnd) = windows_of_class(c.class).difference(&wins0).copied().next() else {
        println!("  no new window");
        let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
        return false;
    };
    let hwnd = HWND(hwnd as *mut _);
    input::force_foreground(hwnd);
    thread::sleep(Duration::from_millis(400));
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
    }
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let blank = (r.left + w / 2, r.top + h * 85 / 100);
    click_at(blank.0, blank.1);
    thread::sleep(Duration::from_millis(400));
    // conhost QuickEdit: a plain click leaves an empty selection; Esc clears it.
    tap(VK_ESCAPE);
    thread::sleep(Duration::from_millis(300));

    let mut ok = true;
    // 1) Ctrl+Insert with nothing selected.
    let r1 = selection::clipboard::copy_selection(None);
    thread::sleep(Duration::from_millis(1200));
    let a = alive(pid);
    println!("  Ctrl+Insert, no selection: {:?} -> ping alive: {a}", r1.as_ref().map(|s| preview(s)));
    ok &= a;

    // 2) Drag-select a line of ping output, then capture.
    drag(r.left + w / 30, r.top + h * 16 / 100, r.left + w * 6 / 10, r.top + h * 24 / 100);
    thread::sleep(Duration::from_millis(250));
    if e2e {
        match visible_window("LLMTranslateSelectionIcon", None) {
            None => {
                println!("  selection icon did not appear");
                ok = false;
            }
            Some((_, ir)) => {
                click_at((ir.left + ir.right) / 2, (ir.top + ir.bottom) / 2);
                thread::sleep(Duration::from_millis(900));
                let popup = visible_window("Tauri Window", Some("LLM Translate · 划词翻译"));
                println!("  icon clicked -> popup visible: {}", popup.is_some());
                ok &= popup.is_some();
                if let Some((ph, _)) = popup {
                    let fg = unsafe { GetForegroundWindow() } == ph;
                    println!("  popup is foreground: {fg}");
                    ok &= fg;
                    thread::sleep(Duration::from_millis(1500));
                    tap(VK_ESCAPE); // closes the popup
                    thread::sleep(Duration::from_millis(300));
                }
            }
        }
    } else {
        let (text, method) = selection::capture_text(Some(uia), None);
        println!("  capture: {:?} {:?}", method, text.as_deref().map(preview));
        ok &= text.is_some();
        // Same selection through the Ctrl+Insert fallback only; the user's
        // clipboard must come back unchanged.
        let ps = |cmd: &str| Command::new("powershell").args(["-NoProfile", "-Command", cmd]).output().unwrap();
        ps("Set-Clipboard -Value 'm3-clipboard-marker'");
        let (text, method) = selection::capture_text(None, None);
        println!("  capture without UIA: {:?} {:?}", method, text.as_deref().map(preview));
        ok &= text.is_some();
        let clip = String::from_utf8_lossy(&ps("Get-Clipboard").stdout).trim().to_string();
        println!("  clipboard restored: {}", clip == "m3-clipboard-marker");
        ok &= clip == "m3-clipboard-marker";
    }
    thread::sleep(Duration::from_millis(1200));
    let a = alive(pid);
    println!("  -> ping alive after selection: {a}");
    ok &= a;

    // Control: clear the selection and send a real Ctrl+C; ping must die.
    input::force_foreground(hwnd);
    thread::sleep(Duration::from_millis(300));
    click_at(blank.0, blank.1);
    thread::sleep(Duration::from_millis(400));
    tap(VK_ESCAPE);
    thread::sleep(Duration::from_millis(300));
    ctrl_c();
    thread::sleep(Duration::from_millis(1500));
    let killed = !alive(pid);
    println!("  CONTROL real Ctrl+C -> ping terminated: {killed} (expected true)");
    ok &= killed;

    let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
    thread::sleep(Duration::from_millis(400));
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    thread::sleep(Duration::from_millis(800));
    println!("  {}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "capture".into());
    let filter = std::env::args().nth(2).unwrap_or_default().to_lowercase();
    let e2e = mode == "e2e";
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Children inherit Ctrl+C handling; make sure it is enabled for them.
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(None, false);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
    }
    if e2e && unsafe { FindWindowW(w!("LLMTranslateSelectionIcon"), None) }.is_err() {
        eprintln!("e2e: the app is not running (no selection icon window)");
        std::process::exit(2);
    }
    let uia = Uia::new().expect("UI Automation");
    let mut failed = 0;
    for c in CASES.iter().filter(|c| c.name.to_lowercase().contains(&filter)) {
        if !run_case(c, e2e, &uia) {
            failed += 1;
        }
    }
    println!("\n{}", if failed == 0 { "ALL PASS".to_string() } else { format!("{failed} case(s) FAILED") });
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
