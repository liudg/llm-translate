//! Selection translation (DESIGN 5.5): after a drag gesture, the worker
//! performs a read-only UIA probe (falling back to strict `IDC_IBEAM` cursor
//! gating when UIA has no TextPattern) before showing the floating icon.
//! Clicking the icon reuses any UIA text cached during the probe or captures
//! via UIA → Ctrl+Insert. Ctrl+C is never sent.

pub mod blacklist;
pub mod clipboard;
pub mod hook;
pub mod icon;
pub mod input;
pub mod uia;

use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::app::settings::{SelectionSettings, normalize_exe};

/// Brief settle time after `WM_LBUTTONUP` so the target app finishes updating
/// its selection before UIA reads it.
const PROBE_SETTLE: Duration = Duration::from_millis(25);

/// Screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone)]
pub struct Capture {
    /// Bounding box of the gesture's press and release points.
    pub anchor: ScreenRect,
    /// Why neither UIA nor Ctrl+Insert produced text.
    pub text: Result<String, clipboard::CopyError>,
    /// Executable of the foreground window, e.g. `Code.exe`.
    pub app: Option<String>,
}

/// Events produced by the selection subsystem for the Tauri app shell.
#[derive(Debug, Clone)]
pub enum SelectionEvent {
    Captured(Capture),
    ExcludeApp(String),
    DisableSelection,
}

pub struct Selection {
    cfg: Arc<RwLock<hook::Config>>,
}

fn normalize_set(items: &[String]) -> blacklist::ProcessSet {
    items.iter().map(|n| normalize_exe(n)).filter(|n| !n.is_empty()).collect()
}

fn to_config(s: &SelectionSettings) -> hook::Config {
    hook::Config {
        enabled: s.enabled,
        filter: blacklist::Filter {
            mode: s.mode,
            blacklist: normalize_set(&s.blacklist),
            whitelist: normalize_set(&s.whitelist),
        },
    }
}

/// Capture the selected text of the focused control: UIA first, then
/// Ctrl+Insert. `owner` is the clipboard owner window.
pub fn capture_text(uia: Option<&uia::Uia>, owner: Option<HWND>) -> Result<String, clipboard::CopyError> {
    let t0 = Instant::now();
    if let Some(u) = uia {
        match u.selection() {
            Ok(s) if !s.trim().is_empty() => {
                tracing::info!("selection via UIA: {} chars in {} ms", s.chars().count(), t0.elapsed().as_millis());
                return Ok(s);
            }
            Ok(_) => tracing::debug!("UIA: no selection"),
            Err(e) => tracing::debug!("UIA failed: {e}"),
        }
    }
    let t1 = Instant::now();
    match clipboard::copy_selection(owner) {
        Ok(s) => {
            let s = uia::normalize(&s);
            tracing::info!(
                "selection via Ctrl+Insert: {} chars in {} ms (UIA {} ms)",
                s.chars().count(),
                t1.elapsed().as_millis(),
                (t1 - t0).as_millis()
            );
            Ok(s)
        }
        Err(e) => {
            tracing::info!("selection capture failed: {e:?} after {} ms", t0.elapsed().as_millis());
            Err(e)
        }
    }
}

