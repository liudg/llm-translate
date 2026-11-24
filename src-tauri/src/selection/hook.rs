//! Hook thread (DESIGN 5.5): a low-level mouse hook plus the icon window, both
//! on one thread with a message loop. Hook callbacks only do arithmetic on
//! atomics and post messages to the icon window; gesture decisions run in the
//! window procedure, so the callback never blocks (Windows silently removes
//! hooks that time out). Text capture runs on the worker thread in `mod.rs`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::Relaxed};
use std::sync::mpsc::{Sender, sync_channel};
use std::sync::{Arc, RwLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::blacklist::{self, Blacklist};
use super::icon::{Icon, scale_at};
use super::ScreenRect;

const MSG_DOWN: u32 = WM_APP + 1;
const MSG_UP: u32 = WM_APP + 2;
const MSG_HOVER: u32 = WM_APP + 3;
const MSG_HIDE: u32 = WM_APP + 4;
const MSG_CONFIG: u32 = WM_APP + 5;

const HIDE_TIMER: usize = 1;
const HIDE_AFTER_MS: u32 = 3000;
/// Minimum drag distance in DIPs.
const DRAG_DIP: f32 = 8.0;
/// Moving this far (DIPs) from the icon hides it.
const FAR_DIP: f32 = 120.0;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub enabled: bool,
    pub drag: bool,
    pub double_click: bool,
    pub shift_click: bool,
    pub blacklist: Blacklist,
}

// ---- state shared with the hook callbacks (same thread, but callbacks must
// not touch the RefCell'd tracker) ----
static TARGET: AtomicIsize = AtomicIsize::new(0);
static ICON_VISIBLE: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicBool = AtomicBool::new(false);
static ICON_RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static FAR_PX: AtomicI32 = AtomicI32::new(120);
/// Accept injected mouse input as gestures (automated tests only).
pub static ACCEPT_INJECTED: AtomicBool = AtomicBool::new(false);

fn post(msg: u32, wp: usize, lp: isize) {
    let h = TARGET.load(Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(wp), LPARAM(lp));
        }
    }
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

