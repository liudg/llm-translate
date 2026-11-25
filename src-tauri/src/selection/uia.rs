//! Read-only UI Automation probing and text extraction (DESIGN 5.5).
//! Must be used on a thread initialised for the multithreaded apartment.

use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{E_FAIL, HWND, POINT, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, SAFEARRAY};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
use windows::core::{Error, Interface, Result};

use super::ScreenRect;

/// Every UIA call gives up after this long, then Ctrl+Insert takes over.
const TIMEOUT_MS: u32 = 200;
/// How far up from the focused element to look for a TextPattern.
const MAX_DEPTH: usize = 6;
/// How far up from an element-from-point leaf to look for a TextPattern.
const MAX_POINT_DEPTH: usize = 8;
/// How far up from a TextPattern element to locate its owning window handle.
const MAX_WINDOW_DEPTH: usize = 32;
/// Short retry window when a web-engine window's accessibility tree is waking up.
const WEB_ENGINE_RETRY_MS: u64 = 40;

/// Result of a read-only UIA probe right after a mouse drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// UIA TextPattern returned non-empty selected text belonging to the drag
    /// window and overlapping the drag region (cached for instant translation).
    Selected(String),
    /// UIA confirms the drag did not select text (chrome/non-text control,
    /// stale selection elsewhere in the window, or an authoritative native
    /// TextPattern with an empty selection).
    NotText(&'static str),
    /// UIA cannot prove or disprove text selection (no TextPattern, transient
    /// UIA read error, or a web-engine document whose selection may live in a
    /// `<canvas>` like xterm.js). Show the icon only with `IDC_IBEAM` evidence.
    NeedIbeam(&'static str),
}

enum TextRead {
    Selected { text: String, rects: Vec<RECT>, elem_rect: Option<RECT> },
    Empty,
    NoPattern,
    ReadError,
}

enum QueryOutcome {
    Selected(String),
    None { confirmed_empty: bool, stale: bool, read_error: bool },
}

/// Controls where a mouse drag is never a text selection (scrollbars, tabs,
/// sliders, window chrome, menus). Checked at the drag start point so dragging
/// chrome cannot re-trigger a stale selection still left in the document.
const NON_TEXT_CONTROLS: [UIA_CONTROLTYPE_ID; 16] = [
    UIA_ScrollBarControlTypeId,
    UIA_ThumbControlTypeId,
    UIA_SliderControlTypeId,
    UIA_TitleBarControlTypeId,
    UIA_MenuBarControlTypeId,
    UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId,
    UIA_TabControlTypeId,
    UIA_TabItemControlTypeId,
    UIA_HeaderControlTypeId,
    UIA_HeaderItemControlTypeId,
    UIA_SplitButtonControlTypeId,
    UIA_ProgressBarControlTypeId,
    UIA_SpinnerControlTypeId,
    UIA_StatusBarControlTypeId,
    UIA_ToolBarControlTypeId,
];

/// Parent controls whose `Text` / `Image` child labels are still non-text
/// chrome (e.g., the title text inside a browser tab or window title bar).
const NON_TEXT_CHROME_PARENTS: [UIA_CONTROLTYPE_ID; 7] = [
    UIA_TabItemControlTypeId,
    UIA_TitleBarControlTypeId,
    UIA_MenuItemControlTypeId,
    UIA_HeaderItemControlTypeId,
    UIA_ScrollBarControlTypeId,
    UIA_ThumbControlTypeId,
    UIA_SliderControlTypeId,
];

fn root_hwnd(h: HWND) -> HWND {
    let r = unsafe { GetAncestor(h, GA_ROOT) };
    if r.is_invalid() { h } else { r }
}

fn point_in_window(pt: POINT, fg: HWND) -> bool {
    if fg.is_invalid() {
        return false;
    }
    let h = unsafe { WindowFromPoint(pt) };
    !h.is_invalid() && root_hwnd(h) == fg
}

/// Convert UIA's SAFEARRAY of `f64` quads `[left, top, width, height, ...]`
/// into screen `RECT`s. Returns `None` if unavailable or empty.
unsafe fn range_rects(range: &IUIAutomationTextRange) -> Option<Vec<RECT>> {
    let sa = unsafe { range.GetBoundingRectangles().ok()? };
    if sa.is_null() {
        return None;
    }
    struct Guard(*mut SAFEARRAY);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = SafeArrayDestroy(self.0);
            }
        }
    }
    let _guard = Guard(sa);
    let (lbound, ubound) = unsafe { (SafeArrayGetLBound(sa, 1).ok()?, SafeArrayGetUBound(sa, 1).ok()?) };
    if ubound < lbound {
        return None;
    }
    let count = (ubound - lbound + 1) as usize;
    if count < 4 || count % 4 != 0 {
        return None;
    }
    let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    unsafe { SafeArrayAccessData(sa, &mut ptr).ok()? };
    let vals = unsafe { std::slice::from_raw_parts(ptr as *const f64, count) };
    let mut out = Vec::with_capacity(count / 4);
    for quad in vals.chunks_exact(4) {
        let (l, t, w, h) = (quad[0], quad[1], quad[2], quad[3]);
        if w > 0.0 && h > 0.0 && l.is_finite() && t.is_finite() && w.is_finite() && h.is_finite() {
            out.push(RECT {
                left: l.floor() as i32,
                top: t.floor() as i32,
                right: (l + w).ceil() as i32,
                bottom: (t + h).ceil() as i32,
            });
        }
    }
    unsafe {
        let _ = SafeArrayUnaccessData(sa);
    }
    (!out.is_empty()).then_some(out)
}

