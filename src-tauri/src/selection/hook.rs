//! Hook thread (DESIGN 5.5): a low-level mouse hook plus the icon window, both
//! on one thread with a message loop. Hook callbacks only do arithmetic on
//! atomics and post messages to the icon window; gesture decisions run in the
//! window procedure, so the callback never blocks (Windows silently removes
//! hooks that time out). Read-only UIA probing and text capture run on the
//! worker thread in `mod.rs`.
//!
//! A once-a-second watch timer on the same thread reinstalls the mouse hook
//! when Windows has dropped it.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, sync_channel};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::settings::FilterMode;

use super::blacklist::{self, Filter};
use super::icon::{Icon, scale_at};
use super::ScreenRect;

const MSG_DOWN: u32 = WM_APP + 1;
const MSG_UP: u32 = WM_APP + 2;
const MSG_HOVER: u32 = WM_APP + 3;
const MSG_HIDE: u32 = WM_APP + 4;
const MSG_CONFIG: u32 = WM_APP + 5;
const MSG_PROBE_DONE: u32 = WM_APP + 6;

const HIDE_TIMER: usize = 1;
const HIDE_AFTER_MS: u32 = 3000;
const WATCH_TIMER: usize = 2;
const WATCH_EVERY_MS: u32 = 1000;
/// At most one hook reinstall per this interval.
const REINSTALL_GAP: Duration = Duration::from_secs(5);
/// Minimum drag distance in DIPs.
const DRAG_DIP: f32 = 8.0;
/// Moving this far (DIPs) from the icon hides it.
const FAR_DIP: f32 = 120.0;

const MENU_CMD_EXCLUDE: i32 = 1;
const MENU_CMD_DISABLE: i32 = 2;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub enabled: bool,
    pub filter: Filter,
}

/// Request sent from the hook thread to the worker thread to probe UIA before
/// showing the icon.
#[derive(Debug, Clone)]
pub struct ProbeReq {
    pub seq: u64,
    pub fg_hwnd: isize,
    pub down_pt: POINT,
    pub up_pt: POINT,
    pub anchor: ScreenRect,
    pub margin_px: i32,
    pub app: Option<String>,
    pub is_web_engine: bool,
    pub ibeam: bool,
}

/// Approved probe result sent back from the worker thread to show the icon.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub seq: u64,
    pub up_pt: POINT,
    pub anchor: ScreenRect,
    pub app: Option<String>,
    /// Pre-captured UIA text when the probe already read a non-empty selection.
    pub cached_text: Option<String>,
}

/// Request sent to the worker thread when the user left-clicks the icon.
#[derive(Debug, Clone)]
pub struct CaptureReq {
    pub anchor: ScreenRect,
    pub app: Option<String>,
    pub cached_text: Option<String>,
}

/// Action triggered from the icon's right-click context menu.
#[derive(Debug, Clone)]
pub enum IconAction {
    ExcludeApp(String),
    DisableSelection,
}

#[derive(Debug, Clone)]
pub enum WorkerJob {
    Probe(ProbeReq),
    Capture(CaptureReq),
    Action(IconAction),
}

// ---- state shared with the hook callbacks (same thread, but callbacks must
// not touch the RefCell'd tracker) ----
static TARGET: AtomicIsize = AtomicIsize::new(0);
static ICON_VISIBLE: AtomicBool = AtomicBool::new(false);
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicBool = AtomicBool::new(false);
static ICON_RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static FAR_PX: AtomicI32 = AtomicI32::new(120);
/// Mouse hook invocations, for the watchdog.
static MOUSE_CALLS: AtomicU32 = AtomicU32::new(0);
/// Monotonic gesture sequence; bumped whenever a gesture starts or is cancelled
/// so stale async UIA probe results are discarded.
static GESTURE_SEQ: AtomicU64 = AtomicU64::new(0);
/// Accept injected mouse input as gestures (automated tests only).
pub static ACCEPT_INJECTED: AtomicBool = AtomicBool::new(false);

pub fn gesture_seq() -> u64 {
    GESTURE_SEQ.load(Relaxed)
}

fn bump_seq() -> u64 {
    GESTURE_SEQ.fetch_add(1, Relaxed) + 1
}

fn post(msg: u32, wp: usize, lp: isize) {
    let h = TARGET.load(Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(wp), LPARAM(lp));
        }
    }
}

