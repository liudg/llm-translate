//! Read the selection through UI Automation (DESIGN 5.5 step 1). Must be used
//! on a thread initialised for the multithreaded apartment.

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Accessibility::*;
use windows::core::{Interface, Result};

/// Every UIA call gives up after this long, then Ctrl+Insert takes over.
const TIMEOUT_MS: u32 = 200;
/// How far up from the focused element to look for a TextPattern.
const MAX_DEPTH: usize = 6;

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

    /// Selected text of the focused control, normalised; empty when there is
    /// no TextPattern or no selection.
    pub fn selection(&self) -> Result<String> {
        unsafe {
            let mut el = self.uia.GetFocusedElement()?;
            for _ in 0..MAX_DEPTH {
                if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                    let ranges = tp.GetSelection()?;
                    let mut out = String::new();
                    for i in 0..ranges.Length()? {
                        out.push_str(&ranges.GetElement(i)?.GetText(-1)?.to_string());
                    }
                    return Ok(normalize(&out));
                }
                match self.walker.GetParentElement(&el) {
                    Ok(p) => el = p,
                    Err(_) => break,
                }
            }
            Ok(String::new())
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
    use super::normalize;

    #[test]
    fn strips_terminal_padding() {
        assert_eq!(normalize("hello   \r\nworld    \r\n"), "hello\nworld");
        assert_eq!(normalize("  indented  "), "  indented");
    }
}