fn rects_overlap(a: ScreenRect, b: RECT, margin: i32) -> bool {
    a.left.saturating_sub(margin) <= b.right
        && a.right.saturating_add(margin) >= b.left
        && a.top.saturating_sub(margin) <= b.bottom
        && a.bottom.saturating_add(margin) >= b.top
}

/// Verify that a UIA selection actually overlaps the drag gesture (within
/// `margin` physical pixels), rejecting stale selections left elsewhere in the
/// document or window. Prefers exact text-range bounding rects when supplied;
/// falls back to the TextPattern element's bounding rect.
pub(crate) fn selection_matches_drag(
    anchor: ScreenRect,
    margin: i32,
    range_rects: &[RECT],
    elem_rect: Option<RECT>,
) -> bool {
    let usable_ranges: Vec<RECT> =
        range_rects.iter().copied().filter(|r| (r.right - r.left) >= 1 && (r.bottom - r.top) >= 4).collect();
    if !usable_ranges.is_empty() {
        return usable_ranges.iter().any(|&r| rects_overlap(anchor, r, margin));
    }
    if let Some(er) = elem_rect
        && (er.right - er.left) > 4
        && (er.bottom - er.top) > 4
    {
        return rects_overlap(anchor, er, margin);
    }
    // Hidden proxy inputs (e.g. 1x1 textarea) report degenerate rects.
    true
}

pub struct Uia {
    uia: IUIAutomation,
    walker: IUIAutomationTreeWalker,
}