/// Wake the hook thread after sending a [`ProbeResult`] on the result channel.
pub fn notify_probe_done() {
    post(MSG_PROBE_DONE, 0, 0);
}

fn pack(pt: POINT) -> isize {
    ((pt.y as u32 as u64) << 32 | pt.x as u32 as u64) as isize
}

fn unpack(lp: LPARAM) -> POINT {
    let v = lp.0 as u64;
    POINT { x: v as u32 as i32, y: (v >> 32) as u32 as i32 }
}

fn icon_rect() -> RECT {
    RECT {
        left: ICON_RECT[0].load(Relaxed),
        top: ICON_RECT[1].load(Relaxed),
        right: ICON_RECT[2].load(Relaxed),
        bottom: ICON_RECT[3].load(Relaxed),
    }
}

fn contains(r: &RECT, pt: POINT) -> bool {
    pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

fn distance(r: &RECT, pt: POINT) -> i32 {
    let dx = (r.left - pt.x).max(pt.x - r.right).max(0);
    let dy = (r.top - pt.y).max(pt.y - r.bottom).max(0);
    dx.max(dy)
}

/// Hide the icon and invalidate any in-flight probe from a hook callback.
fn request_hide() {
    bump_seq();
    if ICON_VISIBLE.swap(false, Relaxed) {
        post(MSG_HIDE, 0, 0);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        MOUSE_CALLS.fetch_add(1, Relaxed);
        if code == HC_ACTION as i32 && !MENU_OPEN.load(Relaxed) {
            let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
            let injected = info.flags & LLMHF_INJECTED != 0;
            if !injected || ACCEPT_INJECTED.load(Relaxed) {
                match wp.0 as u32 {
                    m @ (WM_LBUTTONDOWN | WM_LBUTTONUP) => {
                        post(if m == WM_LBUTTONDOWN { MSG_DOWN } else { MSG_UP }, 0, pack(info.pt));
                    }
                    WM_MOUSEMOVE if ICON_VISIBLE.load(Relaxed) => {
                        let r = icon_rect();
                        let inside = contains(&r, info.pt);
                        if HOVER.swap(inside, Relaxed) != inside {
                            post(MSG_HOVER, inside as usize, 0);
                        }
                        if distance(&r, info.pt) > FAR_PX.load(Relaxed) {
                            request_hide();
                        }
                    }
                    WM_RBUTTONDOWN => {
                        // Right-clicking on the visible icon opens its context
                        // menu; right-clicking anywhere else cancels/hides it.
                        let on_icon = ICON_VISIBLE.load(Relaxed) && contains(&icon_rect(), info.pt);
                        if !on_icon {
                            request_hide();
                        }
                    }
                    WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                        request_hide();
                    }
                    _ => {}
                }
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

/// Installed only while the icon is visible: any real non-modifier key press
/// hides it. Modifiers alone do not (e.g. pressing Ctrl before Ctrl+C).
unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && !MENU_OPEN.load(Relaxed) && matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            let info = &*(lp.0 as *const KBDLLHOOKSTRUCT);
            let vk = VIRTUAL_KEY(info.vkCode as u16);
            let modifier = matches!(
                vk,
                VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU
                    | VK_RMENU | VK_LWIN | VK_RWIN
            );
            if info.flags & LLKHF_INJECTED == KBDLLHOOKSTRUCT_FLAGS(0) && !modifier {
                request_hide();
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

struct Down {
    pt: POINT,
    /// The press was under the standard I-beam cursor.
    ibeam: bool,
    /// Hard non-text cursor at the press (resize sash, crosshair, hidden...).
    hard_non_text: Option<&'static str>,
    /// Top-level window under the press and its rectangle, to tell a window
    /// move/resize from a text drag.
    root: HWND,
    rect: RECT,
}

struct Pending {
    anchor: ScreenRect,
    app: Option<String>,
    cached_text: Option<String>,
}

/// Watchdog state, sampled every `WATCH_EVERY_MS`.
#[derive(Default)]
struct Watch {
    pos: POINT,
    calls: u32,
    input: u32,
    last_reinstall: Option<Instant>,
    reinstalls: u32,
}

struct Tracker {
    icon: Icon,
    cfg: Arc<RwLock<Config>>,
    jobs: Sender<WorkerJob>,
    probe_rx: Receiver<ProbeResult>,
    mouse_hook: Option<HHOOK>,
    kb_hook: Option<HHOOK>,
    down: Option<Down>,
    pending: Option<Pending>,
    watch: Watch,
}

thread_local! {
    static TRACKER: RefCell<Option<Tracker>> = const { RefCell::new(None) };
}

fn window_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

impl Tracker {
    fn apply_config(&mut self) {
        let enabled = self.cfg.read().unwrap().enabled;
        match (enabled, self.mouse_hook) {
            (true, None) => match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) } {
                Ok(h) => {
                    tracing::info!("selection hook installed");
                    self.mouse_hook = Some(h);
                    self.sample_watch();
                    unsafe {
                        SetTimer(Some(self.icon.hwnd), WATCH_TIMER, WATCH_EVERY_MS, None);
                    }
                }
                Err(e) => tracing::error!("installing the mouse hook failed: {e}"),
            },
            (false, Some(h)) => {
                unsafe {
                    let _ = UnhookWindowsHookEx(h);
                    let _ = KillTimer(Some(self.icon.hwnd), WATCH_TIMER);
                }
                self.mouse_hook = None;
                bump_seq();
                self.hide();
                tracing::info!("selection hook removed");
            }
            _ => {}
        }
    }

    /// Record the watchdog baseline; returns the cursor position, or `None`
    /// when it cannot be read (locked workstation, secure desktop).
    fn sample_watch(&mut self) -> Option<POINT> {
        let mut pos = POINT::default();
        unsafe { GetCursorPos(&mut pos) }.ok()?;
        self.watch.pos = pos;
        self.watch.calls = MOUSE_CALLS.load(Relaxed);
        self.watch.input = last_input_tick().unwrap_or(0);
        Some(pos)
    }

    /// Watch timer: reinstall a hook Windows dropped.
    fn on_watch(&mut self) {
        if self.mouse_hook.is_none() {
            self.reinstall_mouse_hook();
            return;
        }
        let (prev_pos, prev_calls, prev_input) = (self.watch.pos, self.watch.calls, self.watch.input);
        let Some(pos) = self.sample_watch() else { return };
        let fg = unsafe { GetForegroundWindow() };
        let fg_pid = if fg.is_invalid() { 0 } else { blacklist::window_pid(fg) };
        let under = unsafe { GetAncestor(WindowFromPoint(pos), GA_ROOT) };
        let under_pid = if under.is_invalid() { 0 } else { blacklist::window_pid(under) };
        let elevated = |pid: u32| pid != 0 && blacklist::runs_elevated_above_us(pid);

        let moved = pos != prev_pos;
        let new_input = self.watch.input != prev_input;
        if moved && new_input && self.watch.calls == prev_calls && !elevated(fg_pid) && !elevated(under_pid) {
            self.reinstall_mouse_hook();
        }
    }

    fn reinstall_mouse_hook(&mut self) {
        if self.watch.last_reinstall.is_some_and(|t| t.elapsed() < REINSTALL_GAP) {
            return;
        }
        self.watch.last_reinstall = Some(Instant::now());
        self.watch.reinstalls += 1;
        unsafe {
            if let Some(h) = self.mouse_hook.take() {
                let _ = UnhookWindowsHookEx(h);
            }
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(h) => self.mouse_hook = Some(h),
                Err(e) => tracing::error!("reinstalling the mouse hook failed: {e}"),
            }
        }
        self.down = None;
        let n = self.watch.reinstalls;
        if n <= 20 || n % 100 == 0 {
            tracing::warn!("the mouse hook was dropped by Windows; reinstalled (#{n})");
        }
    }

    fn on_down(&mut self, pt: POINT) {
        if self.icon.visible && contains(&self.icon.rect, pt) {
            self.down = None; // a click on the icon; its window handles it
            return;
        }
        bump_seq();
        self.hide();
        let root = unsafe { GetAncestor(WindowFromPoint(pt), GA_ROOT) };
        self.down = (!root.is_invalid()).then(|| Down {
            pt,
            ibeam: ibeam_cursor(),
            hard_non_text: hard_non_text_cursor(),
            root,
            rect: window_rect(root),
        });
    }

    fn on_up(&mut self, pt: POINT) {
        if self.icon.visible && contains(&self.icon.rect, pt) {
            return;
        }
        let Some(down) = self.down.take() else {
            tracing::debug!("button up without a tracked press");
            return;
        };
        let cfg = self.cfg.read().unwrap().clone();
        if !cfg.enabled {
            return;
        }
        let s = scale_at(pt);
        let moved = (pt.x - down.pt.x).abs().max((pt.y - down.pt.y).abs());
        if (moved as f32) < DRAG_DIP * s {
            return;
        }
        if window_rect(down.root) != down.rect {
            tracing::debug!("selection icon skipped: the drag moved or resized a window");
            return;
        }
        let up_ibeam = ibeam_cursor();
        let ibeam = down.ibeam || up_ibeam;
        // Resizing a splitter/sash, crosshair canvas, or hidden game pointer
        // is never a text drag (and must not re-trigger a stale UIA selection).
        if let Some(c) = down.hard_non_text
            && !ibeam
        {
            tracing::debug!("selection icon skipped: non-text cursor {c}");
            return;
        }
        // Topmost borderless full-screen windows are screenshot overlays
        // (Feishu, WeChat, QQ, Snipaste, Win+Shift+S...).
        if borderless_fullscreen(down.root) && is_topmost(down.root) {
            tracing::debug!("selection icon skipped: full-screen overlay {}", blacklist::class_name(down.root));
            return;
        }
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_invalid() {
            tracing::debug!("selection icon skipped: no foreground window");
            return;
        }
        let app = match blacklist::check(fg, &cfg.filter) {
            Ok(exe) => exe,
            Err(skip) => {
                tracing::debug!("selection icon skipped: {skip:?}");
                return;
            }
        };
        let root_class = blacklist::class_name(down.root);
        let is_web_engine = matches!(root_class.as_str(), "Chrome_WidgetWin_1" | "MozillaWindowClass");
        let seq = bump_seq();
        let anchor = ScreenRect {
            left: pt.x.min(down.pt.x),
            top: pt.y.min(down.pt.y),
            right: pt.x.max(down.pt.x),
            bottom: pt.y.max(down.pt.y),
        };
        tracing::debug!(
            "drag candidate seq={seq} moved={moved}px ibeam={} ({}->{}) root={} app={}",
            ibeam,
            down.ibeam,
            up_ibeam,
            root_class,
            app.as_deref().unwrap_or("?"),
        );
        let _ = self.jobs.send(WorkerJob::Probe(ProbeReq {
            seq,
            fg_hwnd: fg.0 as isize,
            down_pt: down.pt,
            up_pt: pt,
            anchor,
            margin_px: (32.0 * s).round() as i32,
            app,
            is_web_engine,
            ibeam,
        }));
    }

    fn on_probe_done(&mut self) {
        let mut latest = None;
        while let Ok(res) = self.probe_rx.try_recv() {
            latest = Some(res);
        }
        let Some(res) = latest else { return };
        if res.seq != gesture_seq() || MENU_OPEN.load(Relaxed) || !self.cfg.read().unwrap().enabled {
            return;
        }
        // If the pointer already moved far away during the async UIA probe,
        // skip showing the icon.
        let mut cur = POINT::default();
        if unsafe { GetCursorPos(&mut cur) }.is_ok() {
            let far = (FAR_DIP * scale_at(res.up_pt)) as i32;
            if (cur.x - res.up_pt.x).abs().max((cur.y - res.up_pt.y).abs()) > far {
                tracing::debug!("selection icon skipped: pointer moved away during probe");
                return;
            }
        }
        self.show(res);
    }

    fn show(&mut self, res: ProbeResult) {
        let r = self.icon.show_near(res.up_pt);
        tracing::debug!(
            "selection icon shown at ({},{}) app={} cached={}",
            r.left,
            r.top,
            res.app.as_deref().unwrap_or("?"),
            res.cached_text.is_some(),
        );
        for (a, v) in ICON_RECT.iter().zip([r.left, r.top, r.right, r.bottom]) {
            a.store(v, Relaxed);
        }
        FAR_PX.store((FAR_DIP * scale_at(res.up_pt)) as i32, Relaxed);
        HOVER.store(false, Relaxed);
        ICON_VISIBLE.store(true, Relaxed);
        unsafe {
            SetTimer(Some(self.icon.hwnd), HIDE_TIMER, HIDE_AFTER_MS, None);
        }
        if self.kb_hook.is_none() {
            self.kb_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) }.ok();
        }
        self.pending = Some(Pending { anchor: res.anchor, app: res.app, cached_text: res.cached_text });
    }

    fn hide(&mut self) {
        ICON_VISIBLE.store(false, Relaxed);
        self.pending = None;
        if !self.icon.visible {
            return;
        }
        self.icon.hide();
        unsafe {
            let _ = KillTimer(Some(self.icon.hwnd), HIDE_TIMER);
            if let Some(h) = self.kb_hook.take() {
                let _ = UnhookWindowsHookEx(h);
            }
        }
    }

    fn on_timer(&mut self) {
        // Keep it while the pointer rests on it or its context menu is open.
        if !self.icon.visible || HOVER.load(Relaxed) || MENU_OPEN.load(Relaxed) {
            return;
        }
        self.hide();
    }

    fn on_click(&mut self) {
        if let Some(pending) = self.pending.take() {
            bump_seq();
            self.hide();
            let _ = self.jobs.send(WorkerJob::Capture(CaptureReq {
                anchor: pending.anchor,
                app: pending.app,
                cached_text: pending.cached_text,
            }));
        }
    }
}

