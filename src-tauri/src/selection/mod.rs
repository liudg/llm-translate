//! Selection translation (DESIGN 5.5): the hook thread shows an icon after a
//! selection gesture; clicking it captures the text here via UIA, then
//! Ctrl+Insert. Ctrl+C is never sent.

pub mod blacklist;
pub mod clipboard;
pub mod hook;
pub mod icon;
pub mod input;
pub mod uia;

use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::Serialize;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::app::settings::SelectionSettings;

/// Screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// One icon click.
#[derive(Debug, Clone, Copy)]
pub struct Job {
    /// Bounding box of the gesture's press and release points.
    pub anchor: ScreenRect,
    /// Clipboard owner window (the icon window).
    pub owner: isize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    Uia,
    CtrlInsert,
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub anchor: ScreenRect,
    /// `None`: neither UIA nor Ctrl+Insert produced text.
    pub text: Option<String>,
    pub method: Option<Method>,
}

pub struct Selection {
    cfg: Arc<RwLock<hook::Config>>,
}

fn to_config(s: &SelectionSettings) -> hook::Config {
    hook::Config {
        enabled: s.enabled,
        drag: s.drag,
        double_click: s.double_click,
        shift_click: s.shift_click,
        blacklist: s.blacklist.iter().map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect(),
    }
}

/// Capture the selected text of the focused control: UIA first, then
/// Ctrl+Insert. `owner` is the clipboard owner window.
pub fn capture_text(uia: Option<&uia::Uia>, owner: Option<HWND>) -> (Option<String>, Option<Method>) {
    let t0 = Instant::now();
    if let Some(u) = uia {
        match u.selection() {
            Ok(s) if !s.trim().is_empty() => {
                tracing::info!("selection via UIA: {} chars in {} ms", s.chars().count(), t0.elapsed().as_millis());
                return (Some(s), Some(Method::Uia));
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
            (Some(s), Some(Method::CtrlInsert))
        }
        Err(e) => {
            tracing::info!("selection capture failed: {e:?} after {} ms", t0.elapsed().as_millis());
            (None, None)
        }
    }
}

impl Selection {
    /// Start the hook and worker threads. `on_capture` runs on the worker
    /// thread after each icon click.
    pub fn start(settings: &SelectionSettings, on_capture: impl Fn(Capture) + Send + 'static) -> anyhow::Result<Self> {
        let cfg = Arc::new(RwLock::new(to_config(settings)));
        #[cfg(debug_assertions)]
        if std::env::var_os("LLMT_ACCEPT_INJECTED").is_some() {
            tracing::warn!("selection hook accepts injected input (test mode)");
            hook::ACCEPT_INJECTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new().name("selection-capture".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
            }
            let uia = uia::Uia::new().map_err(|e| tracing::error!("UI Automation unavailable: {e}")).ok();
            for job in rx {
                let (text, method) = capture_text(uia.as_ref(), Some(HWND(job.owner as *mut _)));
                on_capture(Capture { anchor: job.anchor, text, method });
            }
        })?;
        hook::spawn(cfg.clone(), tx)?;
        Ok(Self { cfg })
    }

    pub fn set_config(&self, s: &SelectionSettings) {
        *self.cfg.write().unwrap() = to_config(s);
        hook::notify_config();
    }
}