impl Uia {
    pub fn new() -> Result<Self> {
        let uia: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))?
        };
        if let Ok(u2) = uia.cast::<IUIAutomation2>() {
            unsafe {
                let _ = u2.SetConnectionTimeout(TIMEOUT_MS);
                let _ = u2.SetTransactionTimeout(TIMEOUT_MS);
            }
        }
        let walker = unsafe { uia.ControlViewWalker()? };
        Ok(Self { uia, walker })
    }

    fn element_root_hwnd(&self, start: &IUIAutomationElement) -> Option<HWND> {
        unsafe {
            let mut cur = start.clone();
            for _ in 0..MAX_WINDOW_DEPTH {
                if let Ok(h) = cur.CurrentNativeWindowHandle()
                    && !h.is_invalid()
                {
                    return Some(root_hwnd(h));
                }
                match self.walker.GetParentElement(&cur) {
                    Ok(p) => cur = p,
                    Err(_) => break,
                }
            }
            None
        }
    }

    fn belongs_to_window(&self, el: &IUIAutomationElement, fg: HWND) -> bool {
        !fg.is_invalid() && self.element_root_hwnd(el).is_some_and(|root| root == fg)
    }

    /// Walk up from `start` for at most `max_depth` ancestors looking for a
    /// `TextPattern`. When `fg` is `Some`, rejects elements belonging to any
    /// other top-level window. Distinguishes COM/read failures from confirmed
    /// empty selections and never returns partial multi-range reads.
    fn read_from_element(&self, mut el: IUIAutomationElement, max_depth: usize, fg: Option<HWND>) -> TextRead {
        unsafe {
            for _ in 0..max_depth {
                if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                    if let Some(fg_hwnd) = fg
                        && !self.belongs_to_window(&el, fg_hwnd)
                    {
                        return TextRead::NoPattern;
                    }
                    let Ok(ranges) = tp.GetSelection() else { return TextRead::ReadError };
                    let Ok(len) = ranges.Length() else { return TextRead::ReadError };
                    let mut out = String::new();
                    let mut rects = Vec::new();
                    for i in 0..len {
                        let Ok(r) = ranges.GetElement(i) else { return TextRead::ReadError };
                        let Ok(text) = r.GetText(-1) else { return TextRead::ReadError };
                        out.push_str(&text.to_string());
                        if let Some(mut r_rects) = range_rects(&r) {
                            rects.append(&mut r_rects);
                        }
                    }
                    let text = normalize(&out);
                    if text.trim().is_empty() {
                        return TextRead::Empty;
                    }
                    let elem_rect = el.CurrentBoundingRectangle().ok();
                    return TextRead::Selected { text, rects, elem_rect };
                }
                match self.walker.GetParentElement(&el) {
                    Ok(p) => el = p,
                    Err(_) => break,
                }
            }
            TextRead::NoPattern
        }
    }

    /// Selected text of the focused control, normalised; empty when there is
    /// no TextPattern or no selection. Fails on partial/COM read errors so
    /// Ctrl+Insert can take over instead of returning truncated text.
    pub fn selection(&self) -> Result<String> {
        let el = unsafe { self.uia.GetFocusedElement()? };
        match self.read_from_element(el, MAX_DEPTH, None) {
            TextRead::Selected { text, .. } => Ok(text),
            TextRead::Empty | TextRead::NoPattern => Ok(String::new()),
            TextRead::ReadError => Err(Error::new(E_FAIL, "reading UIA selection failed")),
        }
    }

    /// Check whether `pt` landed on a non-text chrome control (scrollbar, tab,
    /// title bar, slider, menu, etc.).
    fn hit_non_text_chrome(&self, pt: POINT, fg: HWND) -> bool {
        if !point_in_window(pt, fg) {
            return false;
        }
        unsafe {
            let Ok(el) = self.uia.ElementFromPoint(pt) else { return false };
            let Ok(ct) = el.CurrentControlType() else { return false };
            if NON_TEXT_CONTROLS.contains(&ct) {
                return true;
            }
            if (ct == UIA_TextControlTypeId || ct == UIA_ImageControlTypeId)
                && let Ok(parent) = self.walker.GetParentElement(&el)
                && let Ok(pct) = parent.CurrentControlType()
                && NON_TEXT_CHROME_PARENTS.contains(&pct)
            {
                return true;
            }
            false
        }
    }

    /// Query `TextPattern` from the focused element, then from `up_pt` (only
    /// when `up_pt` is inside `fg`), keeping only selections that belong to
    /// `fg` and overlap `anchor`.
    fn query_text_selection(&self, fg: HWND, up_pt: POINT, anchor: ScreenRect, margin_px: i32) -> QueryOutcome {
        let mut confirmed_empty = false;
        let mut stale = false;
        let mut read_error = false;

        let mut inspect = |res: TextRead| -> Option<String> {
            match res {
                TextRead::Selected { text, rects, elem_rect } => {
                    if selection_matches_drag(anchor, margin_px, &rects, elem_rect) {
                        return Some(text);
                    }
                    stale = true;
                }
                TextRead::Empty => confirmed_empty = true,
                TextRead::ReadError => read_error = true,
                TextRead::NoPattern => {}
            }
            None
        };

        if let Ok(focused) = unsafe { self.uia.GetFocusedElement() }
            && let Some(text) = inspect(self.read_from_element(focused, MAX_DEPTH, Some(fg)))
        {
            return QueryOutcome::Selected(text);
        }
        if point_in_window(up_pt, fg)
            && let Ok(at_up) = unsafe { self.uia.ElementFromPoint(up_pt) }
            && let Some(text) = inspect(self.read_from_element(at_up, MAX_POINT_DEPTH, Some(fg)))
        {
            return QueryOutcome::Selected(text);
        }
        QueryOutcome::None { confirmed_empty, stale, read_error }
    }

    /// Read-only probe run before showing the selection icon. Never touches the
    /// clipboard or injects keys.
    pub fn probe(
        &self,
        fg: HWND,
        down_pt: POINT,
        up_pt: POINT,
        anchor: ScreenRect,
        margin_px: i32,
        is_web_engine: bool,
        cancelled: impl Fn() -> bool,
    ) -> Probe {
        if self.hit_non_text_chrome(down_pt, fg) {
            return Probe::NotText("chrome-control");
        }
        if cancelled() {
            return Probe::NotText("cancelled");
        }
        let outcome = match self.query_text_selection(fg, up_pt, anchor, margin_px) {
            QueryOutcome::Selected(s) => return Probe::Selected(s),
            QueryOutcome::None { confirmed_empty: false, stale: false, read_error: false } if is_web_engine => {
                // Chromium / Electron activates its accessibility tree lazily
                // on the first UIA call; give it a brief moment and retry once.
                thread::sleep(Duration::from_millis(WEB_ENGINE_RETRY_MS));
                if cancelled() {
                    return Probe::NotText("cancelled");
                }
                match self.query_text_selection(fg, up_pt, anchor, margin_px) {
                    QueryOutcome::Selected(s) => return Probe::Selected(s),
                    other => other,
                }
            }
            other => other,
        };
        match outcome {
            QueryOutcome::Selected(s) => Probe::Selected(s),
            QueryOutcome::None { read_error: true, .. } => Probe::NeedIbeam("uia-read-error"),
            QueryOutcome::None { stale: true, .. } => {
                if is_web_engine {
                    Probe::NeedIbeam("stale-web-selection")
                } else {
                    Probe::NotText("stale-selection")
                }
            }
            QueryOutcome::None { confirmed_empty: true, .. } => {
                if is_web_engine {
                    // Canvas-rendered text inside Chromium/Firefox (e.g. VS Code
                    // xterm.js, Google Docs) leaves the Document TextPattern empty
                    // while still showing an I-beam cursor.
                    Probe::NeedIbeam("empty-web-selection")
                } else {
                    Probe::NotText("empty-selection")
                }
            }
            QueryOutcome::None { .. } => Probe::NeedIbeam("no-text-pattern"),
        }
    }
}

