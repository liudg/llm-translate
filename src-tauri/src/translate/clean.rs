//! Streaming cleanup (DESIGN 5.3): drop a leading "译文：" style label and a pair
//! of wrapping quotes the model sometimes adds, without delaying the stream much.

const PREFIXES: &[&str] = &[
    "译文：", "译文:", "翻译：", "翻译:", "翻译结果：", "翻译结果:", "Translation:", "Translated text:",
];
// No ASCII apostrophe: it often opens real text ('90s, 'Tis).
const QUOTES: &[(char, char)] = &[('"', '"'), ('“', '”'), ('「', '」'), ('『', '』')];

pub struct Cleaner {
    source_quoted: bool,
    head: Option<String>,
    close: Option<char>,
    /// Trailing text held back because it may be the closing quote.
    tail: String,
}

impl Cleaner {
    pub fn new(source: &str) -> Self {
        let first = source.trim_start().chars().next();
        Self {
            source_quoted: first.is_some_and(|c| QUOTES.iter().any(|(o, _)| *o == c)),
            head: Some(String::new()),
            close: None,
            tail: String::new(),
        }
    }

    /// Feed a delta; returns text that is safe to show now.
    pub fn push(&mut self, delta: &str) -> String {
        if let Some(head) = &mut self.head {
            head.push_str(delta);
            let h = head.trim_start();
            // Keep buffering while the text may still turn into a label, or is
            // too short to tell whether it opens with a quote.
            let maybe_label = PREFIXES.iter().any(|p| p.starts_with(h));
            if maybe_label || h.chars().count() < 2 {
                return String::new();
            }
            let head = self.head.take().unwrap();
            let body = self.strip_head(&head);
            return self.pass(&body);
        }
        self.pass(delta)
    }

    /// End of stream: flush, dropping a matching closing quote.
    pub fn finish(&mut self) -> String {
        if let Some(head) = self.head.take() {
            let body = self.strip_head(&head);
            let mut out = self.pass(&body);
            out.push_str(&self.drain_tail(true));
            return out;
        }
        self.drain_tail(true)
    }

    fn strip_head(&mut self, head: &str) -> String {
        let mut s = head.trim_start();
        for p in PREFIXES {
            if let Some(rest) = s.strip_prefix(p) {
                s = rest.trim_start();
                break;
            }
        }
        if !self.source_quoted {
            if let Some(c) = s.chars().next() {
                if let Some((_, close)) = QUOTES.iter().find(|(o, _)| *o == c) {
                    self.close = Some(*close);
                    s = &s[c.len_utf8()..];
                }
            }
        }
        s.to_string()
    }

    /// Release text, holding back trailing whitespace and (if a quote was
    /// stripped) a trailing closing quote, which are dropped at the end.
    fn pass(&mut self, text: &str) -> String {
        self.tail.push_str(text);
        let mut cut = self.tail.trim_end().len();
        if let Some(close) = self.close {
            if self.tail[..cut].ends_with(close) {
                cut -= close.len_utf8();
            }
        }
        let out = self.tail[..cut].to_string();
        self.tail.drain(..cut);
        out
    }

    /// Whatever is held back at the end is trailing whitespace / closing quote.
    fn drain_tail(&mut self, _end: bool) -> String {
        self.tail.clear();
        String::new()
    }
}

/// Convenience for whole strings (cache, tests).
pub fn clean(source: &str, output: &str) -> String {
    let mut c = Cleaner::new(source);
    let mut s = c.push(output);
    s.push_str(&c.finish());
    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streamed(source: &str, parts: &[&str]) -> String {
        let mut c = Cleaner::new(source);
        let mut s: String = parts.iter().map(|p| c.push(p)).collect();
        s.push_str(&c.finish());
        s
    }

    #[test]
    fn strips_prefix_and_quotes() {
        assert_eq!(clean("你好", "Hello"), "Hello");
        assert_eq!(clean("你好", "译文：你好呀"), "你好呀");
        assert_eq!(clean("hi", "“你好”"), "你好");
        assert_eq!(clean("你好", "Translation: \"Hello\""), "Hello");
        // Source itself quoted: keep quotes.
        assert_eq!(clean("“引用”", "“Quote”"), "“Quote”");
        // Inner quotes are untouched.
        assert_eq!(clean("x", "He said “hi” to me"), "He said “hi” to me");
    }

    #[test]
    fn streaming_matches_whole() {
        assert_eq!(streamed("hi", &["译", "文", "：", "你", "好", "，", "世界"]), "你好，世界");
        assert_eq!(streamed("hi", &["“", "你好", "”"]), "你好");
        assert_eq!(streamed("hi", &["“", "a”b", " c”"]), "a”b c");
        assert_eq!(streamed("hi", &["Hello", " world"]), "Hello world");
        assert_eq!(streamed("hi", &["H"]), "H");
    }
}