fn handle_probe(uia: Option<&uia::Uia>, req: hook::ProbeReq, probe_tx: &std::sync::mpsc::Sender<hook::ProbeResult>) {
    let stale = || req.seq != hook::gesture_seq();
    if stale() {
        return;
    }
    std::thread::sleep(PROBE_SETTLE);
    if stale() {
        return;
    }
    let t0 = Instant::now();
    let fg = HWND(req.fg_hwnd as *mut _);
    let decision = match uia {
        Some(u) => u.probe(fg, req.down_pt, req.up_pt, req.anchor, req.margin_px, req.is_web_engine, stale),
        None => uia::Probe::NeedIbeam("uia-unavailable"),
    };
    if stale() {
        return;
    }
    let cached_text = match decision {
        uia::Probe::Selected(text) => {
            tracing::debug!(
                "probe approved via UIA: {} chars in {} ms (app={})",
                text.chars().count(),
                t0.elapsed().as_millis(),
                req.app.as_deref().unwrap_or("?"),
            );
            Some(text)
        }
        uia::Probe::NotText(reason) => {
            tracing::debug!(
                "probe rejected ({reason}) in {} ms (app={}, ibeam={})",
                t0.elapsed().as_millis(),
                req.app.as_deref().unwrap_or("?"),
                req.ibeam,
            );
            return;
        }
        uia::Probe::NeedIbeam(reason) => {
            if !req.ibeam {
                tracing::debug!(
                    "probe rejected ({reason}, non-ibeam) in {} ms (app={})",
                    t0.elapsed().as_millis(),
                    req.app.as_deref().unwrap_or("?"),
                );
                return;
            }
            tracing::debug!(
                "probe approved via I-beam fallback ({reason}) in {} ms (app={})",
                t0.elapsed().as_millis(),
                req.app.as_deref().unwrap_or("?"),
            );
            None
        }
    };
    let _ = probe_tx.send(hook::ProbeResult {
        seq: req.seq,
        up_pt: req.up_pt,
        anchor: req.anchor,
        app: req.app,
        cached_text,
    });
    hook::notify_probe_done();
}

impl Selection {
    /// Start the hook and worker threads. `on_event` runs on the worker thread.
    pub fn start(
        settings: &SelectionSettings,
        on_event: impl Fn(SelectionEvent) + Send + 'static,
    ) -> anyhow::Result<Self> {
        let cfg = Arc::new(RwLock::new(to_config(settings)));
        #[cfg(debug_assertions)]
        if std::env::var_os("LLMT_ACCEPT_INJECTED").is_some() {
            tracing::warn!("selection hook accepts injected input (test mode)");
            hook::ACCEPT_INJECTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let (job_tx, job_rx) = channel::<hook::WorkerJob>();
        let (probe_tx, probe_rx) = channel::<hook::ProbeResult>();
        // The icon window's thread pumps messages, so it can own the clipboard.
        let owner = hook::spawn(cfg.clone(), job_tx, probe_rx)?;
        std::thread::Builder::new().name("selection-capture".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
            }
            let uia = uia::Uia::new().map_err(|e| tracing::error!("UI Automation unavailable: {e}")).ok();
            for job in job_rx {
                match job {
                    hook::WorkerJob::Probe(req) => handle_probe(uia.as_ref(), req, &probe_tx),
                    hook::WorkerJob::Capture(req) => {
                        let fg = unsafe { GetForegroundWindow() };
                        let app = (!fg.is_invalid())
                            .then(|| blacklist::process_name(blacklist::window_pid(fg)))
                            .flatten()
                            .or(req.app);
                        let text = match req.cached_text {
                            Some(cached) => {
                                tracing::info!("selection via cached UIA: {} chars", cached.chars().count());
                                Ok(cached)
                            }
                            None => {
                                let res = capture_text(uia.as_ref(), Some(HWND(owner as *mut _)));
                                if res.is_err() {
                                    tracing::info!("selection capture failed in {}", app.as_deref().unwrap_or("?"));
                                }
                                res
                            }
                        };
                        on_event(SelectionEvent::Captured(Capture { anchor: req.anchor, text, app }));
                    }
                    hook::WorkerJob::Action(action) => match action {
                        hook::IconAction::ExcludeApp(exe) => on_event(SelectionEvent::ExcludeApp(exe)),
                        hook::IconAction::DisableSelection => on_event(SelectionEvent::DisableSelection),
                    },
                }
            }
        })?;
        Ok(Self { cfg })
    }

    pub fn set_config(&self, s: &SelectionSettings) {
        *self.cfg.write().unwrap() = to_config(s);
        hook::notify_config();
    }
}