/// Show the icon's native right-click menu outside of `TRACKER`'s `RefCell`
/// borrow, since `TrackPopupMenuEx` runs a nested modal message loop.
fn open_icon_context_menu() {
    let mut ctx: Option<(HWND, Option<String>, FilterMode)> = None;
    with_tracker(|t| {
        if !t.icon.visible || t.pending.is_none() {
            return;
        }
        let app = t.pending.as_ref().and_then(|p| p.app.clone());
        let mode = t.cfg.read().unwrap().filter.mode;
        MENU_OPEN.store(true, Relaxed);
        unsafe {
            let _ = KillTimer(Some(t.icon.hwnd), HIDE_TIMER);
        }
        ctx = Some((t.icon.hwnd, app, mode));
    });
    let Some((hwnd, app, mode)) = ctx else { return };

    let cmd = show_native_menu(hwnd, app.as_deref(), mode);

    MENU_OPEN.store(false, Relaxed);
    bump_seq();
    with_tracker(|t| {
        t.hide();
        match cmd {
            MENU_CMD_EXCLUDE => {
                if let Some(exe) = app {
                    let _ = t.jobs.send(WorkerJob::Action(IconAction::ExcludeApp(exe)));
                }
            }
            MENU_CMD_DISABLE => {
                let _ = t.jobs.send(WorkerJob::Action(IconAction::DisableSelection));
            }
            _ => {}
        }
    });
}

