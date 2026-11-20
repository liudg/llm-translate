//! Translation direction (DESIGN 5.3): CJK ideographs >= Latin letters -> zh->en.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Zh2En,
    En2Zh,
}

impl Direction {
    /// Target language name used in the prompts.
    pub fn target_name(self) -> &'static str {
        match self {
            Self::Zh2En => "英语",
            Self::En2Zh => "中文",
        }
    }

    pub fn target_is_zh(self) -> bool {
        self == Self::En2Zh
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x20000..=0x2A6DF | 0x2A700..=0x2EBEF)
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0xC0..=0x24F if c.is_alphabetic())
}

fn counts(text: &str) -> (usize, usize) {
    let (mut cjk, mut latin) = (0usize, 0usize);
    for c in text.chars() {
        if is_cjk(c) {
            cjk += 1;
        } else if is_latin(c) {
            latin += 1;
        }
    }
    (cjk, latin)
}

/// Direction for a whole text (used for the overall label).
pub fn detect(text: &str) -> Direction {
    let (cjk, latin) = counts(text);
    if cjk >= latin { Direction::Zh2En } else { Direction::En2Zh }
}

/// Direction for one paragraph; `None` when it has no letters at all
/// (numbers, symbols, dividers), which is copied through untranslated.
pub fn detect_paragraph(text: &str) -> Option<Direction> {
    let (cjk, latin) = counts(text);
    if cjk == 0 && latin == 0 {
        None
    } else if cjk >= latin {
        Some(Direction::Zh2En)
    } else {
        Some(Direction::En2Zh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions() {
        assert_eq!(detect("你好，世界"), Direction::Zh2En);
        assert_eq!(detect("Hello, world"), Direction::En2Zh);
        assert_eq!(detect("这个API很好用"), Direction::Zh2En);
        assert_eq!(detect("Open the 设置 menu please"), Direction::En2Zh);
        assert_eq!(detect("Café résumé"), Direction::En2Zh);
        assert_eq!(detect("12345"), Direction::Zh2En);
        assert_eq!(detect_paragraph("12345 / ----"), None);
        assert_eq!(detect_paragraph("OK"), Some(Direction::En2Zh));
        assert_eq!(detect_paragraph("好"), Some(Direction::Zh2En));
    }
}