/// CRLF -> LF and drop the padding spaces terminals put at the end of every
/// line (their UIA returns the full row width).
pub fn normalize(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = s.split('\n').map(|l| l.trim_end_matches([' ', '\u{a0}'])).collect();
    lines.join("\n").trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn strips_terminal_padding() {
        assert_eq!(normalize("hello   \r\nworld    \r\n"), "hello\nworld");
        assert_eq!(normalize("  indented  "), "  indented");
    }

    #[test]
    fn selection_matches_drag_accepts_overlapping_range_and_rejects_stale() {
        let drag = ScreenRect { left: 100, top: 200, right: 260, bottom: 215 };
        // Selection right under the drag.
        assert!(selection_matches_drag(drag, 32, &[rect(105, 198, 250, 218)], Some(rect(0, 0, 1200, 900))));
        // Stale selection far away in the same large Document.
        let splitter_drag = ScreenRect { left: 500, top: 600, right: 540, bottom: 605 };
        assert!(!selection_matches_drag(
            splitter_drag,
            32,
            &[rect(105, 198, 250, 218)],
            Some(rect(0, 0, 1200, 900))
        ));
    }

    #[test]
    fn selection_matches_drag_falls_back_to_element_rect_when_ranges_missing() {
        let editor = rect(50, 50, 400, 300);
        let inside = ScreenRect { left: 100, top: 100, right: 200, bottom: 110 };
        let outside = ScreenRect { left: 600, top: 100, right: 700, bottom: 110 };
        assert!(selection_matches_drag(inside, 32, &[], Some(editor)));
        assert!(!selection_matches_drag(outside, 32, &[], Some(editor)));
        // Degenerate 1x1 hidden proxy input is not falsely rejected.
        assert!(selection_matches_drag(outside, 32, &[], Some(rect(0, 0, 1, 1))));
    }
}