fn show_native_menu(hwnd: HWND, app: Option<&str>, mode: FilterMode) -> i32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        if let Some(exe) = app {
            let label = match mode {
                FilterMode::Blacklist => format!("在 {exe} 中禁用划词"),
                FilterMode::Whitelist => format!("从白名单移除 {exe}"),
            };
            let wide: Vec<u16> = label.encode_utf16().chain([0]).collect();
            let _ = AppendMenuW(menu, MF_STRING, MENU_CMD_EXCLUDE as usize, PCWSTR(wide.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let disable: Vec<u16> = "关闭划词翻译".encode_utf16().chain([0]).collect();
        let _ = AppendMenuW(menu, MF_STRING, MENU_CMD_DISABLE as usize, PCWSTR(disable.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let prev_fg = GetForegroundWindow();
        // Temporarily clear WS_EX_NOACTIVATE so SetForegroundWindow succeeds
        // and TrackPopupMenuEx dismisses cleanly when clicking outside.
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as i32));
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0,
            pt.x,
            pt.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex);
        // If dismissed via Esc or selecting a menu item, return focus to the
        // previous foreground window rather than leaving it on the hidden icon.
        if GetForegroundWindow() == hwnd && !prev_fg.is_invalid() && prev_fg != hwnd {
            let _ = SetForegroundWindow(prev_fg);
        }
        cmd.0
    }
}

