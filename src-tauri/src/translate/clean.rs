//! Streaming cleanup (DESIGN 5.3): drop a leading "译文：" style label and a pair
//! of wrapping quotes the model sometimes adds, without delaying the stream much.
//!
//! A leading quote is only a wrapper if its closing quote is the last thing in
//! the output. So the opening quote and what follows are held back until the
//! first closing quote: more text after it (`"AI" is …`) means the quotes
//! belong to the text, and everything is released with them; reaching the end
//! right after it means a wrapper, and both quotes are dropped.

const PREFIXES: &[&str] = &[
    "译文：", "译文:", "翻译：", "翻译:", "翻译结果：", "翻译结果:", "Translation:", "Translated text:",
];
// No ASCII apostrophe: it often opens real text ('90s, 'Tis).
const QUOTES: &[(char, char)] = &[('"', '"'), ('“', '”'), ('「', '」'), ('『', '』')];

/// Text after an opening quote, held until it is clear whether the quote wraps
/// the whole output.
struct Held {
    open: char,
    close: char,
    text: String,
}

pub struct Cleaner {
    source_quoted: bool,
    /// Start of the output, buffered while it may still be a label.
    head: Option<String>,
    held: Option<Held>,
    /// Trailing whitespace held back; dropped at the end.
    tail: String,
}

impl Cleaner {
    pub fn new(source: &str) -> Self {
        let first = source.trim_start().chars().next();
        Self {
            source_quoted: first.is_some_and(|c| QUOTES.iter().any(|(o, _)| *o == c)),
            head: Some(String::new()),
            held: None,
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
            let body = strip_label(h);
            // "Translation: " so far: the quote check needs the next token.
            if body.is_empty() {
                return String::new();
            }
            let body = body.to_string();
            self.head = None;
            return self.start_body(&body);
        }
        if self.held.is_some() {
            return self.hold(delta);
        }
        self.pass(delta)
    }

    /// End of stream: flush, dropping wrapping quotes and trailing whitespace.
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if let Some(head) = self.head.take() {
            let body = strip_label(head.trim_start()).to_string();
            out.push_str(&self.start_body(&body));
        }
        if let Some(held) = self.held.take() {
            let body = held.text.trim_end();
            // A wrapper: the only closing quote is the very last character.
            let wrapped = body.ends_with(held.close) && body.find(held.close) == Some(body.len() - held.close.len_utf8());
            if wrapped {
                out.push_str(body[..body.len() - held.close.len_utf8()].trim_end());
            } else {
                out.push(held.open);
                out.push_str(body);
            }
            return out;
        }
        self.tail.clear();
        out
    }

    /// The label is gone; hold the text if it opens with a quote.
    fn start_body(&mut self, body: &str) -> String {
        if !self.source_quoted
            && let Some(c) = body.chars().next()
            && let Some(&(open, close)) = QUOTES.iter().find(|(o, _)| *o == c)
        {
            self.held = Some(Held { open, close, text: String::new() });
            return self.hold(&body[c.len_utf8()..]);
        }
        self.pass(body)
    }

    fn hold(&mut self, text: &str) -> String {
        let held = self.held.as_mut().unwrap();
        held.text.push_str(text);
        let Some(i) = held.text.find(held.close) else { return String::new() };
        // Only whitespace after the first closing quote: may be the end.
        if held.text[i + held.close.len_utf8()..].trim().is_empty() {
            return String::new();
        }
        // Text goes on after the closing quote: the quotes are part of it.
        let held = self.held.take().unwrap();
        let mut out = String::from(held.open);
        out.push_str(&self.pass(&held.text));
        out
    }

    /// Release text, holding back trailing whitespace (dropped at the end).
    fn pass(&mut self, text: &str) -> String {
        self.tail.push_str(text);
        let cut = self.tail.trim_end().len();
        let out = self.tail[..cut].to_string();
        self.tail.drain(..cut);
        out
    }
}

fn strip_label(s: &str) -> &str {
    let s = s.trim_start();
    PREFIXES.iter().find_map(|p| s.strip_prefix(p)).map_or(s, str::trim_start)
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
        assert_eq!(clean("你好", "\"Hello\"  \n"), "Hello");
        // Source itself quoted: keep quotes.
        assert_eq!(clean("“引用”", "“Quote”"), "“Quote”");
        // Inner quotes are untouched.
        assert_eq!(clean("x", "He said “hi” to me"), "He said “hi” to me");
    }

    #[test]
    fn keeps_quotes_that_belong_to_the_text() {
        assert_eq!(clean("《三体》是一本好书", "\"The Three-Body Problem\" is a good book."), "\"The Three-Body Problem\" is a good book.");
        assert_eq!(clean("你好。", "\"Hello\"."), "\"Hello\".");
        assert_eq!(clean("x", "“a” and “b”"), "“a” and “b”");
        // Unclosed: nothing to unwrap.
        assert_eq!(clean("x", "“dangling"), "“dangling");
    }

    #[test]
    fn streaming_matches_whole() {
        assert_eq!(streamed("hi", &["译", "文", "：", "你", "好", "，", "世界"]), "你好，世界");
        assert_eq!(streamed("hi", &["“", "你好", "”"]), "你好");
        assert_eq!(streamed("hi", &["“", "a”b", " c”"]), "“a”b c”");
        assert_eq!(streamed("hi", &["Hello", " world"]), "Hello world");
        assert_eq!(streamed("hi", &["H"]), "H");
        // Label, space and quote arriving as separate tokens.
        assert_eq!(streamed("你好", &["Translation:", " ", "\"", "Hello", "\""]), "Hello");
    }

    #[test]
    fn quoted_start_streams_once_resolved() {
        let mut c = Cleaner::new("AI 很强");
        assert_eq!(c.push("\"AI"), "");
        assert_eq!(c.push("\""), "");
        // Text after the closing quote: released at once, quotes kept.
        assert_eq!(c.push(" is"), "\"AI\" is");
        assert_eq!(c.push(" strong"), " strong");
        assert_eq!(c.finish(), "");
    }
}