/// Hide the icon from a callback: flip the flag first so only one message is
/// posted.
fn request_hide() {
    if ICON_VISIBLE.swap(false, Relaxed) {
        post(MSG_HIDE, 0, 0);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
            let injected = info.flags & LLMHF_INJECTED != 0;
            if !injected || ACCEPT_INJECTED.load(Relaxed) {
                match wp.0 as u32 {
                    m @ (WM_LBUTTONDOWN | WM_LBUTTONUP) => {
                        let shift = (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
                        let w = info.time as usize | (shift as usize) << 32;
                        post(if m == WM_LBUTTONDOWN { MSG_DOWN } else { MSG_UP }, w, pack(info.pt));
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
                    WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                        request_hide();
                    }
                    _ => {}
                }
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

/// Installed only while the icon is visible: any real key press hides it.
/// Modifiers alone do not (holding Shift auto-repeats key-downs right after a
/// Shift+click).
unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
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
    /// Top-level window under the press and its rectangle, to tell a window
    /// move/resize from a text drag.
    root: HWND,
    rect: RECT,
}

struct Tracker {
    icon: Icon,
    cfg: Arc<RwLock<Config>>,
    /// Anchors of icon clicks, for the capture thread.
    jobs: Sender<ScreenRect>,
    mouse_hook: Option<HHOOK>,
    kb_hook: Option<HHOOK>,
    down: Option<Down>,
    last_up: Option<(POINT, u32)>,
    pending: Option<ScreenRect>,
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
                }
                Err(e) => tracing::error!("installing the mouse hook failed: {e}"),
            },
            (false, Some(h)) => {
                unsafe {
                    let _ = UnhookWindowsHookEx(h);
                }
                self.mouse_hook = None;
                self.hide();
                tracing::info!("selection hook removed");
            }
            _ => {}
        }
    }

    fn on_down(&mut self, pt: POINT) {
        if self.icon.visible && contains(&self.icon.rect, pt) {
            self.down = None; // a click on the icon; its window handles it
            return;
        }
        self.hide();
        let root = unsafe { GetAncestor(WindowFromPoint(pt), GA_ROOT) };
        self.down = (!root.is_invalid()).then(|| Down { pt, root, rect: window_rect(root) });
    }

    fn on_up(&mut self, pt: POINT, time: u32, shift: bool) {
        if self.icon.visible && contains(&self.icon.rect, pt) {
            return;
        }
        let Some(down) = self.down.take() else { return };
        let cfg = self.cfg.read().unwrap().clone();
        let s = scale_at(pt);
        let moved = (pt.x - down.pt.x).abs().max((pt.y - down.pt.y).abs());
        let is_drag = moved as f32 >= DRAG_DIP * s;
        let is_double = !is_drag
            && self.last_up.is_some_and(|(p, t)| unsafe {
                time.wrapping_sub(t) <= GetDoubleClickTime()
                    && (pt.x - p.x).abs() <= GetSystemMetrics(SM_CXDOUBLECLK) / 2
                    && (pt.y - p.y).abs() <= GetSystemMetrics(SM_CYDOUBLECLK) / 2
            });
        self.last_up = (!is_drag).then_some((pt, time));
        let wanted = if is_drag {
            cfg.drag
        } else if is_double {
            cfg.double_click
        } else {
            shift && cfg.shift_click
        };
        if !wanted {
            return;
        }
        if is_drag && window_rect(down.root) != down.rect {
            return; // the drag moved or resized a window
        }
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_invalid() {
            return;
        }
        if let Err(skip) = blacklist::check(fg, &cfg.blacklist) {
            tracing::debug!("selection icon skipped: {skip:?}");
            return;
        }
        let anchor = ScreenRect {
            left: pt.x.min(down.pt.x),
            top: pt.y.min(down.pt.y),
            right: pt.x.max(down.pt.x),
            bottom: pt.y.max(down.pt.y),
        };
        self.show(anchor, pt);
    }

    fn show(&mut self, anchor: ScreenRect, pt: POINT) {
        let r = self.icon.show_near(pt);
        for (a, v) in ICON_RECT.iter().zip([r.left, r.top, r.right, r.bottom]) {
            a.store(v, Relaxed);
        }
        FAR_PX.store((FAR_DIP * scale_at(pt)) as i32, Relaxed);
        HOVER.store(false, Relaxed);
        ICON_VISIBLE.store(true, Relaxed);
        unsafe {
            SetTimer(Some(self.icon.hwnd), HIDE_TIMER, HIDE_AFTER_MS, None);
        }
        if self.kb_hook.is_none() {
            self.kb_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) }.ok();
        }
        self.pending = Some(anchor);
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
        // Keep it while the pointer rests on it.
        if !self.icon.visible || HOVER.load(Relaxed) {
            return;
        }
        self.hide();
    }

    fn on_click(&mut self) {
        if let Some(anchor) = self.pending.take() {
            self.hide();
            let _ = self.jobs.send(anchor);
        }
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
        WM_TIMER if wp.0 == HIDE_TIMER => with_tracker(|t| t.on_timer()),
        MSG_DOWN => with_tracker(|t| t.on_down(unpack(lp))),
        MSG_UP => with_tracker(|t| t.on_up(unpack(lp), wp.0 as u32, (wp.0 >> 32) & 1 == 1)),
        MSG_HOVER => with_tracker(|t| t.icon.set_hover(wp.0 != 0)),
        MSG_HIDE => with_tracker(|t| t.hide()),
        MSG_CONFIG => with_tracker(|t| t.apply_config()),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

/// Start the hook thread; returns the icon window handle (also the clipboard
/// owner for Ctrl+Insert, since this thread always pumps messages).
pub fn spawn(cfg: Arc<RwLock<Config>>, jobs: Sender<ScreenRect>) -> anyhow::Result<isize> {
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
                mouse_hook: None,
                kb_hook: None,
                down: None,
                last_up: None,
                pending: None,
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