/// The window covers its whole monitor without being maximized: a game, video
/// or screenshot overlay in borderless full-screen.
fn borderless_fullscreen(h: HWND) -> bool {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
    unsafe {
        if IsZoomed(h).as_bool() {
            return false;
        }
        let m = MonitorFromWindow(h, MONITOR_DEFAULTTONULL);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if m.is_invalid() || !GetMonitorInfoW(m, &mut mi).as_bool() {
            return false;
        }
        let (w, r) = (window_rect(h), mi.rcMonitor);
        w.left <= r.left && w.top <= r.top && w.right >= r.right && w.bottom >= r.bottom
    }
}

/// `GetTickCount` of the last input event the system received.
fn last_input_tick() -> Option<u32> {
    let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe { GetLastInputInfo(&mut lii) }.as_bool().then_some(lii.dwTime)
}

fn is_topmost(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

fn ibeam_cursor() -> bool {
    unsafe {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        GetCursorInfo(&mut ci).is_ok()
            && ci.flags.0 & (CURSOR_SHOWING.0 | CURSOR_SUPPRESSED.0) != 0
            && LoadCursorW(None, IDC_IBEAM).is_ok_and(|c| c == ci.hCursor)
    }
}

/// Cursor shapes that unambiguously rule out starting a text drag (resize
/// sashes, crosshair, no-drop, busy, hidden game pointer).
fn hard_non_text_cursor() -> Option<&'static str> {
    unsafe {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_err() {
            return None;
        }
        if ci.flags.0 & (CURSOR_SHOWING.0 | CURSOR_SUPPRESSED.0) == 0 || ci.hCursor.is_invalid() {
            return Some("hidden");
        }
        let hard = [
            (IDC_CROSS, "cross"),
            (IDC_SIZEALL, "size-all"),
            (IDC_SIZENWSE, "size-nwse"),
            (IDC_SIZENESW, "size-nesw"),
            (IDC_SIZEWE, "size-we"),
            (IDC_SIZENS, "size-ns"),
            (IDC_NO, "no"),
            (IDC_WAIT, "wait"),
        ];
        hard.into_iter().find(|(id, _)| LoadCursorW(None, *id).is_ok_and(|c| c == ci.hCursor)).map(|(_, n)| n)
    }
}

/// Run `f` on the tracker unless it is already borrowed (re-entrant messages
/// sent by our own SetWindowPos/ShowWindow calls never need it).
fn with_tracker(f: impl FnOnce(&mut Tracker)) {
    TRACKER.with(|t| {
        if let Ok(mut t) = t.try_borrow_mut() {
            if let Some(t) = t.as_mut() {
                f(t);
            }
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => with_tracker(|t| t.on_click()),
        WM_RBUTTONUP => open_icon_context_menu(),
        WM_TIMER if wp.0 == HIDE_TIMER => with_tracker(|t| t.on_timer()),
        WM_TIMER if wp.0 == WATCH_TIMER => with_tracker(|t| t.on_watch()),
        MSG_DOWN => with_tracker(|t| t.on_down(unpack(lp))),
        MSG_UP => with_tracker(|t| t.on_up(unpack(lp))),
        MSG_HOVER => with_tracker(|t| t.icon.set_hover(wp.0 != 0)),
        MSG_HIDE => with_tracker(|t| t.hide()),
        MSG_CONFIG => with_tracker(|t| t.apply_config()),
        MSG_PROBE_DONE => with_tracker(|t| t.on_probe_done()),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

/// Start the hook thread; returns the icon window handle (also the clipboard
/// owner for Ctrl+Insert, since this thread always pumps messages).
pub fn spawn(
    cfg: Arc<RwLock<Config>>,
    jobs: Sender<WorkerJob>,
    probe_rx: Receiver<ProbeResult>,
) -> anyhow::Result<isize> {
    let (tx, rx) = sync_channel(1);
    std::thread::Builder::new().name("selection-hook".into()).spawn(move || unsafe {
        let created = (|| -> windows::core::Result<HWND> {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_HAND)?,
                lpszClassName: w!("LLMTranslateSelectionIcon"),
                ..Default::default()
            };
            RegisterClassExW(&class);
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("LLMTranslateSelectionIcon"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )
        })();
        let hwnd = match created {
            Ok(h) => h,
            Err(e) => {
                let _ = tx.send(Err(anyhow::anyhow!("creating the selection icon window failed: {e}")));
                return;
            }
        };
        TARGET.store(hwnd.0 as isize, Relaxed);
        TRACKER.with(|t| {
            let mut tracker = Tracker {
                icon: Icon::new(hwnd),
                cfg,
                jobs,
                probe_rx,
                mouse_hook: None,
                kb_hook: None,
                down: None,
                pending: None,
                watch: Watch::default(),
            };
            tracker.apply_config();
            *t.borrow_mut() = Some(tracker);
        });
        let _ = tx.send(Ok(hwnd.0 as isize));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    })?;
    rx.recv()?
}

/// Ask the hook thread to re-read the config (install/remove the hook).
pub fn notify_config() {
    post(MSG_CONFIG, 0, 0);
}
